//! Dab and curve strokes: dabs spaced along the path, each standing for the length of stroke
//! it covers (specified in `crate::brush`). A dab stroke lays them as its samples come, and the
//! unfinished cell's dab when it ends; a curve stroke holds its path and lays them all when it
//! ends, along the whole path or, `straight`, along the path through its ends and points.

use super::path::radius;
use super::{Stroke, StrokeSample, curve_path};
use crate::brush::{KernelSpec, Smoothing};
use crate::math::Vec3;
use crate::store::MeshStore;

impl Stroke {
    /// A dab stroke's samples: smoothed, then spaced into dabs.
    pub(super) fn add_dabs(
        &mut self,
        store: &mut MeshStore,
        samples: &[StrokeSample],
        spacing: f32,
        tap: f32,
        smoothing: Smoothing,
    ) {
        for &s in samples {
            if let Some(p) = self.smoother.next(s, smoothing) {
                self.lay(store, p, spacing, tap);
            }
        }
    }

    /// The end of a dab stroke: the dab of the unfinished cell.
    pub(super) fn finish_dabs(&mut self, store: &mut MeshStore) {
        if let Some((at, ds)) = self.spacer.rest() {
            self.dab(store, at, ds);
        }
    }

    /// The end of a curve stroke: dabs along the path it held, or when `straight` along the
    /// path through its ends and the points tapped in ([`curve_path`]), then the unfinished
    /// cell's.
    pub(super) fn finish_curve(&mut self, store: &mut MeshStore, spacing: f32, straight: bool) {
        let pts: Vec<StrokeSample> = if straight {
            curve_path(&self.path())
        } else {
            std::mem::take(&mut self.held)
        };
        for p in pts {
            self.lay(store, p, spacing, 0.0);
        }
        self.finish_dabs(store);
    }

    /// Dabs from the last smoothed point to `p`.
    fn lay(&mut self, store: &mut MeshStore, p: StrokeSample, spacing: f32, tap: f32) {
        for (at, ds) in self.spacer.walk(p, spacing, tap, &self.preset.pressure) {
            self.dab(store, at, ds);
        }
    }

    /// One dab at `at`, standing for `ds` of stroke length, its step from the previous dab.
    pub(super) fn dab(&mut self, store: &mut MeshStore, at: StrokeSample, ds: f32) {
        let step = self.prev.map_or(Vec3::ZERO, |(q, _)| at.pos - q);
        self.dab_toward(store, at, ds, step);
    }

    /// One dab at `at`, standing for `ds` of stroke length, with `step` as its step: the
    /// direction its alpha turns to and a move kernel's delta.
    pub(super) fn dab_toward(
        &mut self,
        store: &mut MeshStore,
        at: StrokeSample,
        ds: f32,
        step: Vec3,
    ) {
        let p = &self.preset;
        let pr = &p.pressure;
        let r = radius(at, pr);
        // Where the size curve reaches zero, the dab has no size and does nothing.
        if r <= 0.0 {
            return;
        }
        let strength = self.strength * pr.strength.eval(at.pressure);
        let hook = matches!(p.kernel, KernelSpec::Hook(_));
        let center = if hook { at.pos - step } else { at.pos };
        let modifier = match p.kernel {
            KernelSpec::Offset(_) if p.modifier != 0.0 => {
                KernelSpec::modifier_term(p.modifier / 100.0 * pr.modifier.eval(at.pressure))
            }
            _ => None,
        };
        let imbed = p.depth.imbed * pr.imbed.eval(at.pressure);
        let pen = p.tip.share(at.pressure);
        self.paint_amount =
            (self.color.as_ref()).map(|c| self.strength * c.eval(at.pressure) * ds / r);
        let mut f = self.frame(store, center, r, step);
        f.amount = strength * ds / r;
        f.strength = strength;
        f.delta = step;
        f.imbed = imbed;
        f.modifier = modifier;
        f.pen = pen;
        self.prev = Some((at.pos, f.aim));
        self.apply(store, f);
    }
}
