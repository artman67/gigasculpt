//! A handle the pen holds: the transform its motion makes, as `transform.apply` takes it
//! (decision 0027, sections 4 and 11). The pen's ray is measured against the handle in the
//! scene, so a move follows the pen under perspective: an arrow by the point of its axis
//! nearest the ray, a plane square by where the ray meets its plane, the centre square by where
//! it meets the screen plane through the origin. A ring turns by the angle the pen sweeps
//! around the gizmo's centre on screen; the grip scales by how far the pen is from that centre
//! against how far the press was. With Ctrl the drag moves a copy of the part (decision 0027,
//! section 14).

use gs_core::camera::Camera;
use gs_core::gizmo::{
    HandleId, Handles, MOVE_SNAP, Pose, along_ray, on_plane, snap_move, snap_turn, sweep,
    toward_viewer,
};
use gs_core::{Quat, Vec3};
use gs_engine::BrushState;
use serde_json::{Value, json};

use super::GizmoPrefs;

/// What a drag does, so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Move by a scene vector.
    Move(Vec3),
    /// Turn about the unit scene vector `axis` through the origin, counterclockwise looking
    /// down it toward the origin; `frame` names it as the gizmo's own axis (0 is X).
    Rotate {
        axis: Vec3,
        frame: Option<usize>,
        degrees: f32,
    },
    /// Scale by a factor along each of the gizmo's axes.
    Scale([f32; 3]),
}

impl Op {
    /// As `transform.apply`'s `op`.
    fn arg(self) -> Value {
        match self {
            Op::Move(d) => json!({ "move": d.to_array() }),
            Op::Rotate {
                axis,
                frame,
                degrees,
            } => {
                let axis = frame.map_or(json!(axis.to_array()), |i| json!(["x", "y", "z"][i]));
                json!({ "rotate": { "axis": axis, "degrees": degrees } })
            }
            Op::Scale(s) => json!({ "scale": s }),
        }
    }

    /// `pose` after it: moved, or turned. A scale leaves the gizmo where it is.
    pub fn moved(self, pose: Pose) -> Pose {
        match self {
            Op::Move(d) => Pose {
                origin: pose.origin + d,
                ..pose
            },
            Op::Rotate { axis, degrees, .. } => Pose {
                axes: (Quat::about(axis, degrees.to_radians()) * pose.axes).normalized(),
                ..pose
            },
            Op::Scale(_) => pose,
        }
    }
}

/// A handle the pen holds, from the press until the pen lifts.
#[derive(Clone, Debug)]
pub struct Grab {
    pub id: HandleId,
    /// The part whose gizmo it is.
    pub part: String,
    /// The pose when the press began.
    pub pose: Pose,
    /// The drag transforms the part (a locked gizmo, without Alt); else it only moves the
    /// gizmo.
    pub transforms: bool,
    /// A Ctrl drag that transforms: it moves a copy of the part, whole.
    pub copy: bool,
    /// The view as the press saw it: its camera and its size in pixels.
    camera: Camera,
    size: [f32; 2],
    /// One gizmo unit in scene units.
    unit: f32,
    /// Where the press began, and the last sample a ring measured from, in the view's pixels.
    start: [f32; 2],
    last: [f32; 2],
    /// The angle the pen swept around the centre so far, in radians, counterclockwise on
    /// screen.
    swept: f32,
    /// Which points move how far; a copy moves whole and leaves them out.
    weights: Weights,
    /// What the drag does so far; `None` until the pen moves.
    pub op: Option<Op>,
    /// A `gizmo.drag` update went out: the engine holds the drag open.
    pub sent: bool,
}

/// The gizmo's falloff, and above -100 how far it reaches (the brush's Size at the gizmo),
/// whether the mask holds points back, and the brush's mirror and parallel planes while one is
/// on, as `transform.apply` takes them.
#[derive(Clone, Debug)]
struct Weights {
    falloff: f32,
    reach: Option<Value>,
    mask: bool,
    symmetry: Option<Value>,
}

impl Grab {
    /// A press at `at` (the view's pixels) on handle `id` of `pose`, seen as `handles` through
    /// `camera` in a view `size` pixels wide and high. The drag's falloff is the gizmo's own
    /// (`prefs`): above -100 it reaches as far as the brush's Size, a ring's radius in pixels
    /// taken at the gizmo's origin in scene units (decision 0027, section 5). It repeats across
    /// the brush's mirror and parallel planes; the mask holds points back while the prefs say
    /// so. A `copy` (Ctrl, on a drag that transforms) moves whole, so none of these apply to it.
    pub fn new(
        (id, part, pose): (HandleId, String, Pose),
        (transforms, copy): (bool, bool),
        (camera, size, handles): (&Camera, [f32; 2], &Handles),
        at: [f32; 2],
        (brush, prefs): (&BrushState, GizmoPrefs),
    ) -> Self {
        let falloff = prefs.falloff.clamp(-100.0, 100.0);
        let reach = match (brush.size_px, brush.size) {
            _ if falloff <= -100.0 => None,
            (Some(px), _) => Some(json!(px * handles.unit / handles.unit_px)),
            (None, Some(length)) => Some(json!(length)),
            (None, None) => None,
        };
        let symmetry = symmetry(brush);
        Self {
            id,
            part,
            pose,
            transforms,
            copy: copy && transforms,
            camera: *camera,
            size,
            unit: handles.unit,
            start: at,
            last: at,
            swept: 0.0,
            weights: Weights {
                falloff,
                reach,
                mask: prefs.mask,
                symmetry,
            },
            op: None,
            sent: false,
        }
    }

    /// Where the drag so far puts the gizmo: moved with it, unless the drag transforms the part
    /// under a sticky pose, which stays where it was (decision 0027).
    pub fn drawn(&self) -> Pose {
        let follows = !self.transforms || !self.pose.sticky;
        (self.op.filter(|_| follows)).map_or(self.pose, |op| op.moved(self.pose))
    }

    /// The pen at `at` (the view's pixels): the drag's transform since the press, when it
    /// changed. While `snap` is given (Shift), a turn snaps to that many steps of a full turn
    /// and a move to [`MOVE_SNAP`] along each of the gizmo's axes; a scale does not snap.
    pub fn follow(&mut self, at: [f32; 2], snap: Option<u32>) -> Option<Op> {
        let op = match (self.free(at, snap)?, snap) {
            (Op::Move(d), Some(_)) => Op::Move(snap_move(d, self.pose.axes, MOVE_SNAP)),
            (op, _) => op,
        };
        (self.op != Some(op)).then(|| {
            self.op = Some(op);
            op
        })
    }

    /// The transform the pen at `at` makes, a move not snapped yet.
    fn free(&mut self, at: [f32; 2], snap: Option<u32>) -> Option<Op> {
        let (x, y, z) = self.pose.axes.axes();
        let axes = [x, y, z];
        let op = match (self.id.kind(), self.id.axis()) {
            ("arrow", Some(i)) => {
                let (from, to) = (
                    self.aim(along_ray, axes[i], self.start)?,
                    self.aim(along_ray, axes[i], at)?,
                );
                Op::Move(axes[i] * (to - from))
            }
            ("plane", Some(i)) => Op::Move(
                self.aim(on_plane, axes[i], at)? - self.aim(on_plane, axes[i], self.start)?,
            ),
            ("box", Some(i)) => {
                let from = self.aim(along_ray, axes[i], self.start)?;
                if from.abs() < self.unit * 1e-3 {
                    return None;
                }
                let mut s = [1.0; 3];
                s[i] = (self.aim(along_ray, axes[i], at)? / from).max(0.0);
                Op::Scale(s)
            }
            ("centre", _) => {
                // In the screen plane through the origin: the gizmo keeps its depth in the view.
                let forward = self.camera.axes().2;
                Op::Move(
                    self.aim(on_plane, forward, at)? - self.aim(on_plane, forward, self.start)?,
                )
            }
            ("grip", _) => {
                let c = self
                    .camera
                    .project(self.pose.origin.to_array(), self.size)?;
                let (from, to) = (sub(self.start, c), sub(at, c));
                let from = libm::hypotf(from[0], from[1]);
                if from < 1.0 {
                    return None;
                }
                Op::Scale([libm::hypotf(to[0], to[1]) / from; 3])
            }
            (_, frame) => self.turn(at, frame, &axes, snap)?,
        };
        Some(op)
    }

    /// A ring's turn: the angle swept around the centre on screen, about the ring's axis (the
    /// view ring's is the line of sight), counterclockwise as the viewer sees it.
    fn turn(
        &mut self,
        at: [f32; 2],
        frame: Option<usize>,
        axes: &[Vec3; 3],
        snap: Option<u32>,
    ) -> Option<Op> {
        let o = self.pose.origin;
        let c = self.camera.project(o.to_array(), self.size)?;
        // Too near the centre to say which way the pen went: wait for it to move off.
        if let Some(turned) = sweep(c, self.last, at) {
            self.swept += turned;
            self.last = at;
        }
        let toward = toward_viewer(&self.camera, o);
        let (axis, sign) = match frame {
            Some(i) if axes[i].dot(toward) < 0.0 => (axes[i], -1.0),
            Some(i) => (axes[i], 1.0),
            None => (toward, 1.0),
        };
        let mut degrees = sign * self.swept.to_degrees();
        if let Some(steps) = snap {
            degrees = snap_turn(degrees, steps);
        }
        Some(Op::Rotate {
            axis,
            frame,
            degrees,
        })
    }

    /// The pen's ray at `at` against the unit `direction` through the origin by `aim`: gs-core's
    /// `along_ray` (how far along that axis) or `on_plane` (where it meets the plane across it).
    fn aim<T>(&self, aim: Aim<T>, direction: Vec3, at: [f32; 2]) -> Option<T> {
        aim(&self.camera, self.size, (self.pose.origin, direction), at)
    }

    /// `gizmo.drag`'s update for `op`: the whole transform since the press, snapping while
    /// Shift is held to `snap` steps of a turn and a move to [`MOVE_SNAP`], as `follow` did.
    /// A copy's is the operation alone, marked as a copy.
    pub fn update(&self, op: Op, snap: Option<u32>) -> Value {
        let mut transform = json!({ "part": self.part, "op": op.arg() });
        if !self.copy {
            let w = &self.weights;
            transform["mask"] = json!(w.mask);
            transform["falloff"] = json!(w.falloff);
            if let Some(reach) = &w.reach {
                transform["reach"] = reach.clone();
            }
            if let Some(symmetry) = &w.symmetry {
                transform["symmetry"] = symmetry.clone();
            }
        }
        let mut args = json!({ "phase": "update", "transform": transform });
        if self.copy {
            args["copy"] = json!(true);
        }
        if let Some(steps) = snap {
            args["snap"] = json!({ "steps": steps, "spacing": MOVE_SNAP });
        }
        args
    }
}

/// The brush's Mirror and Parallel planes as `transform.apply` takes them, while one is on: a
/// drag and a deformer repeat across them (decision 0027, section 6). Radial is left out.
pub(super) fn symmetry(brush: &BrushState) -> Option<Value> {
    let (mirror, parallel) = (brush.symmetry.mirror, brush.symmetry.parallel);
    let on = [mirror, parallel].iter().any(|a| a.x || a.y || a.z);
    on.then(|| json!({ "mirror": mirror, "parallel": parallel }))
}

/// One of gs-core's aims of the pen's ray, as [`Grab::aim`] takes it.
type Aim<T> = fn(&Camera, [f32; 2], (Vec3, Vec3), [f32; 2]) -> Option<T>;

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// `gizmo.place` putting part `part`'s gizmo at `pose`'s origin and axes.
pub fn place(part: &str, pose: Pose) -> Value {
    json!({ "part": part, "origin": pose.origin.to_array(), "axes": pose.axes.to_array() })
}
