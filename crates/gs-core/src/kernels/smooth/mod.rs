//! Smooth: pull toward a neighbour average (Smooth, Smooth Strong, the auto-smooth post effect).
//! The modes and settings decision 0049 added run in [`modes`].

mod kept;
pub(super) mod modes;
mod whole;

pub(super) use kept::{Flat, stronger_kept, uniform_kept};

use super::Frame;
use crate::brush::SmoothMode;
use crate::config::VertId;
use crate::kernel::SculptSurface;
use crate::math::Vec3;
use crate::view::{NodeView, PosRead, edge_neighbours};
use kept::Kept;

use std::ops::{Add, Mul, Range};

/// The largest step one run of the Smooth kernel takes: the exponent `gain * w * amount` in
/// `1 - exp(-x)`, with the influence `w` at most 1. A stronger dab runs as sub-steps of at most
/// this (decision 0036, section 2), so a Smooth stroke does the same whatever its spacing;
/// measured with `tests/core/brush_claims.rs`, `strength_does_not_depend_on_spacing`.
pub(super) const MOST_STEP: f32 = 0.5;

/// The most sub-steps one dab runs, which bounds its cost. A dab past it takes larger steps.
pub(super) const MOST_STEPS: u32 = 32;

/// How many sub-steps a dab of the Smooth kernel with `gain` and `amount` runs:
/// `ceil(gain * amount / MOST_STEP)`, from 1 to [`MOST_STEPS`]. It reads only the preset's gain
/// and the dab's amount, the same `f32` numbers on every machine and thread count, so a stroke
/// still gives the same bits everywhere.
pub(super) fn sub_steps(gain: f32, amount: f32) -> u32 {
    let x = gain * amount / MOST_STEP;
    if x > 1.0 {
        (x.ceil() as u32).min(MOST_STEPS)
    } else {
        1
    }
}

/// The neighbours a vertex averages: its boundary neighbours if it has any, else all of them.
fn chosen(nb: &[(VertId, u8)]) -> impl Iterator<Item = VertId> + Clone + '_ {
    let boundary = nb.iter().any(|e| e.1 == 1);
    nb.iter()
        .filter(move |e| !boundary || e.1 == 1)
        .map(|e| e.0)
}

/// Mean of `f` over the vertices `ids`, in their order.
fn mean<T>(ids: impl Iterator<Item = VertId>, mut f: impl FnMut(VertId) -> T) -> Option<T>
where
    T: Copy + Default + Add<Output = T> + Mul<f32, Output = T>,
{
    let (sum, k) = ids.fold((T::default(), 0u32), |(s, k), u| (s + f(u), k + 1));
    (k != 0).then(|| sum * (1.0 / k as f32))
}

/// `d` without its part along `n`.
fn along_surface(d: Vec3, n: Vec3) -> Vec3 {
    d - n * n.dot(d)
}

/// Smooth's scratch for the slot loop of a node or part: the edge neighbours of the vertex being
/// moved, and the stronger mode's means around the part's vertices.
pub(super) struct Scratch {
    mine: Vec<(VertId, u8)>,
    /// The chosen neighbours of the vertex being moved, copied from the kept lists.
    ids: Vec<VertId>,
    around: Around,
}

impl Scratch {
    /// A scratch for the slots `slots` of `node`: Stronger keeps the means around them.
    pub(super) fn new(node: &NodeView<'_>, slots: Range<u32>) -> Self {
        let around = Around {
            base: node.base + slots.start,
            slots: slots.len(),
            means: Vec::new(),
            nb: Vec::new(),
            kept: None,
        };
        Self {
            mine: Vec::new(),
            ids: Vec::new(),
            around,
        }
    }
}

/// The mean around each of a part's own vertices (`base..base + slots`), found once: each is the neighbour of about six others in the node, and the
/// positions read stay the same all step.
struct Around {
    base: VertId,
    slots: usize,
    means: Vec<Option<Vec3>>,
    /// Scratch for a vertex's edge neighbours.
    nb: Vec<(VertId, u8)>,
    /// For a dab's rounds, the part's neighbour lists.
    kept: Option<Kept>,
}

impl Around {
    /// The mean of the chosen neighbours of `u`, or `u` itself where it has none. Kept out of
    /// line so that the other modes' loop stays small.
    #[inline(never)]
    fn of(&mut self, s: SculptSurface<'_>, u: VertId) -> Vec3 {
        let (pos, topo, visible) = (s.pos(), s.topo(), s.visible());
        let l = u.wrapping_sub(self.base) as usize;
        let Around {
            slots,
            means,
            nb,
            kept,
            ..
        } = self;
        let own = l < *slots;
        if let Some(kept) = kept {
            if !own {
                let i = kept.outer_list(s, u, nb);
                let (ids, o) = (&kept.ids, &mut kept.outer[i]);
                let (a, b) = (o.1.0 as usize, o.1.1 as usize);
                return *o.2.get_or_insert_with(|| {
                    mean(ids[a..b].iter().copied(), |x| pos.get(x)).unwrap_or(pos.get(u))
                });
            }
            if means.is_empty() {
                means.resize(*slots, None);
            }
            return *means[l].get_or_insert_with(|| {
                let (a, b) = kept.list(s, u, l, nb);
                mean(kept.ids[a..b].iter().copied(), |x| pos.get(x)).unwrap_or(pos.get(u))
            });
        }
        let mut find = || {
            edge_neighbours(&topo, u, |f| visible.face(f), nb);
            mean(chosen(nb), |x| pos.get(x)).unwrap_or(pos.get(u))
        };
        if !own {
            return find();
        }
        if means.is_empty() {
            means.resize(*slots, None);
        }
        *means[l].get_or_insert_with(find)
    }
}

/// The translation of vertex `v` (at `p`, normal `n`) with influence `w`, under a plain spec's
/// mode and gain, the two it reads. `nb` is the node's scratch. The modes' path calls it too
/// for the old modes ([`modes`]), so their shared inner code stays inlined here.
#[expect(
    clippy::too_many_arguments,
    reason = "one vertex's whole context, read once"
)]
pub(super) fn smooth(
    (mode, gain): (SmoothMode, f32),
    s: SculptSurface<'_>,
    f: &Frame,
    v: VertId,
    p: Vec3,
    n: Vec3,
    w: f32,
    nb: &mut Scratch,
) -> Vec3 {
    let Scratch { mine, ids, around } = nb;
    let raw = if let Some(kept) = &mut around.kept {
        let l = v.wrapping_sub(around.base) as usize;
        let (a, b) = kept.list(s, v, l, &mut around.nb);
        ids.clear();
        ids.extend_from_slice(&kept.ids[a..b]);
        toward(mode, s, ids.iter().copied(), p, n, around)
    } else {
        let (topo, visible) = (s.topo(), s.visible());
        edge_neighbours(&topo, v, |f| visible.face(f), mine);
        toward(mode, s, chosen(mine), p, n, around)
    };
    raw.unwrap_or(Vec3::ZERO) * (1.0 - libm::expf(-gain * w * f.amount))
}

/// The raw translation of the vertex at `p` (normal `n`) toward its chosen neighbours `nb`,
/// before the rate: `None` when it has none, and for the modes [`modes`] runs.
#[inline]
fn toward(
    mode: SmoothMode,
    s: SculptSurface<'_>,
    nb: impl Iterator<Item = VertId> + Clone,
    p: Vec3,
    n: Vec3,
    around: &mut Around,
) -> Option<Vec3> {
    let pos = s.pos();
    match mode {
        SmoothMode::Uniform => mean(nb, |u| pos.get(u)).map(|t| t - p),
        SmoothMode::Stronger => mean(nb, |u| around.of(s, u)).map(|t| t - p),
        SmoothMode::Tangent => mean(nb, |u| pos.get(u)).map(|t| along_surface(t - p, n)),
        SmoothMode::Relax => mean(nb.clone(), |u| (pos.get(u) - p).len2().sqrt())
            .and_then(|len| {
                mean(nb, |u| {
                    let e = pos.get(u) - p;
                    let l = e.len2().sqrt();
                    if l == 0.0 {
                        Vec3::ZERO
                    } else {
                        e * (1.0 - len / l)
                    }
                })
            })
            .map(|d| along_surface(d, n)),
        SmoothMode::AlongStroke | SmoothMode::AcrossStroke | SmoothMode::GroupBorders => None,
    }
}
