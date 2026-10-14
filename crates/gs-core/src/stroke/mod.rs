//! The stroke pipeline (ARCHITECTURE 4.3), shared by every brush: samples, Stroke Smoothing,
//! spacing, dab frame, symmetry expansion, node query, influence, kernel, post effects, plane
//! clamp and axis lock, apply, bounds and normals. Each stroke is one history step.
//!
//! Samples arrive in world space: the engine turns pen samples into surface hits (or, for a
//! drag, points on the plane through the grab) and brush sizes into world radii. The stroke
//! class decides what a sample does:
//!
//! - dab: smoothed, then spaced into dabs that each stand for a length of stroke;
//! - drag: the mesh goes back to its stroke-start state and moves by the pointer's total offset,
//!   so dragging out and back returns the surface exactly;
//! - anchored: back to the stroke start, then one dab as one press would lay it: at the press,
//!   sized by the drag and turned to it (Drag Rect), or at the pointer (Drag Dot);
//! - curve and gesture: collected, and applied when the stroke ends; Clay Line and Clip Curve
//!   along the path through the points Alt taps in ([`curve_path`], decision 0055).
//!
//! Every class can smooth its samples (Stroke Smoothing). Dab and drag strokes always pass them
//! through the smoother; anchored, curve and gesture strokes only when smoothing is on, so with
//! it off they keep every sample as before (decision 0036).
//!
//! The class files: `dab` (dab and curve strokes), `drag` (drag and anchored strokes) and
//! `gesture`. The step every class shares is `step`, the dab frame `frame`, smoothing and
//! spacing `path`, and the pen's feel (its sensitivities and lazy pressure) `pen`. What a stroke
//! takes and gives back, and the check of its parameters, is `params`.

mod dab;
mod drag;
mod frame;
mod gesture;
mod params;
mod path;
mod pen;
mod step;
mod wrap;

use gs_protocol::{GROUP, MASK};

use crate::brush::{BrushPreset, Column, KernelSpec, Smoothing, StrokeSpec};
use crate::columns::AttrDef;
use crate::curve::Curve;
use crate::falloff::Tip;
use crate::kernel::Scratch;
use crate::kernels::{Cols, PaintBuffer, PassStart};
use crate::live::{self, LiveDetail, LiveReport};
use crate::math::Vec3;
use crate::par::merge;
use crate::rng::CommandRng;
use crate::store::MeshStore;
use crate::symmetry::{Symmetry, Xform};
pub use gesture::rectangle;
pub use params::{PaintColors, StrokeError, StrokeParams, StrokeReport, StrokeSample, View};
use params::{check, refuse};
pub use path::{MOST_DABS_PER_SAMPLE, curve_path, placed};
use path::{Points, Smoother, Spacer};
pub use pen::PenFeel;

/// The planes a refining stroke's passes mirror across, whose mirror map they edit in pairs:
/// Mirror and Parallel planes alike, as both place a copy at the mirrored place; none for a
/// radial stroke.
fn mirrored(sym: Symmetry) -> [bool; 3] {
    if sym.radial.is_some() {
        [false; 3]
    } else {
        std::array::from_fn(|a| sym.mirror[a] || sym.parallel[a])
    }
}

/// A stroke in progress. See the module docs.
pub struct Stroke {
    preset: BrushPreset,
    params: StrokeParams,
    tip: Tip,
    strength: f32,
    passes: Vec<Xform>,
    /// Each pass's direction map ([`Symmetry::directions`]).
    directions: Vec<Xform>,
    cols: Cols,
    /// The column an attribute brush writes, registered with its first value
    /// ([`MeshStore::put_column`]).
    column: Option<AttrDef>,
    /// The post smoothing runs: it has a gain and the part is within its face limit.
    auto_smooth: bool,
    /// The alpha level the latest dab frame chose.
    level: usize,
    smoother: Smoother,
    spacer: Spacer,
    /// The first smoothed sample (drag and anchored strokes).
    first: Option<StrokeSample>,
    /// The sample an anchored stroke's latest update laid its dab for.
    laid: Option<StrokeSample>,
    /// The previous dab point, and the direction its first axis followed (decision 0058).
    prev: Option<(Vec3, Vec3)>,
    starts: Vec<PassStart>,
    /// Samples of a curve or gesture stroke.
    held: Vec<StrokeSample>,
    points: Points,
    scratch: Scratch,
    report: StrokeReport,
    /// The stroke opened the history step itself (otherwise it marked its start inside the
    /// caller's step).
    owns_step: bool,
    /// The settings a refining stroke uses.
    live: Option<LiveDetail>,
    /// A refining stroke's mesh as it started, which its non-accumulating dab frames sample
    /// (decision 0021, section 5): the ids of the open step's snapshot no longer name the
    /// same points once the topology changes.
    start_view: Option<MeshStore>,
    /// A paint stroke's colors in floats, until its end (decision 0039, section 6); `None` for
    /// a stroke that paints no color, or blends it (Smooth).
    paint: Option<PaintBuffer>,
    /// The strength curve paint takes, when Color Sensitivity differs from Strength
    /// Sensitivity ([`PenFeel`]).
    color: Option<Curve>,
    /// The latest dab's amount through `color`, for its paint.
    paint_amount: Option<f32>,
    /// The latest sample's pressure after lazy pressure, while it is on.
    eased: Option<f32>,
    /// The tiles Wrap Mode copies each frame over, read from the part as the stroke began.
    wrap: Option<wrap::Wrap>,
}

/// A whole stroke in one call.
pub fn stroke(
    store: &mut MeshStore,
    preset: &BrushPreset,
    params: StrokeParams,
    samples: &[StrokeSample],
) -> Result<StrokeReport, StrokeError> {
    let mut s = Stroke::begin(store, preset, params)?;
    if let Err(e) = s.add(store, samples) {
        s.cancel(store);
        return Err(e);
    }
    Ok(s.finish(store))
}

/// The column an attribute brush writes, if `store` can take it. The column is registered with
/// the first value written ([`MeshStore::put_column`]), so a stroke that changes nothing adds
/// none (review 0003 M3).
fn column_of(store: &MeshStore, kernel: &KernelSpec) -> Result<Option<AttrDef>, StrokeError> {
    let KernelSpec::Attribute(a) = kernel else {
        return Ok(None);
    };
    let def = a.column.def();
    match store.attr_fits(&def) {
        Ok(_) => Ok(Some(def)),
        Err(e) => refuse(e.to_string()),
    }
}

impl Stroke {
    /// Start a stroke: check the parameters and that the column an attribute brush writes can
    /// be written, and open the history step. Inside a step the caller already opened, the stroke joins
    /// that step instead and marks its own start in it: drag and anchored updates go back to
    /// that mark, and the start readers read it, so earlier edits in the step stay.
    pub fn begin(
        store: &mut MeshStore,
        preset: &BrushPreset,
        mut params: StrokeParams,
    ) -> Result<Stroke, StrokeError> {
        check(preset, &params)?;
        let dir = params.view.dir;
        let sym = params.symmetry;
        let live = params
            .live_detail
            .filter(|_| live::refuses(preset).is_none());
        if let Some(ld) = live
            && ld.budget < 2
            && (store.mirror_maps().iter()).any(|m| mirrored(sym)[m.0])
        {
            return refuse(format!(
                "a mirrored refining stroke edits in pairs, 2 operations each: the live detail \
                 budget must be at least 2, not {}",
                ld.budget
            ));
        }
        let mut preset = preset.clone();
        let color = params.pen.feel(&mut preset.pressure);
        let preset = &preset;
        params.view.dir = dir.normalized();
        params.invert ^= preset.subtracts();
        let column = column_of(store, &preset.kernel)?;
        // Smooth blends color with no buffer: it reads its neighbours' colors as they are.
        let paint = match (preset.paint, &preset.kernel) {
            (Some(p), kernel) if !matches!(kernel, KernelSpec::Smooth(_)) => {
                if let Err(e) = store.attr_fits(&AttrDef::color()) {
                    return refuse(e.to_string());
                }
                Some(PaintBuffer::new(p.color_strength))
            }
            _ => None,
        };
        // Mask and hide brushes are not held back by the mask.
        let unmasked = matches!(&preset.kernel,
            KernelSpec::Attribute(a) if matches!(a.column, Column::Mask | Column::Hide));
        let cols = Cols {
            mask: store.attr_id(MASK).filter(|_| !unmasked),
            group: store.attr_id(GROUP),
        };
        let post = &preset.post;
        let limit = post.auto_smooth_max_faces as usize;
        let auto_smooth = post.auto_smooth > 0.0 && (limit == 0 || store.face_count() <= limit);
        let start_view = (live.is_some() && !preset.sampling.accumulate).then(|| store.share());
        let owns_step = !store.step_open();
        if owns_step {
            store.begin_step();
        } else {
            store.set_mark();
        }
        Ok(Stroke {
            tip: preset.tip.tip(),
            passes: sym.passes(),
            directions: sym.directions(),
            preset: preset.clone(),
            params,
            strength: preset.strength,
            cols,
            column,
            auto_smooth,
            level: 0,
            smoother: Smoother::default(),
            spacer: Spacer::new(preset.stroke.directional()),
            first: None,
            laid: None,
            prev: None,
            starts: Vec::new(),
            held: Vec::new(),
            points: Points::default(),
            scratch: Scratch::default(),
            report: StrokeReport {
                live: live.map(|_| LiveReport::default()),
                ..StrokeReport::default()
            },
            owns_step,
            live,
            start_view,
            paint,
            color,
            paint_amount: None,
            eased: None,
            wrap: wrap::Wrap::of(store, preset.tip.wrap),
        })
    }

    /// Alt for what comes next: the dabs laid by the samples fed after this, a drag's next
    /// update (which redoes the whole drag from the stroke's start), and what the stroke lays
    /// when it ends (decision 0036, section 5). A Subtract preset is inverted once more, as at
    /// [`Stroke::begin`].
    pub fn set_invert(&mut self, alt: bool) {
        self.params.invert = alt != self.preset.subtracts();
    }

    /// Feed samples. A batch with a bad sample is refused whole and changes nothing.
    pub fn add(
        &mut self,
        store: &mut MeshStore,
        samples: &[StrokeSample],
    ) -> Result<(), StrokeError> {
        let bad = |s: &StrokeSample| {
            !(s.pos.to_array().iter().all(|x| x.is_finite())
                && (0.0..=1.0).contains(&s.pressure)
                && s.radius > 0.0
                && s.radius.is_finite())
        };
        if let Some(i) = samples.iter().position(bad) {
            return refuse(format!(
                "sample {i} needs a finite position, pressure from 0 to 1 and a positive radius"
            ));
        }
        let eased: Vec<StrokeSample>;
        let samples = if self.params.pen.lazy > 0.0 {
            eased = (samples.iter())
                .map(|&s| {
                    let pressure = self.params.pen.ease(self.eased, s.pressure);
                    self.eased = Some(pressure);
                    StrokeSample { pressure, ..s }
                })
                .collect();
            &eased
        } else {
            samples
        };
        match self.preset.stroke {
            StrokeSpec::Dab {
                spacing,
                tap,
                smoothing,
                ..
            } => self.add_dabs(store, samples, spacing, tap, smoothing),
            StrokeSpec::Drag { smoothing } => self.add_drag(store, samples, smoothing),
            StrokeSpec::Anchored {
                follow,
                tap,
                smoothing,
            } => self.add_anchored(store, samples, follow, tap, smoothing),
            StrokeSpec::Curve { smoothing, .. } | StrokeSpec::Gesture { smoothing, .. } => {
                self.hold(samples, smoothing);
            }
        }
        Ok(())
    }

    /// `samples` through Stroke Smoothing, or as they are when it is off: the smoother would
    /// still drop a repeated sample, which these classes have always kept (decision 0036).
    fn smoothed(&mut self, samples: &[StrokeSample], k: Smoothing) -> Vec<StrokeSample> {
        if k.is_off() {
            return samples.to_vec();
        }
        (samples.iter())
            .filter_map(|&s| self.smoother.next(s, k))
            .collect()
    }

    /// End the stroke: the last dab, a curve's dabs or a gesture, then close the history step
    /// (or, inside the caller's step, drop the stroke's mark and leave the step open).
    pub fn finish(mut self, store: &mut MeshStore) -> StrokeReport {
        match self.preset.stroke {
            StrokeSpec::Dab { .. } => self.finish_dabs(store),
            StrokeSpec::Curve {
                spacing, straight, ..
            } => self.finish_curve(store, spacing, straight),
            StrokeSpec::Gesture { .. } => self.gesture(store),
            StrokeSpec::Drag { .. } | StrokeSpec::Anchored { .. } => {}
        }
        if let Some(paint) = &mut self.paint {
            let painted = paint.finish(store, CommandRng::from_seed(self.params.seed));
            self.report.nodes = merge(&self.report.nodes, &painted);
        }
        if self.live.is_some() {
            // No dead slot outlives a stroke, and nodes are merged and re-sorted (decision 0021,
            // section 7).
            let kept = store.maintain_nodes();
            self.report.nodes = merge(&self.report.nodes, &kept);
        }
        if self.owns_step {
            self.report.step = Some(store.end_step());
        } else {
            store.clear_mark();
        }
        self.report
    }

    /// Abandon the stroke: the mesh goes back exactly to how it was at `begin`.
    pub fn cancel(self, store: &mut MeshStore) {
        if self.owns_step {
            store.cancel_step();
        } else {
            store.revert_step();
            store.clear_mark();
        }
    }

    /// What the stroke has done so far.
    pub fn report(&self) -> &StrokeReport {
        &self.report
    }
}
