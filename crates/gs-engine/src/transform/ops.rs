//! The operations `transform.apply` takes, the deformers among them (decision 0041, amended
//! 2026-10-12), and the three arguments every operation shares: `axes`, `gradient` and `box`.
//! Each is checked here, where an error can name the very field (the schema can only name the
//! `op` or the optional field around it), and resolved into gs-core's [`Op`] and [`Scope`]. A
//! call is settled before it runs: the box and the spherize radius it leaves out are measured on
//! the part's points and written into it, so its log line holds them and a replay measures
//! nothing.

use gs_core::deform::{Curve, Deform, Gradient, Lattice, Offset, Scope};
use gs_core::transform::{Frame, Op, Similarity, Transform};
use gs_core::{Aabb, MeshStore};
use gs_protocol::{CommandError, ErrorCode, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::apply::{
    AxisArg, ExactFrame, FlattenArg, FrameArg, OpArg, RotateArg, TransformApply, finite,
};
use crate::sculpt::{Axes, Axis, SymmetryArg};
use crate::{Example, errors};

/// A bend or a fold.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BendArg {
    /// The frame's axis the shape runs along: a bend curves the box's span along it, a fold
    /// creases it at the plane through the frame's origin across it.
    pub axis: Axis,
    /// The frame's axis it turns toward; not `axis`.
    pub toward: Axis,
    /// How far, in degrees: a bend's whole arc, -3600 to 3600; a fold's crease, -360 to 360.
    pub degrees: f32,
}

/// A twist.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TwistArg {
    /// The frame's axis to twist about.
    pub axis: Axis,
    /// The turn across the box's span, in degrees, -3600 to 3600; the slice through the frame's
    /// origin stays.
    pub degrees: f32,
}

/// A taper.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaperArg {
    /// The frame's axis to taper along.
    pub axis: Axis,
    /// -1 to 1: how much narrower the box's far face gets (wider below 0); the other side of
    /// the frame's origin the other way.
    pub amount: f32,
    /// The taper's curve along the axis, 0.5 to 10: 1 (the default) is straight.
    #[serde(default = "one")]
    pub exponent: f32,
}

fn one() -> f32 {
    1.0
}

/// A shear.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShearArg {
    /// The frame's axis whose slices slide.
    pub axis: Axis,
    /// The frame's axis they slide along; not `axis`.
    pub toward: Axis,
    /// How far the box's far face slides, in scene units.
    pub amount: f32,
    /// "linear" (the default) or "smooth", easing in and out.
    #[serde(default)]
    pub profile: ShearProfile,
}

/// How a shear slides along its span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ShearProfile {
    #[default]
    Linear,
    Smooth,
}

/// A clip.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipArg {
    /// The frame's axis to clip along.
    pub axis: Axis,
    /// How far inside the box's top face the plane is, in scene units; below 0, inside its
    /// bottom face. Every point past the plane goes onto it.
    pub depth: f32,
}

/// A spherize.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpherizeArg {
    /// -1 to 1: how far each point goes toward the sphere (away from it below 0).
    pub amount: f32,
    /// The sphere's radius about the frame's origin, in scene units, 0 or more. Left out, the
    /// mean distance of the points that move; the engine writes it in before the call runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
}

/// A noise.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseArg {
    /// The most a point moves along each of the frame's axes, in scene units.
    pub amount: f32,
    /// The size of the noise's cells, in scene units, above 0.
    pub scale: f32,
    /// Which noise: the same seed gives the same noise at the same place. Default 0.
    #[serde(default)]
    pub seed: u64,
}

/// A cage: a lattice of points across the box, some moved, and the part moved with it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LatticeArg {
    /// "soft" (the default): each moved point bends the part smoothly over two cells each way.
    /// "hard": over one cell, straight between the points.
    #[serde(default)]
    pub kind: LatticeKind,
    /// The cage's points along the frame's x, y and z, each 2 to 32, resting evenly across
    /// the box from its low corner to its high one.
    pub size: [u32; 3],
    /// The moved points; every other point stays. Points moved by zero are left out of the
    /// logged call.
    #[serde(default)]
    pub offsets: Vec<OffsetArg>,
}

/// How a cage bends the part between its points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LatticeKind {
    #[default]
    Soft,
    Hard,
}

/// One moved cage point.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OffsetArg {
    /// The point's place along x, y and z, counted from the box's low corner, each below the
    /// size.
    pub at: [u32; 3],
    /// How far it moves along the frame's axes, in scene units.
    pub by: Vec3,
}

/// A curve the part is laid along.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurveArg {
    /// The frame's axis the part runs along; its distance along the axis from the box's low
    /// face becomes its distance along the curve.
    pub axis: Axis,
    /// 2 to 32 points in the frame the curve passes through, in order, each off the one before
    /// and not turning straight back. Left out, the axis line through the frame's origin from
    /// the box's low face to its high one, which bends nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Vec3>>,
    /// The turn about the curve at each point, in degrees, -3600 to 3600: one a point (two
    /// when the points are left out). Left out, none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twist: Option<Vec<f32>>,
    /// The part's cross-section scale at each point, 0 to 10, one a point. Left out, 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Vec<f32>>,
}

/// A fade of the operation along one of the frame's axes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientArg {
    /// The frame's axis it fades along.
    pub axis: Axis,
    /// Where across the box the fade starts, 0 to 1: full before it.
    pub from: f32,
    /// Where it ends, 0 to 1 and at least 0.01 from `from`: nothing after it.
    pub to: f32,
}

/// A box in the frame: offsets from its origin along its axes, in scene units.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxArg {
    pub lo: Vec3,
    /// Not below `lo` on any axis.
    pub hi: Vec3,
}

impl TransformApply {
    /// The operation about `frame`, checked: finite numbers, a turn's axis not zero, no scale
    /// factor below 0, a flatten from 0 to 1, and what [`deform`] refuses.
    pub(super) fn op(&self, frame: Frame) -> Result<Op, CommandError> {
        Ok(match self.op {
            OpArg::Move(Vec3(d)) => Op::Move(gs_core::Vec3::from_array(finite("/op/move", d)?)),
            OpArg::Rotate(RotateArg { axis, degrees }) => {
                let [degrees] = finite("/op/rotate/degrees", [degrees])?;
                let (x, y, z) = frame.axes.axes();
                let axis = match axis {
                    AxisArg::Frame(a) => [x, y, z][a as usize],
                    AxisArg::Vector(Vec3(v)) => unit(v).ok_or_else(|| {
                        errors::args_invalid("/op/rotate/axis", "the axis must not be zero")
                    })?,
                };
                Op::Rotate {
                    axis,
                    angle: degrees.to_radians(),
                }
            }
            OpArg::Scale(s) => {
                // Below zero turns points inside out; under a falloff even two such factors do,
                // where a weight has taken one of them past zero and not the other.
                if s.iter().any(|&k| k < 0.0) {
                    let why = "a factor below 0 turns points inside out";
                    let fix = "to mirror, use mesh.flip; to turn half way, rotate 180";
                    return Err(CommandError {
                        hint: Some(fix.into()),
                        ..errors::args_invalid("/op/scale", why)
                    });
                }
                Op::Scale(finite("/op/scale", s)?)
            }
            OpArg::Inflate(a) => Op::Inflate(finite("/op/inflate", [a])?[0]),
            OpArg::Flatten(FlattenArg { axis, amount }) => {
                if !(0.0..=1.0).contains(&amount) {
                    let message = "amount must be from 0 to 1";
                    return Err(errors::args_invalid("/op/flatten/amount", message));
                }
                let mut s = [1.0; 3];
                s[axis as usize] = 1.0 - amount;
                Op::Scale(s)
            }
            ref op => Op::Deform(deform(op.clone())?),
        })
    }
}

/// `v` at unit length, worked out in `f64` so tiny vectors do not underflow; `None` for a zero
/// or not finite vector.
fn unit(v: [f32; 3]) -> Option<gs_core::Vec3> {
    let d = v.map(f64::from);
    let n = d.iter().map(|x| x * x).sum::<f64>().sqrt();
    (n > 0.0 && n.is_finite()).then(|| gs_core::Vec3::from_array(d.map(|x| (x / n) as f32)))
}

/// The deformer `op` names, checked: degrees, amounts, the exponent and the radius within
/// their ranges, lengths finite, noise's cells above 0, `toward` not the axis itself, and the
/// cage and the curve as [`lattice`] and [`curve`] check them.
fn deform(op: OpArg) -> Result<Deform, CommandError> {
    Ok(match op {
        OpArg::Bend(b) => {
            let (axis, toward, angle) = turning("/op/bend", b, 3600.0)?;
            Deform::Bend {
                axis,
                toward,
                angle,
            }
        }
        OpArg::Fold(b) => {
            let (axis, toward, angle) = turning("/op/fold", b, 360.0)?;
            Deform::Fold {
                axis,
                toward,
                angle,
            }
        }
        OpArg::Twist(TwistArg { axis, degrees }) => Deform::Twist {
            axis: axis as usize,
            angle: within("/op/twist/degrees", degrees, -3600.0, 3600.0)?.to_radians(),
        },
        OpArg::Taper(TaperArg {
            axis,
            amount,
            exponent,
        }) => Deform::Taper {
            axis: axis as usize,
            amount: within("/op/taper/amount", amount, -1.0, 1.0)?,
            exponent: within("/op/taper/exponent", exponent, 0.5, 10.0)?,
        },
        OpArg::Shear(s) => Deform::Shear {
            axis: s.axis as usize,
            toward: across("/op/shear/toward", s.axis, s.toward)?,
            amount: finite("/op/shear/amount", [s.amount])?[0],
            smooth: s.profile == ShearProfile::Smooth,
        },
        OpArg::Clip(ClipArg { axis, depth }) => Deform::Clip {
            axis: axis as usize,
            depth: finite("/op/clip/depth", [depth])?[0],
        },
        OpArg::Spherize(SpherizeArg { amount, radius }) => Deform::Spherize {
            amount: within("/op/spherize/amount", amount, -1.0, 1.0)?,
            radius: radius
                .map(|r| distance("/op/spherize/radius", r, 0.0))
                .transpose()?,
        },
        OpArg::Noise(NoiseArg {
            amount,
            scale,
            seed,
        }) => Deform::Noise {
            amount: finite("/op/noise/amount", [amount])?[0],
            scale: distance("/op/noise/scale", scale, f32::MIN_POSITIVE)?,
            seed,
        },
        OpArg::Lattice(l) => Deform::Lattice(lattice(&l)?),
        OpArg::Curve(c) => Deform::Curve(curve(c)?),
        _ => return Err(errors::internal("not a deformer")),
    })
}

/// A cage, checked: each size 2 to 32, and each offset's point inside the cage, named once,
/// moved by finite numbers.
fn lattice(l: &LatticeArg) -> Result<Lattice, CommandError> {
    if l.size.iter().any(|s| !(2..=32).contains(s)) {
        let message = "each must be from 2 to 32";
        return Err(errors::args_invalid("/op/lattice/size", message));
    }
    let mut named = std::collections::BTreeSet::new();
    for (i, o) in l.offsets.iter().enumerate() {
        let at = format!("/op/lattice/offsets/{i}/at");
        if (0..3).any(|k| o.at[k] >= l.size[k]) {
            return Err(errors::args_invalid(
                &at,
                "must be below the size on every axis",
            ));
        }
        if !named.insert(o.at) {
            return Err(errors::args_invalid(
                &at,
                "names a point an offset before moved",
            ));
        }
        finite(&format!("/op/lattice/offsets/{i}/by"), o.by.0)?;
    }
    let offsets = (l.offsets.iter())
        .map(|o| Offset {
            at: o.at,
            by: o.by.0,
        })
        .collect();
    Ok(Lattice {
        soft: l.kind == LatticeKind::Soft,
        size: l.size,
        offsets,
    })
}

/// A curve, checked: twists -3600 to 3600 degrees, scales 0 to 10, and what
/// [`Curve::fault`] finds.
fn curve(c: CurveArg) -> Result<Curve, CommandError> {
    let ranges = [("twist", &c.twist, 3600.0), ("scale", &c.scale, 10.0)];
    for (name, values, most) in ranges {
        let least = if name == "twist" { -most } else { 0.0 };
        for (i, &v) in values.iter().flatten().enumerate() {
            within(&format!("/op/curve/{name}/{i}"), v, least, most)?;
        }
    }
    let curve = Curve {
        axis: c.axis as usize,
        points: (c.points).map(|p| p.iter().map(|v| v.0).collect()),
        twist: (c.twist).map(|t| t.iter().map(|d| d.to_radians()).collect()),
        scale: c.scale,
    };
    match curve.fault() {
        Some((at, why)) => Err(errors::args_invalid(&format!("/op/curve/{at}"), why)),
        None => Ok(curve),
    }
}

/// A bend's or a fold's axes and angle in radians, the degrees at most `most` either way.
fn turning(at: &str, b: BendArg, most: f32) -> Result<(usize, usize, f32), CommandError> {
    let toward = across(&format!("{at}/toward"), b.axis, b.toward)?;
    let degrees = within(&format!("{at}/degrees"), b.degrees, -most, most)?;
    Ok((b.axis as usize, toward, degrees.to_radians()))
}

/// `toward` as an index, refused when it is `axis`.
fn across(at: &str, axis: Axis, toward: Axis) -> Result<usize, CommandError> {
    if toward == axis {
        return Err(errors::args_invalid(at, "toward must be another axis"));
    }
    Ok(toward as usize)
}

/// `v` if it is from `lo` to `hi`.
fn within(at: &str, v: f32, lo: f32, hi: f32) -> Result<f32, CommandError> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(errors::args_invalid(
            at,
            format!("must be from {lo} to {hi}"),
        ))
    }
}

/// `v` if it is a finite distance of at least `least`.
fn distance(at: &str, v: f32, least: f32) -> Result<f32, CommandError> {
    if v >= least && v.is_finite() {
        Ok(v)
    } else if least > 0.0 {
        Err(errors::args_invalid(at, "must be a distance above 0"))
    } else {
        Err(errors::args_invalid(at, "must be a distance of 0 or more"))
    }
}

impl TransformApply {
    /// The shared arguments, checked: at least one axis, a gradient's ends 0 to 1 and 0.01
    /// apart, a box's numbers finite and `lo` not above `hi`.
    pub(super) fn scope(&self) -> Result<Scope, CommandError> {
        let axes = match self.axes {
            None => [true; 3],
            Some(Axes { x, y, z }) if x || y || z => [x, y, z],
            Some(_) => return Err(errors::args_invalid("/axes", "choose at least one axis")),
        };
        let gradient = (self.gradient)
            .map(|g| {
                let from = within("/gradient/from", g.from, 0.0, 1.0)?;
                let to = within("/gradient/to", g.to, 0.0, 1.0)?;
                if (to - from).abs() < 0.01 {
                    let message = "from and to must be at least 0.01 apart";
                    return Err(errors::args_invalid("/gradient", message));
                }
                let axis = g.axis as usize;
                Ok(Gradient { axis, from, to })
            })
            .transpose()?;
        let bounds = (self.bounds)
            .map(|b| {
                let (lo, hi) = (finite("/box/lo", b.lo.0)?, finite("/box/hi", b.hi.0)?);
                if (0..3).any(|k| lo[k] > hi[k]) {
                    let message = "lo must not be above hi on any axis";
                    return Err(errors::args_invalid("/box", message));
                }
                let (lo, hi) = (lo.map(|v| v + 0.0), hi.map(|v| v + 0.0));
                let [lo, hi] = [lo, hi].map(gs_core::Vec3::from_array);
                Ok(Aabb { lo, hi })
            })
            .transpose()?;
        Ok(Scope {
            axes,
            gradient,
            bounds,
        })
    }

    /// Writes into the call the box and the spherize radius it leaves out and needs, measured
    /// on `store`'s points as the part placed by `place` holds them, in scene units, and drops
    /// the cage's offsets of zero; then gives the transform of the part's own points the call
    /// makes now, which is what its replay makes, bit for bit. `diagonal` gives the part's
    /// bounding-box diagonal in the scene.
    pub(crate) fn settle(
        &mut self,
        store: &MeshStore,
        place: Similarity,
        diagonal: impl Fn() -> f32,
    ) -> Result<Transform, CommandError> {
        let t = store.measured(&place.part_transform(&self.resolve(&diagonal)?));
        if let OpArg::Lattice(l) = &mut self.op {
            l.offsets.retain(|o| o.by.0.iter().any(|&b| b != 0.0));
        }
        let scene = |v: gs_core::Vec3| Vec3(v.to_array().map(|l| place.length(l)));
        if self.bounds.is_none()
            && let Some(b) = t.scope.bounds
        {
            let (lo, hi) = (scene(b.lo), scene(b.hi));
            self.bounds = Some(BoxArg { lo, hi });
        }
        if let (OpArg::Spherize(s), Op::Deform(Deform::Spherize { radius, .. })) =
            (&mut self.op, t.op)
            && s.radius.is_none()
        {
            s.radius = radius.map(|r| place.length(r));
        }
        Ok(place.part_transform(&self.resolve(&diagonal)?))
    }

    /// The spherize radius the call holds.
    pub(super) fn radius(&self) -> Option<f32> {
        match self.op {
            OpArg::Spherize(s) => s.radius,
            _ => None,
        }
    }
}

/// The frame origin of most examples, off the sphere's vertices.
const OFF: [f32; 3] = [0.05, 0.03, 0.02];

/// `op` about an unturned frame at `origin`.
fn call(op: OpArg, origin: [f32; 3]) -> TransformApply {
    let frame = FrameArg::Exact(ExactFrame {
        origin: Vec3(origin),
        axes: [0.0, 0.0, 0.0, 1.0],
    });
    TransformApply {
        frame,
        ..TransformApply::new(op)
    }
}

/// `op` mirrored across x, about a frame off the plane.
fn mirrored(op: OpArg) -> TransformApply {
    let mirror = Axes {
        x: true,
        ..Axes::default()
    };
    TransformApply {
        symmetry: Some(SymmetryArg {
            mirror,
            ..SymmetryArg::default()
        }),
        ..call(op, [0.3, 0.05, 0.02])
    }
}

/// The examples of bend, fold, twist and taper.
fn turns_and_taper() -> [(&'static str, TransformApply); 4] {
    let (bend, fold) = (
        BendArg {
            axis: Axis::Y,
            toward: Axis::X,
            degrees: 60.0,
        },
        BendArg {
            axis: Axis::X,
            toward: Axis::Y,
            degrees: 40.0,
        },
    );
    let twist = TwistArg {
        axis: Axis::Y,
        degrees: 90.0,
    };
    let taper = TaperArg {
        axis: Axis::Y,
        amount: 0.5,
        exponent: 2.0,
    };
    [
        (
            "bend the sphere 60 degrees toward x along its height",
            call(OpArg::Bend(bend), OFF),
        ),
        ("fold it 40 degrees across x", call(OpArg::Fold(fold), OFF)),
        (
            "twist both sides a quarter turn about y, mirrored across x",
            mirrored(OpArg::Twist(twist)),
        ),
        (
            "taper it to half at the top, along a curve",
            call(OpArg::Taper(taper), [0.02, -0.5, 0.03]),
        ),
    ]
}

/// The examples of the cage, soft and hard, and the curve.
fn cage_and_curve() -> [(&'static str, TransformApply); 3] {
    let cage = |kind, n, at, by| LatticeArg {
        kind,
        size: [n; 3],
        offsets: vec![OffsetArg { at, by: Vec3(by) }],
    };
    let curve = CurveArg {
        axis: Axis::Y,
        points: Some(
            [
                [0.0, -1.0, 0.0],
                [0.3, -0.3, 0.0],
                [-0.3, 0.3, 0.0],
                [0.0, 1.0, 0.0],
            ]
            .map(Vec3)
            .to_vec(),
        ),
        twist: Some(vec![0.0, 30.0, 60.0, 90.0]),
        scale: Some(vec![1.0, 1.2, 1.2, 1.0]),
    };
    let (soft, hard) = (LatticeKind::Soft, LatticeKind::Hard);
    [
        (
            "lift the top centre of a soft 3 by 3 by 3 cage",
            call(
                OpArg::Lattice(cage(soft, 3, [1, 2, 1], [0.0, 0.4, 0.0])),
                OFF,
            ),
        ),
        (
            "pull a hard 2 by 2 by 2 cage's top corner out, mirrored across x",
            mirrored(OpArg::Lattice(cage(hard, 2, [1, 1, 1], [0.2, 0.2, 0.2]))),
        ),
        (
            "lay it along an S up y, twisting a quarter turn and swelling in the middle",
            call(OpArg::Curve(curve), OFF),
        ),
    ]
}

/// One example of each deformer and of the shared arguments, on a sphere, about frames off its
/// vertices; four across a mirror plane the frame is off.
pub(super) fn examples() -> Vec<Example<TransformApply>> {
    let shear = ShearArg {
        axis: Axis::Y,
        toward: Axis::Z,
        amount: 0.3,
        profile: ShearProfile::Smooth,
    };
    let clip = ClipArg {
        axis: Axis::X,
        depth: 0.2,
    };
    let spherize = TransformApply {
        axes: Some(Axes {
            x: true,
            z: true,
            ..Axes::default()
        }),
        ..call(
            OpArg::Spherize(SpherizeArg {
                amount: 0.6,
                radius: None,
            }),
            OFF,
        )
    };
    let noise = NoiseArg {
        amount: 0.03,
        scale: 0.2,
        seed: 7,
    };
    let fade = TransformApply {
        gradient: Some(GradientArg {
            axis: Axis::Y,
            from: 0.3,
            to: 0.8,
        }),
        ..call(OpArg::Move(Vec3([0.2, 0.0, 0.0])), OFF)
    };
    let own = json!({ "op": { "bend": { "axis": "y", "toward": "y", "degrees": 30.0 } } });
    let sphere = || json!({ "subdivisions": 3 });
    let rest = [
        (
            "shear its top a third of a unit along z, easing in",
            call(OpArg::Shear(shear), OFF),
        ),
        (
            "clip a fifth of a unit off both sides, mirrored across x",
            mirrored(OpArg::Clip(clip)),
        ),
        (
            "round it toward a cylinder about y: spherize over x and z",
            spherize,
        ),
        (
            "roughen both sides with a little noise, mirrored across x",
            mirrored(OpArg::Noise(noise)),
        ),
        ("move it along x, fading out up its height", fade),
    ];
    (turns_and_taper()
        .into_iter()
        .chain(rest)
        .chain(cage_and_curve()))
    .map(|(summary, args)| Example::new(summary, args).after("primitive.sphere", sphere()))
    .chain([Example::raw("a bend toward its own axis is refused", own)
        .after("primitive.sphere", sphere())
        .fails_with(ErrorCode::ArgsInvalid)])
    .collect()
}
