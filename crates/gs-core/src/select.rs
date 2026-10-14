//! Selectors (research 13, 8.4; decisions 0015 and 0026): a set of faces described ("masked
//! faces", "faces in groups 2 and 5", "faces facing the camera") and resolved into one bit per
//! face, per node ([`FaceSet`]). Every command that acts on a region takes one, so the rules for
//! what a region is live here once. Screen outlines resolve through the gesture's face rule
//! (`stroke::gesture`) into the same type.
//!
//! The face-from-points rule lives only here: a face's mask is the mean of its corners' masks,
//! or their largest (`any` corner) or smallest (`all` corners).

use crate::bits::Bits;
use crate::columns::AttrDef;
use crate::config::{FaceId, NodeConfig, NodeId};
use crate::math::{NO, Vec3, arity, face_center, face_cross};
use crate::store::MeshStore;
use crate::stroke::View;
use crate::view::PosRead;

/// Faces picked, per node: bit `j` of node `n` is the node's face slot `j`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceSet {
    nodes: Vec<Option<Bits>>,
}

impl FaceSet {
    /// The faces of node `n` that are in the set, or `None` if none is.
    pub fn node(&self, n: NodeId) -> Option<&Bits> {
        self.nodes.get(n as usize)?.as_ref()
    }

    /// How many faces are in the set.
    pub fn count(&self) -> usize {
        self.nodes.iter().flatten().map(Bits::count).sum()
    }

    /// Whether face `f` of a store cut by `cfg` is in the set.
    #[inline]
    pub fn contains(&self, cfg: NodeConfig, f: FaceId) -> bool {
        self.node(cfg.face_node(f))
            .is_some_and(|b| b.get(cfg.face_local(f) as usize))
    }

    /// The faces in both sets.
    #[must_use]
    pub fn and(&self, o: &FaceSet) -> FaceSet {
        let nodes = (self.nodes.iter().enumerate())
            .map(|(n, b)| {
                let (Some(b), Some(c)) = (b, o.node(n as NodeId)) else {
                    return Bits::zeros(0);
                };
                (0..b.len()).map(|j| b.get(j) && c.get(j)).collect()
            })
            .collect();
        FaceSet::from_nodes(nodes)
    }

    /// One entry per node; an entry with no bit set is stored as `None`.
    pub(crate) fn from_nodes(nodes: Vec<Bits>) -> FaceSet {
        let nodes = nodes.into_iter().map(|b| b.any().then_some(b)).collect();
        FaceSet { nodes }
    }
}

/// How a face's mask comes from its corners' masks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FaceRule {
    /// The mean of the corners.
    #[default]
    Mean,
    /// The largest corner: a face counts as masked if any corner is.
    Any,
    /// The smallest corner: a face counts as masked only if every corner is.
    All,
}

/// A set of faces, described. Hidden faces are picked like visible ones, so a selector can name
/// faces to show again; `Visible` and `Hidden` pick by visibility. Dead face slots are never
/// picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    /// Faces whose mask, by `rule`, is at least `threshold`.
    Masked { threshold: f32, rule: FaceRule },
    /// Faces whose mask, by `rule`, is below `threshold`.
    Unmasked { threshold: f32, rule: FaceRule },
    /// Faces in any of these face groups (0 is no group).
    Groups(Vec<u32>),
    /// Every face.
    All,
    /// Faces not hidden.
    Visible,
    /// Hidden faces.
    Hidden,
    /// These faces, by store id.
    Faces(Vec<FaceId>),
    /// Faces turned toward a camera: the angle between a face's unit normal and the direction to
    /// the camera (from the face's center to the eye, or against the view direction for an
    /// orthographic camera) is at most `angle` degrees, and under 90 in every case. No occlusion
    /// test: inner surfaces facing the camera count (decision 0026).
    Facing { view: View, angle: f32 },
}

/// Whether face `fc` faces `view` within `min_cos` (see [`Selector::Facing`]).
fn facing(fc: [u32; 4], get: impl Fn(u32) -> Vec3, view: View, min_cos: f32) -> bool {
    let n = face_cross(fc, &get).normalized();
    let to = match view.eye {
        Some(eye) => (eye - face_center(fc, &get)).normalized(),
        None => (-view.dir).normalized(),
    };
    let c = n.dot(to);
    c > 0.0 && c >= min_cos
}

impl MeshStore {
    /// The faces `sel` picks.
    pub fn select(&self, sel: &Selector) -> FaceSet {
        let mask = self
            .attr_id(&AttrDef::mask().name)
            .map(|m| self.attr_reader(m));
        let group = self
            .attr_id(&AttrDef::face_group().name)
            .map(|g| self.attr_reader(g));
        let visible = self.visible();
        let pos = self.pos();
        let min_cos = match sel {
            Selector::Facing { angle, .. } => libm::cos(f64::from(*angle).to_radians()) as f32,
            _ => 0.0,
        };
        let listed = match sel {
            Selector::Faces(ids) | Selector::Groups(ids) => {
                let mut ids = ids.clone();
                ids.sort_unstable();
                ids
            }
            _ => Vec::new(),
        };
        let nodes = gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let picks = |j: usize, fc: [u32; 4]| -> bool {
                let f = node.first_face + j as u32;
                let face_mask = |rule: FaceRule| {
                    let m = |v: u32| mask.map_or(0.0, |r| r.scalar(v));
                    let c = &fc[..arity(fc)];
                    match rule {
                        FaceRule::Mean => c.iter().map(|&v| m(v)).sum::<f32>() / c.len() as f32,
                        FaceRule::Any => c.iter().map(|&v| m(v)).fold(f32::MIN, f32::max),
                        FaceRule::All => c.iter().map(|&v| m(v)).fold(f32::MAX, f32::min),
                    }
                };
                match sel {
                    Selector::Masked { threshold, rule } => face_mask(*rule) >= *threshold,
                    Selector::Unmasked { threshold, rule } => face_mask(*rule) < *threshold,
                    Selector::Groups(_) => listed
                        .binary_search(&group.map_or(0, |r| r.label(f)))
                        .is_ok(),
                    Selector::All => true,
                    Selector::Visible => visible.face(f),
                    Selector::Hidden => !visible.face(f),
                    Selector::Faces(_) => listed.binary_search(&f).is_ok(),
                    Selector::Facing { view, .. } => facing(fc, |v| pos.get(v), *view, min_cos),
                }
            };
            (node.faces.iter().enumerate())
                .map(|(j, &fc)| fc[0] != NO && picks(j, fc))
                .collect()
        });
        FaceSet::from_nodes(nodes)
    }
}
