//! `mesh.smooth` (decision 0041, section 3, amended 2026-10-12 for DF5): the smoothing family
//! in one command. The amount is whole rounds of the Smooth kernel's largest step, all run in
//! one kernel call, so a live drag that adds rounds and a call at the final amount give the
//! same bits; a negative amount sharpens once from the start. The frame, mask, symmetry, axes,
//! gradient and box are `transform.apply`'s, read by the same code.

use gs_core::brush::SmoothMode;
use gs_core::transform::Transform;
use gs_core::{MeshStore, NodeId, StepReport};
use gs_protocol::{CommandError, Cost, Kind, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{Part, pinned};
use crate::sculpt::{Axes, Axis, SymmetryArg};
use crate::transform::{BoxArg, ExactFrame, FrameArg, GradientArg, OpArg, TransformApply};
use crate::{Command, Ctx, Example, errors, register_command};

/// Amount per round: an amount turns into `ceil(amount / PER_ROUND)` rounds.
const PER_ROUND: f32 = 50.0;

/// Smooth a part's points toward their neighbours, or sharpen them away.
///
/// `amount` from -100 to 500: the Deform palette's Smooth, Polish and Relax go to 100 and the
/// Gizmo's smooth deformers to 500. Above 0 it is whole rounds, `ceil(amount / 50)` of them
/// (100 is 2, 500 is 10), each moving every point at full weight 1 - e^-0.5 (about 39%) of the
/// way toward its target, so the result depends only on the amount. Below 0 (`smooth` and
/// `strong` only) each point moves away from its mean once, by `-amount / 100` of the way, so
/// -100 doubles the detail. Boundary points average their boundary neighbours only, so a
/// boundary stays a boundary. `mask`, `symmetry`, `axes`, `gradient` and `box` work as in
/// `transform.apply`: the mask holds points back, a gradient fades the smoothing across the
/// box, and points on a mirror plane stay on it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshSmooth {
    #[doc = crate::command::part_doc!("The part to smooth.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// "smooth" (the default) pulls toward the neighbours' mean; "strong" toward the mean of
    /// their means; "polish" keeps the volume with `keep` (Taubin's shrink and grow steps) or
    /// slides points along the surface without it; "relax" evens out the edges along the
    /// surface.
    #[serde(default)]
    pub mode: SmoothArg,
    /// With "polish": keep the volume. Default true.
    #[serde(default = "crate::command::yes")]
    pub keep: bool,
    /// How much, -100 to 500 (see above); 0 changes nothing.
    #[schemars(range(min = -100, max = 500))]
    pub amount: f32,
    /// The frame `axes`, `gradient` and `box` are measured in: "gizmo" (the default) or an
    /// exact `{origin, axes}`, as `transform.apply` takes it.
    #[serde(default)]
    pub frame: FrameArg,
    /// The mask holds points back: a fully masked point does not move. Default true.
    #[serde(default = "crate::command::yes")]
    pub mask: bool,
    /// Mirror and parallel planes through the part's origin, as `transform.apply` takes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<SymmetryArg>,
    /// The frame's axes whose part of the change is kept, at least one; left out, all three.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<Axes>,
    /// Fade the smoothing across the box along one of the frame's axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<GradientArg>,
    /// The box the gradient is measured in, in the frame. Left out with a gradient, the box
    /// around the points that move, written in before it runs.
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoxArg>,
}

/// A smoothing mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmoothArg {
    /// Toward the neighbours' mean.
    #[default]
    Smooth,
    /// Toward the mean of the neighbours' means.
    Strong,
    /// Smooth and keep the volume, or slide along the surface.
    Polish,
    /// Even out the edges along the surface.
    Relax,
}

/// What `mesh.smooth` did.
#[derive(Serialize, JsonSchema)]
pub struct Smoothed {
    /// The part it smoothed.
    pub part: String,
    /// The frame it used, exactly.
    pub frame: ExactFrame,
    /// The box its gradient was measured in.
    #[serde(rename = "box", skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoxArg>,
    /// How many rounds the amount ran (0 for a sharpening).
    pub rounds: u32,
}

/// Why a smoothing changed nothing.
const NO_CHANGE: &str = "the smoothing left every point where it was";

impl Command for MeshSmooth {
    const NAME: &'static str = "mesh.smooth";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    // Decision 0041, section 8, and its DF5 amendment.
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}},
        {"mask_holds": {"switch": "/mask"}}, "hidden_holds", "positions_only",
        {"identity_at_zero": {"set": {"/amount": 0}}},
        {"placement_invariant": {"pinned": {"/frame": "/frame", "/box": "/box"},
            "unless": "/symmetry", "tolerance": 6e-6}},
        {"keeps_symmetry": {"planes": "/symmetry/mirror", "origin": "/frame/origin",
            "pinned": {"/frame": "/frame", "/box": "/box"}, "tolerance": 1e-6}}]"#;
    type Output = Smoothed;

    fn examples() -> Vec<Example<Self>> {
        let sphere = || json!({ "subdivisions": 3 });
        let at = |origin| {
            FrameArg::Exact(ExactFrame {
                origin: Vec3(origin),
                axes: [0.0, 0.0, 0.0, 1.0],
            })
        };
        let smooth = Self::new(SmoothArg::Smooth, 100.0);
        let polish = Self::new(SmoothArg::Polish, 60.0);
        let relax = Self::new(SmoothArg::Relax, 40.0);
        let sharpen = Self::new(SmoothArg::Strong, -50.0);
        let faded = Self {
            frame: at([0.05, 0.03, 0.02]),
            gradient: Some(GradientArg {
                axis: Axis::Y,
                from: 1.0,
                to: 0.0,
            }),
            ..Self::new(SmoothArg::Smooth, 200.0)
        };
        let mirrored = Self {
            frame: at([0.5, 0.03, 0.02]),
            symmetry: Some(SymmetryArg {
                mirror: Axes {
                    x: true,
                    ..Axes::default()
                },
                ..SymmetryArg::default()
            }),
            gradient: Some(GradientArg {
                axis: Axis::X,
                from: 1.0,
                to: 0.0,
            }),
            ..Self::new(SmoothArg::Smooth, 80.0)
        };
        vec![
            Example::new("smooth the only part, the palette at 100", smooth)
                .after("primitive.sphere", sphere()),
            Example::new("polish it, keeping its volume", polish)
                .after("primitive.sphere", sphere()),
            Example::new("relax its edges along the surface", relax)
                .after("primitive.sphere", sphere()),
            Example::new("sharpen it: strong at -50", sharpen).after("primitive.sphere", sphere()),
            Example::new(
                "smooth the top most, fading to nothing at the bottom",
                faded,
            )
            .after("primitive.sphere", sphere()),
            Example::new("fade across x, mirrored, so both sides match", mirrored)
                .after("primitive.sphere", sphere()),
        ]
    }

    /// Pins the frame, and the box a gradient needs.
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let part = pinned(self.part.clone())?;
        self.pin_frame(cx, &part)?;
        self.settle(cx.doc.get_part(&part)?)?;
        Ok(())
    }

    fn run(mut self, cx: &mut Ctx) -> Result<Smoothed, CommandError> {
        let part = pinned(self.part.clone())?;
        let t = self.settle(cx.doc.get_part(&part)?)?;
        cx.noop_reason(NO_CHANGE);
        if self.amount != 0.0 {
            cx.doc
                .edit_store(&part, |s| written(self.apply(s, &t, 0)))?;
        }
        Ok(Smoothed {
            part,
            frame: self.weighing().exact()?,
            bounds: self.bounds,
            rounds: self.rounds()?,
        })
    }
}
register_command!(MeshSmooth, "Smooth");

impl MeshSmooth {
    /// `amount` of `mode` about the gizmo, on the active part.
    pub(crate) fn new(mode: SmoothArg, amount: f32) -> Self {
        Self {
            part: None,
            mode,
            keep: true,
            amount,
            frame: FrameArg::default(),
            mask: true,
            symmetry: None,
            axes: None,
            gradient: None,
            bounds: None,
        }
    }

    /// The rounds the amount runs, 0 for a sharpening; refuses an amount outside -100 to 500,
    /// and below 0 for "polish" and "relax".
    pub(crate) fn rounds(&self) -> Result<u32, CommandError> {
        let a = self.amount;
        if !(-100.0..=500.0).contains(&a) {
            return Err(errors::args_invalid(
                "/amount",
                "amount must be from -100 to 500",
            ));
        }
        if a < 0.0 && matches!(self.mode, SmoothArg::Polish | SmoothArg::Relax) {
            let message = "only smooth and strong sharpen: polish and relax go from 0";
            return Err(errors::args_invalid("/amount", message));
        }
        Ok(if a > 0.0 {
            (a / PER_ROUND).ceil() as u32
        } else {
            0
        })
    }

    /// The `transform.apply` call that holds this call's weights: its frame, mask, symmetry,
    /// axes, gradient and box, with an operation that is not read.
    pub(crate) fn weighing(&self) -> TransformApply {
        TransformApply {
            part: self.part.clone(),
            frame: self.frame,
            mask: self.mask,
            symmetry: self.symmetry,
            axes: self.axes,
            gradient: self.gradient,
            bounds: self.bounds,
            ..TransformApply::new(OpArg::Move(Vec3([0.0; 3])))
        }
    }

    /// Writes the gizmo's pose into the call as numbers when the frame is "gizmo", and gives
    /// the call's weighing so pinned; refuses what [`Self::rounds`] refuses.
    pub(crate) fn pin_frame(
        &mut self,
        cx: &Ctx,
        part: &str,
    ) -> Result<TransformApply, CommandError> {
        self.rounds()?;
        let mut weighing = self.weighing();
        weighing.pin_frame(cx, part)?;
        self.frame = weighing.frame;
        Ok(weighing)
    }

    /// Writes into the call the box its gradient needs and leaves out, measured on `found`'s
    /// points as they are, and gives its weights on those points; refuses what
    /// `transform.apply` and [`Self::rounds`] refuse.
    pub(crate) fn settle(&mut self, found: &Part) -> Result<Transform, CommandError> {
        self.rounds()?;
        let mut weighing = self.weighing();
        let place = found.props().transform;
        let t = weighing.settle(found.store(), place, || found.world_diagonal())?;
        self.bounds = weighing.bounds;
        Ok(t)
    }

    /// Runs the smoothing on `store` inside its open step, `t` its weights: the rounds from
    /// `from` on (a store that already ran the first `from` since its step began), or the
    /// sharpening.
    pub(crate) fn apply(&self, store: &mut MeshStore, t: &Transform, from: u32) -> StepReport {
        let mode = match self.mode {
            SmoothArg::Smooth => SmoothMode::Uniform,
            SmoothArg::Strong => SmoothMode::Stronger,
            SmoothArg::Polish if self.keep => SmoothMode::Uniform,
            SmoothArg::Polish => SmoothMode::Tangent,
            SmoothArg::Relax => SmoothMode::Relax,
        };
        let keep = self.mode == SmoothArg::Polish && self.keep;
        let rounds = self.rounds().unwrap_or_default();
        if self.amount < 0.0 {
            store.sharpen(t, mode, -self.amount / 100.0)
        } else if rounds > from {
            store.smooth(t, (mode, keep), from..rounds)
        } else {
            StepReport::default()
        }
    }
}

/// The nodes a step wrote, ascending.
pub(crate) fn written(StepReport { brushed, shell }: StepReport) -> Vec<NodeId> {
    let mut nodes: Vec<NodeId> = brushed.into_iter().chain(shell).collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}
