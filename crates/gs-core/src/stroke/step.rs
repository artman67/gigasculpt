//! The step every stroke class shares: a dab or drag update expanded into its frames in one
//! place (the symmetry passes, then Wrap Mode's copies), Live Detail under them, the kernel and
//! the post effects, and what the report counts.

use super::{Stroke, frame, mirrored};
use crate::alpha::{AlphaRead, Sign};
use crate::brush::{KernelSpec, PlaneSource, SmoothSpec};
use crate::config::NodeId;
use crate::kernel::Scratch;
use crate::kernels::{Ctx, DabKernel, Frame, Paint, PaintBuffer, blend, paint};
use crate::live;
use crate::math::Vec3;
use crate::par::merge;
use crate::rng::CommandRng;
use crate::store::MeshStore;

/// How fast a dab paints color: the Mask Pen's gain, so a Paint dab mixes as a Mask Pen dab
/// masks (decision 0039, section 6).
const PAINT_GAIN: f32 = 8.0;

impl Stroke {
    /// The area normal under the stroke's first dab, in its first symmetry pass, once that dab
    /// ran: the start normal Pull Out's direction reads (decision 0049, section 7).
    pub fn start_normal(&self) -> Option<Vec3> {
        self.starts.first().map(|s| s.normal)
    }

    pub(super) fn ctx(&self) -> Ctx<'_> {
        let sign = if self.preset.kernel.signed_alpha() {
            Sign::Signed
        } else {
            Sign::Intensity
        };
        let alpha = (self.params.alpha.as_deref())
            .zip(self.preset.tip.alpha.as_ref())
            .map(|(pyramid, spec)| AlphaRead {
                pyramid,
                mid: spec.mid,
                tiles: spec.tiles,
                sign,
            });
        Ctx {
            preset: &self.preset,
            tip: &self.tip,
            alpha,
            sym: self.params.symmetry,
            lock: self.params.lock,
            invert: self.params.invert,
            starts: &self.starts,
            cols: self.cols,
        }
    }

    /// Expand a dab into its frames ([`Stroke::frames`]) and run its kernel and post effects.
    pub(super) fn apply(&mut self, store: &mut MeshStore, mut f: Frame) {
        if self.starts.is_empty() {
            self.start(store, &f);
        }
        if let KernelSpec::Plane(p) = &self.preset.kernel {
            let first = &self.starts[0];
            match p.source {
                PlaneSource::Dab => {}
                PlaneSource::Stroke => (f.normal, f.origin) = (first.normal, first.origin),
                PlaneSource::StrokeNormal => f.normal = first.normal,
            }
        }
        if self.params.alpha.is_some() {
            let finest = &mut self.report.alpha_finest;
            *finest = Some(finest.map_or(f.level, |k| k.min(f.level)));
        }
        let frames = self.frames(&f);
        let refined = match (&self.live, &mut self.report.live) {
            (Some(ld), Some(out)) => {
                let spheres: Vec<(Vec3, f32)> =
                    frames.iter().map(|f| (f.center, f.radius)).collect();
                let mirrored = mirrored(self.params.symmetry);
                live::refine(store, &spheres, ld, mirrored, out)
            }
            _ => Vec::new(),
        };
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut buffer = self.paint.take();
        if let Some(b) = &mut buffer {
            b.forget(&refined);
        }
        let nodes = self.kernels(store, &frames, &mut scratch, buffer.as_mut());
        self.scratch = scratch;
        self.paint = buffer;
        self.note(&merge(&nodes, &refined));
    }

    /// The one step that repeats a dab: its copy in each symmetry pass, then Wrap Mode's copies
    /// of each (decision 0054, section 3). Live Detail, the kernels and paint all read these
    /// frames, so a new way to repeat a dab adds its copies here.
    fn frames(&self, f: &Frame) -> Vec<Frame> {
        let mut frames: Vec<Frame> = (self.passes.iter().zip(&self.directions).enumerate())
            .map(|(i, (&at, &dir))| frame::copy(f, at, dir, i))
            .collect();
        // Mirror passes in an order fixed by place, greatest center first, not by the side
        // the pen is on, so their deltas sum alike from either side (decision 0021, L4).
        if self.params.symmetry.radial.is_none() {
            let key = |f: &Frame| f.center.to_array();
            frames.sort_by(|a, b| {
                let (a, b) = (key(a), key(b));
                (0..3)
                    .map(|k| b[k].total_cmp(&a[k]))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        match &self.wrap {
            Some(w) => {
                let ctx = self.ctx();
                w.copies(&frames, |f| ctx.reach(f))
            }
            None => frames,
        }
    }

    /// The dab over its passes `frames`, in decision 0039's order: the kernel, the post effects,
    /// then paint, a color brush's into the stroke's `buffer`. Returns the nodes written.
    fn kernels(
        &self,
        store: &mut MeshStore,
        frames: &[Frame],
        scratch: &mut Scratch,
        buffer: Option<&mut PaintBuffer>,
    ) -> Vec<NodeId> {
        let ctx = self.ctx();
        if let (KernelSpec::Attribute(k), Some(def)) = (&self.preset.kernel, &self.column) {
            return paint(store, &ctx, frames, k, def);
        }
        let mut nodes = Vec::new();
        if self.preset.sculpts() {
            let main = DabKernel {
                ctx: &ctx,
                frames,
                spec: &self.preset.kernel,
                gravity: true,
            };
            nodes = main.run(store, scratch);
            let post = &self.preset.post;
            if self.auto_smooth {
                let spec =
                    KernelSpec::Smooth(SmoothSpec::new(post.auto_smooth_mode, post.auto_smooth));
                let post = DabKernel {
                    ctx: &self.calm(),
                    frames,
                    spec: &spec,
                    gravity: false,
                };
                nodes = merge(&nodes, &store.run_kernel(&post, scratch).brushed);
            }
        }
        let Some(spec) = self.preset.paint else {
            return nodes;
        };
        // Paint goes as far as the dab's amount through Color Sensitivity, when it has its own.
        let felt: Vec<Frame>;
        let frames = match self.paint_amount {
            Some(amount) => {
                felt = (frames.iter()).map(|&f| Frame { amount, ..f }).collect();
                &felt
            }
            None => frames,
        };
        let calm = self.calm();
        let painted = match (&self.preset.kernel, buffer) {
            (KernelSpec::Smooth(sm), _) => {
                let rng = CommandRng::from_seed(self.params.seed);
                let dab = self.report.dabs;
                blend(store, &calm, frames, sm.gain, spec.color_strength, rng, dab)
            }
            (_, Some(b)) => {
                // Alt as the key gives it: a Subtract preset's flip is for sculpting.
                let alt = ctx.invert != self.preset.subtracts();
                let colors = self.params.colors;
                b.dab(store, &calm, frames, &Paint::of(PAINT_GAIN, colors, alt))
            }
            (_, None) => Vec::new(),
        };
        merge(&nodes, &painted)
    }

    /// The dab's context for what reads only the size of its weight, never its sign: the post
    /// effects and paint. Not inverted, and the alpha's factor by its size.
    fn calm(&self) -> Ctx<'_> {
        let ctx = self.ctx();
        let size = |a: AlphaRead<'_>| match a.sign {
            Sign::Signed => Sign::Size,
            other => other,
        };
        Ctx {
            invert: false,
            alpha: ctx.alpha.map(|a| AlphaRead { sign: size(a), ..a }),
            ..ctx
        }
    }

    pub(super) fn note(&mut self, nodes: &[NodeId]) {
        let r = &mut self.report;
        r.dabs += 1;
        r.max_dab_nodes = r.max_dab_nodes.max(nodes.len());
        r.nodes = merge(&r.nodes, nodes);
    }
}
