//! Mask makers and their one combine step (decision 0026, 7): a value at every visible point
//! from how recessed the surface is, from its face groups or from its features, blurred if
//! asked, then joined with the mask already there by the rule every maker shares. The cavity
//! measure is the engine's one, which the cavity automask reads too (`crate::cavity`, decision
//! 0056).

use super::{MaskFilter, Scope};
use crate::cavity::{self, Walk};
use crate::columns::AttrDef;
use crate::config::{NodeId, VertId};
use crate::store::MeshStore;
use crate::view::{PosRead, TopoRead, edge_neighbours};

/// What a mask maker measures.
#[derive(Clone, Debug, PartialEq)]
pub enum MaskMaker {
    /// How recessed each point is, against the points out to `rings` rings (1 to 10) around it.
    /// `contrast` (-100 to 100) scales the measure; a negative one flips the mask.
    Cavity { contrast: f32, rings: u32 },
    /// 0 at every point touching a visible face of one of these groups, 1 elsewhere.
    Groups { keep: Vec<u32> },
    /// 1 at every point on a chosen feature, 0 elsewhere: an edge only one visible face uses
    /// (`border`: open edges and the edge of the hidden region), or visible faces of more than
    /// one group around the point (`groups`).
    Features { border: bool, groups: bool },
}

/// How a made value joins the mask already there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Combine {
    /// The made value.
    #[default]
    Replace,
    /// The old mask plus the made value.
    Add,
    /// The old mask less the made value.
    Subtract,
    /// The old mask times the made value.
    Multiply,
    /// The lower of the two.
    Min,
    /// The higher of the two.
    Max,
}

impl Combine {
    /// The new mask from the old one `m` and the made value `v`, clamped to 0..1.
    pub fn apply(self, m: f32, v: f32) -> f32 {
        let new = match self {
            Combine::Replace => v,
            Combine::Add => m + v,
            Combine::Subtract => m - v,
            Combine::Multiply => m * v,
            Combine::Min => m.min(v),
            Combine::Max => m.max(v),
        };
        new.clamp(0.0, 1.0)
    }
}

/// The combine step's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NewMask {
    /// How the made value joins the old mask.
    pub mode: Combine,
    /// What the made value is scaled by first, 0 to 1.
    pub intensity: f32,
    /// How far each point moves from its old mask to the combined one, 0 to 1.
    pub blend: f32,
    /// Blur passes over the made values before they are combined.
    pub blur: u32,
}

impl Default for NewMask {
    fn default() -> NewMask {
        NewMask {
            mode: Combine::Replace,
            intensity: 1.0,
            blend: 1.0,
            blur: 0,
        }
    }
}

/// Made values, one list per node by local slot. Hidden and dead points hold 0, unread.
type Made = Vec<Vec<f32>>;

impl MeshStore {
    /// Make a mask with `maker` and write it at the visible points as `new` says: `v` is
    /// `intensity` times the (blurred) made value, and each point's mask moves `blend` of the
    /// way to `mode`'s combination of it and `v`. Hidden and dead points keep their mask.
    /// Returns the nodes written.
    pub fn mask_make(&mut self, maker: &MaskMaker, new: NewMask) -> Vec<NodeId> {
        let mut made = self.made(maker);
        for _ in 0..new.blur {
            made = self.blurred(&made);
        }
        let cfg = self.cfg;
        self.map_mask(Scope::Visible, |v, m| {
            let value = made[cfg.node_of(v) as usize][cfg.local_of(v) as usize];
            m + new.blend * (new.mode.apply(m, new.intensity * value) - m)
        })
    }

    /// The maker's value at every visible point.
    fn made(&self, maker: &MaskMaker) -> Made {
        let (p, n, t, visible) = (self.pos(), self.nrm(), self.topo(), self.visible());
        let group = (self.attr_id(&AttrDef::face_group().name)).map(|g| self.attr_reader(g));
        let label = |f| group.map_or(0, |r| r.label(f));
        let mut keep = match maker {
            MaskMaker::Groups { keep } => keep.clone(),
            _ => Vec::new(),
        };
        keep.sort_unstable();
        gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let (mut walk, mut nb) = (Walk::new(), Vec::new());
            (0..node.pos.len())
                .map(|l| {
                    if node.pos[l].is_dead() || node.hidden(l) {
                        return 0.0;
                    }
                    let v = node.base + l as u32;
                    let faces = || t.ring(v).iter().copied().filter(|&f| visible.face(f));
                    let on = match *maker {
                        MaskMaker::Cavity { contrast, rings } => {
                            let nrm = |u| n.get(u);
                            let f = cavity::measure((&p, &t, visible), nrm, v, rings, &mut walk);
                            return value(f, contrast);
                        }
                        MaskMaker::Groups { .. } => {
                            !faces().any(|f| keep.binary_search(&label(f)).is_ok())
                        }
                        MaskMaker::Features { border, groups } => {
                            let mut ids = faces().map(label);
                            let first = ids.next();
                            let mixed = groups && first.is_some_and(|a| ids.any(|b| b != a));
                            let open = border && {
                                edge_neighbours(&t, v, |f| visible.face(f), &mut nb);
                                nb.iter().any(|e| e.1 == 1)
                            };
                            mixed || open
                        }
                    };
                    f32::from(u8::from(on))
                })
                .collect()
        })
    }

    /// One blur pass over made values: each visible point moves half way to the mean of its
    /// neighbours across visible faces (the mask blur's rule), read from the values before.
    fn blurred(&self, made: &Made) -> Made {
        let (t, visible, cfg) = (self.topo(), self.visible(), self.cfg);
        gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let mut nb = Vec::new();
            (0..node.pos.len())
                .map(|l| {
                    let m = made[k][l];
                    if node.pos[l].is_dead() || node.hidden(l) {
                        return m;
                    }
                    edge_neighbours(&t, node.base + l as u32, |f| visible.face(f), &mut nb);
                    if nb.is_empty() {
                        return m;
                    }
                    let at = |u: VertId| made[cfg.node_of(u) as usize][cfg.local_of(u) as usize];
                    let mean = nb.iter().map(|e| at(e.0)).sum::<f32>() / nb.len() as f32;
                    let blur = MaskFilter::Blur;
                    m + (blur.target(m, mean) - m) * blur.step()
                })
                .collect()
        })
    }
}

/// The cavity mask from the measure `f`: its depth at `|contrast|`, taken from 1 for a negative
/// contrast (Blender's Inverted; the automask measures the other way instead, decision 0056).
fn value(f: f32, contrast: f32) -> f32 {
    let value = cavity::depth(f, contrast.abs());
    if contrast < 0.0 { 1.0 - value } else { value }
}
