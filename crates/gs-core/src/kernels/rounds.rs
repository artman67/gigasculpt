//! A Smooth dab's sub-steps as the rounds of one step (decision 0036, section 2, amended).

use std::ops::Range;

use super::{DabKernel, Frame, smooth};
use crate::brush::{SmoothMode, SmoothSpec};
use crate::config::VertId;
use crate::kernel::{Free, Kernel, Reach, SculptSurface, Steps};
use crate::math::Vec3;
use crate::view::NodeView;

/// The sub-steps of a Smooth dab whose deltas read no normals, as the rounds of one step
/// (decision 0036, section 2, amended): each part of a node keeps its neighbour lists and the
/// slots the first round touched, the only ones a later round can move.
pub(super) struct Rounds<'a> {
    /// The dab, with each round's share of the amount.
    dab: &'a DabKernel<'a>,
    k: SmoothSpec,
}

/// What a part of a node keeps from round to round of a Smooth dab.
pub(super) struct Kept {
    /// Neighbour lists, and for Stronger the means around the part's slots and the other
    /// parts' points it reads, each found once a round.
    nb: smooth::Scratch,
    /// Stronger's means in a later round, found in one pass.
    flat: smooth::Flat,
    slots: Range<u32>,
    /// The slots the first round touched, once it has run.
    touched: Option<Vec<u32>>,
}

impl Steps for Rounds<'_> {
    type Part = Kept;

    fn spheres(&self) -> Vec<(Vec3, f32)> {
        self.dab.spheres()
    }

    fn free(&self) -> Vec<Free> {
        self.dab.free()
    }

    fn reach(&self) -> Reach {
        self.dab.reach()
    }

    fn part(&self, node: &NodeView<'_>, slots: Range<u32>) -> Kept {
        Kept {
            nb: smooth::Scratch::keeping(node, slots.clone()),
            flat: smooth::Flat::default(),
            slots,
            touched: None,
        }
    }

    /// [`DabKernel::each`] for the Smooth kernel over the part's slots, after the first round
    /// over those it touched.
    fn deltas(
        &self,
        s: &SculptSurface<'_>,
        node: &NodeView<'_>,
        kept: &mut Kept,
        out: &mut Vec<(u32, Vec3)>,
    ) {
        let (s, mode, gain, invert) = (*s, self.k.mode, self.k.gain, self.dab.ctx.invert);
        let sign = move |t: Vec3| if invert { -t } else { t };
        let full = |f: &Frame, v, p, n, w, nb: &mut smooth::Scratch| {
            sign(smooth::smooth((mode, gain), s, f, v, p, n, w, nb))
        };
        let Kept {
            nb,
            flat,
            slots,
            touched,
        } = kept;
        nb.round();
        let later = |l, t: Option<Vec3>| out.push((l, t.unwrap_or(Vec3::ZERO)));
        match touched {
            Some(list) if mode == SmoothMode::Uniform => {
                // Uniform reads the kept lists as they are, so a later round leaves them be.
                let kernel = |f: &Frame, v, p, _, w, nb: &mut smooth::Scratch| {
                    sign(smooth::uniform_kept(gain, s, f, nb, node, (v, p, w)))
                };
                self.visit(s, node, nb, list.iter().copied(), &kernel, later);
            }
            Some(list) if mode == SmoothMode::Stronger => {
                // Stronger finds the round's means in one pass, then each slot reads its own.
                nb.find_means(s, node, flat);
                let flat = &*flat;
                let kernel = |f: &Frame, v, p, _, w, nb: &mut smooth::Scratch| {
                    sign(smooth::stronger_kept(gain, f, (nb, flat), (v, p, w)))
                };
                self.visit(s, node, nb, list.iter().copied(), &kernel, later);
            }
            Some(list) => self.visit(s, node, nb, list.iter().copied(), &full, later),
            None => {
                let mut first = Vec::new();
                self.visit(s, node, nb, slots.clone(), &full, |l, t| {
                    if let Some(t) = t {
                        first.push(l);
                        out.push((l, t));
                    }
                });
                *touched = Some(first);
            }
        }
    }
}

impl<'a> Rounds<'a> {
    /// The rounds of `dab`, which holds each round's share of the amount, for Smooth `k`.
    pub(super) fn new(dab: &'a DabKernel<'a>, k: SmoothSpec) -> Self {
        Self { dab, k }
    }

    /// Phase 1 of the dab over the slots `ls` of `node`: `each(l, delta)` for every slot, the
    /// delta `None` where the dab does not touch it.
    fn visit(
        &self,
        s: SculptSurface<'_>,
        node: &NodeView<'_>,
        nb: &mut smooth::Scratch,
        ls: impl Iterator<Item = u32>,
        kernel: &impl Fn(&Frame, VertId, Vec3, Vec3, f32, &mut smooth::Scratch) -> Vec3,
        each: impl FnMut(u32, Option<Vec3>),
    ) {
        let ls = ls.map(|l| (l as usize, l));
        self.dab.slots(s, node, kernel).run(ls, nb, each);
    }
}
