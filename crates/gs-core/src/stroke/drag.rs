//! Drag and anchored strokes: redone from the stroke start at every update. A drag goes back
//! to the stroke-start surface and moves by the pointer's whole offset from its first sample, so
//! dragging out and back returns the surface exactly; an anchored stroke starts over and lays
//! one dab as a single press would: Drag Rect at the press, sized by the drag and turned to it,
//! Drag Dot (`follow`) at the pointer with the brush size (decision 0051).

use super::path::radius;
use super::{Stroke, StrokeSample};
use crate::brush::{Smoothing, StrokeSpec};
use crate::math::Vec3;
use crate::store::MeshStore;

impl Stroke {
    /// The mesh back to the stroke's start, and a paint stroke's buffer with it.
    fn back_to_start(&mut self, store: &mut MeshStore) {
        store.revert_step();
        if let Some(paint) = &mut self.paint {
            paint.clear();
        }
    }

    /// Back to the stroke's start as if nothing were laid yet: the mesh and a paint stroke's
    /// buffer, the previous dab, and what each pass learned at the first dab. Every anchored
    /// update starts this way, so it lays what one press would; the engine starts a Drag Dot
    /// sample this way too, to aim it at the surface as the stroke found it (decision 0051).
    pub fn rewind(&mut self, store: &mut MeshStore) {
        self.back_to_start(store);
        self.prev = None;
        self.starts.clear();
    }

    /// A drag stroke's samples: smoothed, then one update to the latest.
    pub(super) fn add_drag(
        &mut self,
        store: &mut MeshStore,
        samples: &[StrokeSample],
        smoothing: Smoothing,
    ) {
        let mut last = None;
        for &s in samples {
            if let Some(p) = self.smoother.next(s, smoothing) {
                self.first.get_or_insert(p);
                last = Some(p);
            }
        }
        if let Some(p) = last {
            self.drag(store, p);
        }
    }

    /// An anchored stroke's samples: smoothed when smoothing is on, then one update to the
    /// latest. A moving sample (Space, Drag Rect only) moves the rectangle, its press and its
    /// far corner, by the pen's step instead (decision 0062, row 782).
    pub(super) fn add_anchored(
        &mut self,
        store: &mut MeshStore,
        samples: &[StrokeSample],
        follow: bool,
        tap: f32,
        smoothing: Smoothing,
    ) {
        let mut last = None;
        for &s in samples {
            if let Some(step) = self.space_step(s.pos, self.first.is_some()) {
                for p in [&mut self.first, &mut self.laid].into_iter().flatten() {
                    p.pos += step;
                }
                last = self.laid;
            } else {
                for p in self.smoothed(&[s], smoothing) {
                    self.first.get_or_insert(p);
                    last = Some(p);
                }
            }
        }
        if let Some(last) = last {
            self.laid = Some(last);
            self.anchored(store, last, follow, tap);
        }
    }

    /// An anchored stroke's latest update laid again, after a [`Stroke::rewind`]: a Drag Dot
    /// sample that misses the surface keeps the last stamp (decision 0051, section 5). The
    /// update is redone, never fed again, so smoothing does not move it.
    pub fn relay(&mut self, store: &mut MeshStore) {
        if let (Some(at), StrokeSpec::Anchored { follow, tap, .. }) =
            (self.laid, self.preset.stroke)
        {
            self.anchored(store, at, follow, tap);
        }
    }

    /// A drag update: back to the stroke start, then move by the pointer's offset.
    fn drag(&mut self, store: &mut MeshStore, latest: StrokeSample) {
        let Some(first) = self.first else { return };
        let r = radius(first, &self.preset.pressure);
        if self.starts.is_empty() {
            let f = self.frame(store, first.pos, r, Vec3::ZERO);
            self.start(store, &f);
        }
        self.back_to_start(store);
        let n = self.starts[0].normal;
        let t = latest.pos - first.pos;
        let t = if self.params.invert { n * n.dot(t) } else { t };
        if t.len2() == 0.0 {
            return;
        }
        let mut f = self.bare(first.pos, r, n, self.starts[0].origin, t);
        f.strength = self.strength * self.preset.pressure.strength.eval(latest.pressure);
        // The update stands for the stroke length of its offset, for the post effects.
        f.amount = f.strength * t.len2().sqrt() / r;
        self.paint_amount = (self.color.as_ref())
            .map(|c| self.strength * c.eval(latest.pressure) * t.len2().sqrt() / r);
        f.delta = t;
        f.pen = self.preset.tip.share(latest.pressure);
        self.apply(store, f);
    }

    /// An anchored update: back to the stroke start, then one dab. A Drag Rect dab is centered
    /// on the press, its radius the drag's length and its step the drag, so its alpha turns to
    /// it; a Drag Dot dab is at the pointer with no step, as a tap is.
    fn anchored(&mut self, store: &mut MeshStore, latest: StrokeSample, follow: bool, tap: f32) {
        let Some(first) = self.first else { return };
        self.rewind(store);
        let (at, step) = if follow {
            (latest, Vec3::ZERO)
        } else {
            let drag = latest.pos - first.pos;
            let at = StrokeSample {
                radius: drag.len2().sqrt(),
                pressure: latest.pressure,
                ..first
            };
            (at, drag)
        };
        // A drag of zero gives a dab of no size, which `dab_toward` skips.
        self.dab_toward(store, at, tap * radius(at, &self.preset.pressure), step);
    }
}
