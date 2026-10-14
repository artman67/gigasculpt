//! `transform.apply` (decision 0027, section 4): the one command that moves, turns, scales,
//! inflates, flattens or deforms (decision 0041) a part's points about a frame, with weights.
//! The gizmo's drags commit as it, and agents call it directly.

use gs_core::transform::{Distance, Frame, RIGID, Transform, Weights};
use gs_core::{MeshStore, NodeId, StepReport};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Length, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ops::{
    BendArg, BoxArg, ClipArg, CurveArg, GradientArg, LatticeArg, NoiseArg, ShearArg, SpherizeArg,
    TaperArg, TwistArg,
};
use super::{Reads, Route, edit_points, ops, route};
use crate::command::PartArg;
use crate::document::{Props, pinned};
use crate::mesh::resolve_length;
use crate::sculpt::{Axes, Axis, SymmetryArg};
use crate::{Command, Ctx, Example, errors, register_command};

/// Move, turn, scale, inflate or flatten a part's points about a frame.
///
/// Each point gets the transform scaled by its weight: hidden points weigh 0, the mask holds
/// points back (`mask`), and above -100 `falloff` fades the transform out with the distance
/// from the frame's origin, reaching 0 at `reach`. A weight scales the transform, not its
/// result: a point at half weight moves half the distance, or turns half the angle about the
/// same axis. With `symmetry`, the side of each mirror plane away from the frame's origin gets
/// the mirrored transform, and points on a plane stay on it; the side of a parallel plane away
/// from the origin gets the transform unmirrored, about the origin's mirror image. The planes go
/// through the part's own origin along its own axes. The gizmo's drags are logged as this
/// command, with the gizmo's frame written out as numbers.
///
/// A move, a turn or an even scale that every point gets fully (rigid, nothing masked or
/// hidden, no mirror plane on, and no parallel plane but under a move) moves the whole part: it
/// changes the part's transform and leaves its points as they are, which costs the same undo
/// whatever the part's size.
/// `part.apply_transform` writes the transform into the points.
///
/// On a part with detail levels shown below its top, an edit of the points runs on the top
/// when no edit waits to be carried and it is not `inflate` or a falloff measured along the
/// `surface`: every level gets the change of its own points, and switching level carries
/// nothing. Otherwise it edits the shown level, carried up at the next switch.
///
/// The deformers (bend, fold, twist, taper, shear, clip, spherize, noise, lattice, curve) work
/// in the frame: their axes are its axes, and those that work along a span measure it in
/// `box`, which, left out, is the box around the points that move, written into the call
/// before it runs. A weight scales a deformer's angle or amount, clip's and spherize's change,
/// the cage's offsets, and the curve's every turn, twist and scale. Any operation
/// takes `axes`, to keep its change along some of the frame's axes only, and `gradient`, to
/// fade it across the box; a deformer, or any operation with either, edits the points.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransformApply {
    #[doc = crate::command::part_doc!("The part to transform.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// What the transform turns and scales about: "gizmo" (the part's gizmo, the default) or
    /// an exact `{origin, axes}`. The engine writes the gizmo's numbers into the call before
    /// it runs, so the log line never depends on where the gizmo was.
    #[serde(default)]
    pub frame: FrameArg,
    /// What to do: exactly one of `move`, `rotate`, `scale`, `inflate`, `flatten`, `bend`,
    /// `fold`, `twist`, `taper`, `shear`, `clip`, `spherize`, `noise`, `lattice` and `curve`.
    pub op: OpArg,
    /// The mask holds points back: a fully masked point does not move. Default true.
    #[serde(default = "crate::command::yes")]
    pub mask: bool,
    /// -100 (the default) moves every point fully, a rigid transform. Above it, the transform
    /// fades out from the frame's origin to `reach`; higher values pull it in toward the
    /// origin, as a brush's `falloff` does.
    #[serde(default = "rigid")]
    #[schemars(range(min = -100, max = 100))]
    pub falloff: f32,
    /// How far the falloff reaches from the frame's origin: scene units, or a percentage such
    /// as "50%" of the part's bounding-box diagonal. Needed when `falloff` is above -100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Length>,
    /// How the distance for the falloff is measured: "straight" (the default) or "surface",
    /// along the visible surface from the point nearest the origin.
    #[serde(default)]
    pub distance: DistanceArg,
    /// Mirror and parallel planes through the part's origin, as `brush.stroke` takes them.
    /// Radial symmetry is not supported for transforms yet and is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<SymmetryArg>,
    /// The frame's axes whose part of the change is kept, at least one; left out, all three.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<Axes>,
    /// Fade the operation across the box along one of the frame's axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<GradientArg>,
    /// The box a deformer's span and the gradient are measured in, in the frame. Left out
    /// where one is needed, the box around the points that move, written in before it runs.
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoxArg>,
}

fn rigid() -> f32 {
    RIGID
}

/// A transform's frame: the part's gizmo, or given exactly.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FrameArg {
    /// "gizmo": the part's gizmo as it is now.
    Gizmo(GizmoWord),
    /// A frame given exactly.
    Exact(ExactFrame),
}

impl Default for FrameArg {
    fn default() -> Self {
        Self::Gizmo(GizmoWord::Gizmo)
    }
}

/// The word "gizmo".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GizmoWord {
    Gizmo,
}

/// A frame: a point and three axes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExactFrame {
    /// The point the transform turns and scales about, in scene units.
    pub origin: Vec3,
    /// The frame's axes as a quaternion `[x, y, z, w]`; its rotation's columns are the frame's
    /// X, Y and Z. Made unit length before use; it must not be zero.
    pub axes: [f32; 4],
}

/// One transform.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpArg {
    /// Move by a vector, in scene units.
    Move(Vec3),
    /// Turn about an axis through the frame's origin.
    Rotate(RotateArg),
    /// Scale about the frame's origin by a factor along each of the frame's axes, each 0 or
    /// more: one below zero would turn points inside out.
    Scale([f32; 3]),
    /// Move each point along its normal, in scene units: positive outward.
    Inflate(f32),
    /// Flatten toward the frame's plane across an axis.
    Flatten(FlattenArg),
    /// Curve the box's span along an axis into an arc.
    Bend(BendArg),
    /// Crease at the plane through the frame's origin across an axis.
    Fold(BendArg),
    /// Turn each slice about an axis, more along the box.
    Twist(TwistArg),
    /// Narrow (or widen) each slice along an axis.
    Taper(TaperArg),
    /// Slide each slice along another axis.
    Shear(ShearArg),
    /// Put every point past a plane inside the box onto it.
    Clip(ClipArg),
    /// Move points toward a sphere about the frame's origin.
    Spherize(SpherizeArg),
    /// Move points by a smooth noise of where they are.
    Noise(NoiseArg),
    /// Move points with a cage of points across the box, some of them moved.
    Lattice(LatticeArg),
    /// Lay the box's span along an axis along a curve through points.
    Curve(CurveArg),
}

/// A turn.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateArg {
    /// "x", "y" or "z" (the frame's axis), or a vector in scene space.
    pub axis: AxisArg,
    /// How far, in degrees, counterclockwise looking down the axis toward the origin.
    pub degrees: f32,
}

/// An axis to turn about.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AxisArg {
    /// One of the frame's axes.
    Frame(Axis),
    /// A vector in scene space; it must not be zero.
    Vector(Vec3),
}

/// A flatten.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlattenArg {
    /// The frame's axis to flatten along: the points move toward the plane across it.
    pub axis: Axis,
    /// How far: 0 leaves the points, 1 puts them on the plane.
    #[schemars(range(min = 0, max = 1))]
    pub amount: f32,
}

/// How a distance from the frame's origin is measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DistanceArg {
    /// The straight line.
    #[default]
    Straight,
    /// Along the visible surface.
    Surface,
}

/// What `transform.apply` did.
#[derive(Serialize, JsonSchema)]
pub struct Applied {
    /// The part it transformed.
    pub part: String,
    /// The frame it used, exactly.
    pub frame: ExactFrame,
    /// The box it measured against, when it needed one.
    #[serde(rename = "box", skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoxArg>,
    /// The radius spherize used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
}

impl Command for TransformApply {
    const NAME: &'static str = "transform.apply";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    // Decision 0041, section 8 (amended 2026-10-12): the identity of each operation, and the
    // frame, box and radius the result reports pinned for the re-runs on the parts baked. The
    // worst seen: placement 5.8e-7 (noise on a placed part), symmetry exact.
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}},
        {"mask_holds": {"switch": "/mask"}}, "hidden_holds", "positions_only",
        {"identity_at_zero": {"set": {"/op/move": [0, 0, 0], "/op/rotate/degrees": 0,
            "/op/scale": [1, 1, 1], "/op/inflate": 0, "/op/flatten/amount": 0,
            "/op/bend/degrees": 0, "/op/fold/degrees": 0, "/op/twist/degrees": 0,
            "/op/taper/amount": 0, "/op/shear/amount": 0, "/op/clip/depth": 0,
            "/op/spherize/amount": 0, "/op/noise/amount": 0, "/op/lattice/offsets": [],
            "/op/curve/points": null, "/op/curve/twist": null, "/op/curve/scale": null}}},
        {"placement_invariant": {"pinned": {"/frame": "/frame", "/box": "/box",
            "/op/spherize/radius": "/radius"}, "unless": "/symmetry", "tolerance": 6e-6}},
        {"keeps_symmetry": {"planes": "/symmetry/mirror", "origin": "/frame/origin",
            "pinned": {"/frame": "/frame", "/box": "/box", "/op/spherize/radius": "/radius"},
            "tolerance": 1e-6}}]"#;
    type Output = Applied;

    fn examples() -> Vec<Example<Self>> {
        let sphere = || json!({ "subdivisions": 3 });
        let at = |origin| {
            FrameArg::Exact(ExactFrame {
                origin: Vec3(origin),
                axes: [0.0, 0.0, 0.0, 1.0],
            })
        };
        let lift = Self::new(OpArg::Move(Vec3([0.0, 0.1, 0.0])));
        let turn = Self::new(OpArg::Rotate(RotateArg {
            axis: AxisArg::Frame(Axis::Z),
            degrees: 30.0,
        }));
        let squash = Self {
            frame: at([0.0, 0.5, 0.0]),
            ..Self::new(OpArg::Scale([1.0, 0.5, 1.0]))
        };
        let swell = Self {
            frame: at([0.0, 0.5, 0.0]),
            falloff: 0.0,
            reach: Some(Length::Percent(40.0)),
            distance: DistanceArg::Surface,
            ..Self::new(OpArg::Inflate(0.05))
        };
        let x = Axes {
            x: true,
            ..Axes::default()
        };
        let press = Self {
            frame: at([0.5, 0.0, 0.0]),
            symmetry: Some(SymmetryArg {
                mirror: x,
                ..SymmetryArg::default()
            }),
            ..Self::new(OpArg::Flatten(FlattenArg {
                axis: Axis::X,
                amount: 0.5,
            }))
        };
        let lean = Self {
            frame: at([0.5, 0.0, 0.0]),
            falloff: 0.0,
            reach: Some(Length::Units(0.8)),
            symmetry: Some(SymmetryArg {
                parallel: x,
                ..SymmetryArg::default()
            }),
            ..Self::new(OpArg::Rotate(RotateArg {
                axis: AxisArg::Frame(Axis::Z),
                degrees: 20.0,
            }))
        };
        let radial = json!({
            "op": { "move": [0.0, 0.1, 0.0] },
            "symmetry": { "radial": { "axis": "y", "count": 4 } },
        });
        let mirror = json!({ "op": { "scale": [-1.0, 1.0, 1.0] } });
        let plain = vec![
            Example::new("move the only part up a tenth of a unit", lift)
                .after("primitive.sphere", sphere()),
            Example::new("turn it 30 degrees about the gizmo's z", turn)
                .after("primitive.sphere", sphere()),
            Example::new("squash it to half its height about its top", squash)
                .after("primitive.sphere", sphere()),
            Example::new("swell the top softly, measured along the surface", swell)
                .after("primitive.sphere", sphere()),
            Example::new("flatten both sides at once, mirrored across x", press)
                .after("primitive.sphere", sphere()),
            Example::new("lean both sides the same way, parallel across x", lean)
                .after("primitive.sphere", sphere()),
            Example::raw("radial symmetry is refused", radial)
                .after("primitive.sphere", sphere())
                .fails_with(ErrorCode::ArgsInvalid),
            Example::raw(
                "a scale below 0 is refused: it turns the part inside out",
                mirror,
            )
            .after("primitive.sphere", sphere())
            .fails_with(ErrorCode::ArgsInvalid),
        ];
        plain.into_iter().chain(ops::examples()).collect()
    }

    /// Pins the frame, and the box and spherize radius the call leaves out and needs.
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let part = pinned(self.part.clone())?;
        self.pin_frame(cx, &part)?;
        let found = cx.doc.get_part(&part)?;
        let place = found.props().transform;
        self.settle(found.store(), place, || found.world_diagonal())?;
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<Applied, CommandError> {
        let part = pinned(self.part.clone())?;
        let found = cx.doc.get_part(&part)?;
        let t = self.resolve(|| found.world_diagonal())?;
        let props = found.props();
        let route = route(found, props.transform, &t);
        cx.noop_reason(NO_CHANGE);
        match route {
            Route::Placed(transform) => cx.doc.set_props(&part, Props { transform, ..props })?,
            Route::Points(t) => edit_points(cx, &part, Reads::of(&t), |s| apply(s, &t))?,
        }
        self.applied(part)
    }
}
register_command!(TransformApply, "Transform");

/// Why a transform changed nothing.
pub(crate) const NO_CHANGE: &str = "the transform left every point where it was";

/// Applies `t` to `store`, inside its open step, and returns the nodes it wrote, ascending.
pub(crate) fn apply(store: &mut MeshStore, t: &Transform) -> Vec<NodeId> {
    let StepReport { brushed, shell } = store.transform(t);
    let mut written: Vec<NodeId> = brushed.into_iter().chain(shell).collect();
    written.sort_unstable();
    written.dedup();
    written
}

impl TransformApply {
    /// A rigid `op` about the gizmo, on the active part.
    pub(crate) fn new(op: OpArg) -> Self {
        Self {
            part: None,
            frame: FrameArg::default(),
            op,
            mask: true,
            falloff: RIGID,
            reach: None,
            distance: DistanceArg::Straight,
            symmetry: None,
            axes: None,
            gradient: None,
            bounds: None,
        }
    }

    /// What the call did to `part`, as its result reports it.
    pub(crate) fn applied(&self, part: String) -> Result<Applied, CommandError> {
        Ok(Applied {
            part,
            frame: self.exact()?,
            bounds: self.bounds,
            radius: self.radius(),
        })
    }

    /// Writes the gizmo's pose into the call as numbers when the frame is "gizmo". A given frame
    /// is kept as it was written, so pinning a logged call again changes nothing; it is made
    /// unit length when it runs.
    pub(crate) fn pin_frame(&mut self, cx: &Ctx, part: &str) -> Result<(), CommandError> {
        if let FrameArg::Gizmo(_) = self.frame {
            let (pose, _) = crate::gizmo::pose_of(cx, part)?;
            self.frame = FrameArg::Exact(ExactFrame {
                origin: Vec3(pose.origin.to_array()),
                axes: pose.axes.to_array(),
            });
        }
        Ok(())
    }

    /// The pinned frame.
    pub(crate) fn exact(&self) -> Result<ExactFrame, CommandError> {
        match self.frame {
            FrameArg::Exact(f) => Ok(f),
            FrameArg::Gizmo(_) => Err(errors::internal("the frame was not pinned")),
        }
    }

    /// The transform in the scene, `diagonal` giving the part's bounding-box diagonal in the
    /// scene before the transform (asked only for a `reach` in percent). Refuses numbers that
    /// are not finite, a zero axis, a scale factor below 0, a flatten outside 0 to 1, a missing
    /// or empty reach, radial symmetry and a plane both mirror and parallel.
    pub(crate) fn resolve(
        &self,
        diagonal: impl FnOnce() -> f32,
    ) -> Result<Transform, CommandError> {
        let f = self.exact()?;
        let axes = unit_quat(f.axes).ok_or_else(|| {
            errors::args_invalid("/frame/axes", "axes must be a quaternion that is not zero")
        })?;
        let frame = Frame {
            origin: gs_core::Vec3::from_array(finite("/frame/origin", f.origin.0)?),
            axes: gs_core::Quat::from_array(axes),
        };
        Ok(Transform {
            op: self.op(frame)?,
            frame,
            weights: self.weights(diagonal)?,
            symmetry: self.symmetry()?,
            scope: self.scope()?,
        })
    }

    fn weights(&self, diagonal: impl FnOnce() -> f32) -> Result<Weights, CommandError> {
        let [falloff] = finite("/falloff", [self.falloff])?;
        if !(-100.0..=100.0).contains(&falloff) {
            let message = "falloff must be from -100 to 100";
            return Err(errors::args_invalid("/falloff", message));
        }
        let reach = match self.reach {
            _ if falloff <= RIGID => 0.0,
            Some(Length::Units(units)) => units,
            Some(length) => resolve_length(length, diagonal()),
            None => {
                let message = "a falloff above -100 needs a reach";
                return Err(errors::args_invalid("/reach", message));
            }
        };
        if falloff > RIGID && !(reach > 0.0 && reach.is_finite()) {
            let message = "reach must be a distance above 0";
            return Err(errors::args_invalid("/reach", message));
        }
        Ok(Weights {
            mask: self.mask,
            falloff,
            reach,
            distance: match self.distance {
                DistanceArg::Straight => Distance::Straight,
                DistanceArg::Surface => Distance::Surface,
            },
        })
    }

    fn symmetry(&self) -> Result<gs_core::symmetry::Symmetry, CommandError> {
        let Some(s) = self.symmetry else {
            return Ok(gs_core::symmetry::Symmetry::default());
        };
        if s.radial.is_some() {
            let message = "radial symmetry is not supported for transforms yet";
            return Err(errors::args_invalid("/symmetry/radial", message));
        }
        s.checked()
    }
}

/// `v` if every number in it is finite, else `E_ARGS_INVALID` at `path`.
pub(crate) fn finite<const N: usize>(path: &str, v: [f32; N]) -> Result<[f32; N], CommandError> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(v)
    } else {
        Err(errors::args_invalid(path, "every number must be finite"))
    }
}

/// `q` at unit length, worked out in `f64` so tiny quaternions do not underflow; `None` for a
/// zero or not finite quaternion.
pub(crate) fn unit_quat(q: [f32; 4]) -> Option<[f32; 4]> {
    let d = q.map(f64::from);
    let n = d.iter().map(|x| x * x).sum::<f64>().sqrt();
    (n > 0.0 && n.is_finite()).then(|| d.map(|x| (x / n) as f32))
}
