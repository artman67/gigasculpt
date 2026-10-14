//! Smoothing a whole part (decision 0041, section 3, amended 2026-10-12 for DF5): whole rounds
//! of the Smooth kernel's largest step, all inside one kernel call, over the points a
//! transform's weights reach (the mask, hidden faces, a gradient, the kept axes and the mirror
//! planes), or one sharpening step away from each point's mean.

use std::ops::Range;

use super::{MOST_STEP, Scratch, toward};
use crate::brush::SmoothMode;
use crate::config::VertId;
use crate::kernel::{Kernel, Reach, Scratch as Buffers, SculptSurface, StepReport, Steps};
use crate::math::Vec3;
use crate::normals::vertex_normal;
use crate::store::MeshStore;
use crate::transform::{Transform, Weighted, flip};
use crate::view::{NodeView, edge_neighbours};

/// Taubin's pass-band number, `1 / lambda + 1 / mu`: each round's growing step `mu` follows
/// from its shrinking step `lambda` by it (Taubin 1995, "A signal processing approach to fair
/// surface design", which suggests 0.01 to 0.1).
const KPB: f32 = 0.1;

impl MeshStore {
    /// Rounds `rounds` of a whole-part smoothing in `mode` (Uniform, Stronger, Tangent or
    /// Relax), each one step of the kernel's largest size, 0.5, scaled by each point's weight
    /// under `t` (its operation is not read); with `keep`, each round is Taubin's pair of a
    /// Uniform shrinking step and a growing step, which keeps the volume. Rounds from 0 read the
    /// weights from the points as they are; a run that goes on from rounds already run inside
    /// the open step (`rounds.start` above 0) reads them from the step's start, where the first
    /// rounds read them, so it gives the bits of one run of all of them. Inside an open step.
    pub fn smooth(
        &mut self,
        t: &Transform,
        (mode, keep): (SmoothMode, bool),
        rounds: Range<u32>,
    ) -> StepReport {
        let more = rounds.start > 0;
        let n = rounds.len() as u32 * if keep { 2 } else { 1 };
        let k = Whole::new(self, t, (mode, keep), None, more);
        self.run_steps(&k, n, &mut Buffers::default())
    }

    /// One step of `mode`'s pull reversed, `amount` times its full length, scaled by each
    /// point's weight under `t`: each point moves away from its mean in the points as they are
    /// (Blender's Enhance Details). Inside an open step.
    pub fn sharpen(&mut self, t: &Transform, mode: SmoothMode, amount: f32) -> StepReport {
        let k = Whole::new(self, t, (mode, false), Some(amount), false);
        self.run_steps(&k, 1, &mut Buffers::default())
    }
}

/// A whole-part smoothing as the rounds of one kernel call.
struct Whole {
    k: Weighted,
    mode: SmoothMode,
    /// Taubin's pairs: even rounds shrink, odd rounds grow.
    keep: bool,
    /// One step away from the mean, this many times its length.
    sharpen: Option<f32>,
    /// Weigh from the open step's start.
    more: bool,
}

/// A point a part moves: its slot, its weight, the Mirror planes its region lies across, the
/// axes it holds (it is on a Mirror plane), and whether it has a boundary edge.
struct Point {
    l: u32,
    w: f32,
    r: u32,
    held: u32,
    edge: bool,
}

/// What a part of a node keeps from round to round.
struct Kept {
    nb: Scratch,
    points: Vec<Point>,
    round: u32,
    /// Scratch for a point's edge neighbours.
    edges: Vec<(VertId, u8)>,
}

impl Whole {
    fn new(
        store: &MeshStore,
        t: &Transform,
        (mode, keep): (SmoothMode, bool),
        sharpen: Option<f32>,
        more: bool,
    ) -> Whole {
        Whole {
            k: Weighted::new(store, t).measured(store),
            mode: if keep { SmoothMode::Uniform } else { mode },
            keep,
            sharpen,
            more,
        }
    }

    /// The rate of a point of weight `w` in round `round`: the share of its pull it moves.
    fn rate(&self, w: f32, round: u32) -> f32 {
        if let Some(amount) = self.sharpen {
            return -amount * w;
        }
        let lambda = 1.0 - libm::expf(-MOST_STEP * w);
        if self.keep && round % 2 == 1 {
            lambda / (KPB * lambda - 1.0)
        } else {
            lambda
        }
    }

    /// The point a part moves at slot `l` of `at` (the node as the weights read it), if it
    /// weighs above zero.
    fn point(&self, at: &NodeView<'_>, mask: Option<&[f32]>, l: usize) -> Option<Point> {
        let (v, r, w) = self.k.weigh(at, mask, l)?;
        let w = self.k.ready.ramp(v, w);
        let s = &self.k.t.symmetry;
        let p = at.pos[l].to_array();
        let held = (0..3)
            .filter(|&a| s.mirror[a] && p[a].abs() <= s.clip)
            .map(|a| 1 << a)
            .sum();
        (w != 0.0).then_some(Point {
            l: l as u32,
            w,
            r,
            held,
            edge: false,
        })
    }

    /// The change of `p`, the point at vertex `v`, in this round: its pull times its rate, along
    /// the kept axes of its region's frame, without the axes it holds.
    fn change(
        &self,
        s: SculptSurface<'_>,
        kept: &mut Kept,
        e: &Point,
        (v, p): (VertId, Vec3),
    ) -> Vec3 {
        let tangent = matches!(self.mode, SmoothMode::Tangent | SmoothMode::Relax);
        // A boundary point averages its boundary neighbours only, which already lie along the
        // boundary: it is not turned into the surface.
        let n = if tangent && !e.edge {
            vertex_normal(&s.topo(), &s.pos(), v)
        } else {
            Vec3::ZERO
        };
        let Scratch { ids, around, .. } = &mut kept.nb;
        let Some(lists) = &mut around.kept else {
            return Vec3::ZERO;
        };
        let (a, b) = lists.list(s, v, v.wrapping_sub(around.base) as usize, &mut around.nb);
        ids.clear();
        ids.extend_from_slice(&lists.ids[a..b]);
        let pull = toward(self.mode, s, ids.iter().copied(), p, n, around);
        let d = pull.unwrap_or(Vec3::ZERO) * self.rate(e.w, kept.round);
        let mut d = flip(self.k.ready.keep(flip(d, e.r)), e.r).to_array();
        for (a, c) in d.iter_mut().enumerate() {
            if e.held & (1 << a) != 0 {
                *c = 0.0;
            }
        }
        Vec3::from_array(d)
    }
}

impl Steps for Whole {
    type Part = Kept;

    fn spheres(&self) -> Vec<(Vec3, f32)> {
        Kernel::spheres(&self.k)
    }

    fn reach(&self) -> Reach {
        Kernel::reach(&self.k)
    }

    fn part(&self, node: &NodeView<'_>, slots: Range<u32>) -> Kept {
        Kept {
            nb: Scratch::keeping(node, slots.clone()),
            points: Vec::new(),
            round: 0,
            edges: Vec::new(),
        }
    }

    fn deltas(
        &self,
        s: &SculptSurface<'_>,
        node: &NodeView<'_>,
        kept: &mut Kept,
        out: &mut Vec<(u32, Vec3)>,
    ) {
        let s = *s;
        if kept.round == 0 {
            self.first(s, node, kept);
        }
        kept.nb.round();
        let points = std::mem::take(&mut kept.points);
        for e in &points {
            let v = node.base + e.l;
            let d = self.change(s, kept, e, (v, node.pos[e.l as usize]));
            out.push((e.l, d));
        }
        kept.points = points;
        kept.round += 1;
    }
}

impl Whole {
    /// Before a part's first round: the points it moves, weighed from the step's start when
    /// the run goes on from rounds already run, and for Tangent and Relax which have a boundary
    /// edge.
    fn first(&self, s: SculptSurface<'_>, node: &NodeView<'_>, kept: &mut Kept) {
        let start = s.start_pos();
        let at = match start.cols.get(node.id as usize) {
            Some(pos) if self.more => NodeView {
                pos: pos.as_slice(),
                ..*node
            },
            _ => *node,
        };
        let mask = self.k.mask_of(node);
        let around = &kept.nb.around;
        let slots = around.base - node.base..around.base - node.base + around.slots as u32;
        let tangent = matches!(self.mode, SmoothMode::Tangent | SmoothMode::Relax);
        let (topo, visible) = (s.topo(), s.visible());
        for l in slots {
            let Some(mut e) = self.point(&at, mask, l as usize) else {
                continue;
            };
            if tangent {
                edge_neighbours(&topo, node.base + l, |f| visible.face(f), &mut kept.edges);
                e.edge = kept.edges.iter().any(|x| x.1 == 1);
            }
            kept.points.push(e);
        }
    }
}
