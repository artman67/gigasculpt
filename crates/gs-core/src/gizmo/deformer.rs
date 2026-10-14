//! The open deformer's handles (decision 0041, amended 2026-10-13 for `DF7c`, sections 2 to 6):
//! one pure function turns the deformer's frame, its operation and box, and a camera into
//! every handle's outline in pixels and the argument it drives, as [`handles`](super::handles)
//! does for the gizmo. The window draws and picks with it and `gizmo.describe` reports it.
//!
//! A cone sits at the middle of the box's face on the camera's side of its axis and drags one
//! argument by how far the pen goes along it; a twist has a ring about its axis; a cage and a
//! curve have a dot at each of their points. A splitting Mirror or Parallel plane repeats the
//! box, without handles.

use super::handles::{AXIS_FADE, Shape, looked_along, nearest, outline_distance, ramp, square};
use super::{MOVE_SNAP, Style, snap_turn};
use crate::camera::Camera;
use crate::deform::{Curve, Deform, Lattice, place};
use crate::math::{Aabb, Vec3};
use crate::symmetry::Symmetry;
use crate::transform::{Frame, Op, Similarity, Transform, flip};

/// A cone's length and its base's width, in gizmo units.
const CONE: (f32, f32) = (0.3, 0.16);
/// A dot's side, in gizmo units.
const DOT: f32 = 0.1;
/// A face nearer the frame's origin than this share of the box's span gives its cone to the
/// opposite face: a bend, twist or skew barely moves it.
const NEAR: f32 = 0.1;
const RING_SAMPLES: usize = 64;
const XYZ: [&str; 3] = ["x", "y", "z"];
const SCALE: [&str; 3] = ["/scale/0", "/scale/1", "/scale/2"];
const ANY: [f32; 2] = [-f32::MAX, f32::MAX];

/// What a deformer's handles are made from.
#[derive(Clone, Copy, Debug)]
pub struct Deformer<'a> {
    pub op: &'a Op,
    /// The box in frame coordinates: the call's, or for an operation that takes none the
    /// part's box as it is now, [`seen`] in the frame.
    pub bounds: Aabb,
    /// Its Mirror and Parallel planes, through the part's origin.
    pub symmetry: &'a Symmetry,
    /// The part's placement in the scene, which the planes are in.
    pub placed: &'a Similarity,
}

impl<'a> Deformer<'a> {
    /// The deformer `transform` is, on a part whose live points' box in its own space is
    /// `bounds`, placed in the scene by `placed`.
    pub fn of(transform: &'a Transform, bounds: Aabb, placed: &'a Similarity) -> Self {
        let frame = &transform.frame;
        Deformer {
            op: &transform.op,
            bounds: (transform.scope.bounds).unwrap_or_else(|| seen(bounds, placed, frame)),
            symmetry: &transform.symmetry,
            placed,
        }
    }
}

/// What a handle looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A filled triangle: its tip, then its base's two ends.
    Cone,
    /// A closed line about an axis.
    Ring,
    /// A filled square.
    Dot,
}

impl Kind {
    /// As `gizmo.describe` reports it: `cone`, `ring` or `dot`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Cone => "cone",
            Kind::Ring => "ring",
            Kind::Dot => "dot",
        }
    }

    fn shape(self) -> Shape {
        match self {
            Kind::Ring => Shape::Ring,
            Kind::Cone | Kind::Dot => Shape::Square,
        }
    }
}

/// What a number is, for how it snaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Scene units: Shift snaps it to [`MOVE_SNAP`].
    Length,
    /// Degrees: Shift snaps it to the Gizmo's steps of a turn.
    Angle,
    /// A factor: it does not snap.
    Factor,
}

/// The argument a handle drives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    /// The number at `at`, a JSON pointer into the call's `op`: `start` plus `rate` for each
    /// scene unit the pen goes along a cone (for the ring, each degree it sweeps about its
    /// axis), kept within `range`.
    Number {
        at: &'static str,
        start: f32,
        rate: f32,
        range: [f32; 2],
        unit: Unit,
    },
    /// The move, `start` plus the distance along the cone.
    Move { start: Vec3 },
    /// The cage point at index `at`, offset `by` from where it rests, in scene units along the
    /// frame's axes.
    Cage { at: [u32; 3], by: [f32; 3] },
    /// The curve's point `at`, in frame coordinates (an end of the axis line while the curve
    /// has no points).
    Curve { at: usize, place: [f32; 3] },
}

impl Drive {
    /// The JSON pointer into the call's `op` it writes: a dot writes its whole list.
    pub fn at(&self) -> &'static str {
        match self {
            Drive::Number { at, .. } => at,
            Drive::Move { .. } => "/move",
            Drive::Cage { .. } => "/lattice/offsets",
            Drive::Curve { .. } => "/curve/points",
        }
    }
}

/// A handle's argument after a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Moved {
    Number(f32),
    Vector(Vec3),
}

/// One handle on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct DeformerHandle {
    pub kind: Kind,
    pub drive: Drive,
    /// The frame axis of the face a cone sits on, or the ring's axis; `None` for a dot.
    pub axis: Option<usize>,
    /// How strongly it is drawn, 0 to 1: a cone fades as the gizmo's arrows do when looked
    /// along.
    pub fade: f32,
    /// Whether a press can grab it.
    pub pickable: bool,
    /// Its outline in pixels, origin top-left (see [`Kind`]).
    pub points: Vec<[f32; 2]>,
    /// In the scene: a cone's base (the face's middle), the ring's centre, the dot.
    pub at: Vec3,
    /// In the scene: the unit direction a cone points, the ring's axis; zero for a dot.
    pub along: Vec3,
}

impl DeformerHandle {
    /// As `gizmo.describe` reports it: `cone_x`, `ring_z`, `dot_1_2_0`, `point_0`.
    pub fn name(&self) -> String {
        match self.drive {
            Drive::Cage { at: [i, j, k], .. } => format!("dot_{i}_{j}_{k}"),
            Drive::Curve { at, .. } => format!("point_{at}"),
            _ => format!("{}_{}", self.kind.name(), XYZ[self.axis.unwrap_or(0)]),
        }
    }

    /// Its argument after a drag of `by`: scene units along a cone, or degrees swept about the
    /// ring's axis, counterclockwise looking down it. With `snap` (Shift) a length snaps to
    /// [`MOVE_SNAP`] and an angle to `snap` steps of a turn; a factor does not snap. `None` for
    /// a dot, or a value that is not finite.
    pub fn moved(&self, by: f32, snap: Option<u32>) -> Option<Moved> {
        let snapped = |v: f32, unit| match (snap, unit) {
            (Some(_), Unit::Length) => (v / MOVE_SNAP).round() * MOVE_SNAP,
            (Some(steps), Unit::Angle) => snap_turn(v, steps),
            _ => v,
        };
        let moved = match self.drive {
            Drive::Number {
                start,
                rate,
                range,
                unit,
                ..
            } => Moved::Number(snapped(start + rate * by, unit).clamp(range[0], range[1])),
            Drive::Move { start } => {
                let was = start.dot(self.along);
                let now = snapped(was + by, Unit::Length);
                Moved::Vector(start + self.along * (now - was))
            }
            Drive::Cage { .. } | Drive::Curve { .. } => return None,
        };
        let finite = match moved {
            Moved::Number(v) => v.is_finite(),
            Moved::Vector(v) => v.to_array().iter().all(|c| c.is_finite()),
        };
        finite.then_some(moved)
    }
}

/// A line drawn with the handles: a box's edge, a row of the cage, the curve.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its points in pixels, in order.
    pub points: Vec<[f32; 2]>,
    /// A repeat's, drawn at half strength.
    pub repeat: bool,
}

/// Every handle of an open deformer seen through a camera.
#[derive(Clone, Debug, PartialEq)]
pub struct DeformerHandles {
    /// One gizmo unit in scene units at the frame's origin.
    pub unit: f32,
    /// One gizmo unit in pixels.
    pub unit_px: f32,
    /// The box's eight corners in the scene, then each repeat's: corner `k` is at the box's
    /// high end on axis `a` when bit `a` of `k` is set.
    pub boxes: Vec<[Vec3; 8]>,
    /// The boxes' edges, then the cage's rows or the curve; a line with a point the camera
    /// cannot see is left out.
    pub lines: Vec<Line>,
    /// The handles in the order a tie is broken in; one with a point the camera cannot see is
    /// left out.
    pub handles: Vec<DeformerHandle>,
}

/// The handles of `deformer` about `frame` (the open's frame, pinned at the pick), seen
/// through `camera` in an image `size` pixels wide and high, sized by `style`.
pub fn deformer_handles(
    frame: &Frame,
    deformer: &Deformer,
    (camera, size, style): (&Camera, [f32; 2], &Style),
) -> DeformerHandles {
    let unit_px = style.size_pt * style.px_per_pt;
    let unit = unit_px * camera.pixel_size(camera.depth(frame.origin), size);
    let (x, y, z) = frame.axes.axes();
    let (lo, hi) = (deformer.bounds.lo.to_array(), deformer.bounds.hi.to_array());
    let mut view = View {
        camera,
        size,
        unit,
        unit_px,
        origin: frame.origin,
        axes: [x, y, z],
        lo,
        hi,
        toward: Vec3::ZERO,
        side: [1.0; 3],
    };
    let middle = std::array::from_fn(|a| f32::midpoint(lo[a], hi[a]));
    view.toward = -looked_along(camera, view.scene(middle));
    view.side = view
        .axes
        .map(|e| if e.dot(view.toward) < 0.0 { -1.0 } else { 1.0 });
    let boxes = repeats(frame, deformer);
    let mut lines: Vec<Line> = (boxes.iter().enumerate())
        .flat_map(|(k, corners)| view.edges(corners, k > 0))
        .collect();
    let handles = view.handles(deformer.op, &mut lines);
    DeformerHandles {
        unit,
        unit_px,
        boxes,
        lines,
        handles,
    }
}

/// The handle a press at `at` grabs, by its place in `handles`: the nearest pickable one within
/// `radius` pixels, the earliest on a tie, as [`pick`](super::pick) does for the gizmo.
pub fn pick_deformer(handles: &[DeformerHandle], at: [f32; 2], radius: f32) -> Option<usize> {
    let found = (handles.iter().enumerate())
        .filter(|(_, h)| h.pickable)
        .map(|(i, h)| (i, outline_distance(h.kind.shape(), &h.points, at)));
    nearest(found, radius)
}

/// The box `bounds`, in a part's own space placed in the scene by `placed`, as seen in
/// `frame`: the box around its eight corners, in frame coordinates.
pub fn seen(bounds: Aabb, placed: &Similarity, frame: &Frame) -> Aabb {
    let back = frame.axes.inverse();
    let (lo, hi) = (bounds.lo.to_array(), bounds.hi.to_array());
    let mut out = Aabb::EMPTY;
    for k in 0..8 {
        let p = placed.point(Vec3::from_array(corner(lo, hi, k)));
        out.grow(back.rotate(p - frame.origin));
    }
    out
}

/// Corner `k` of the box from `lo` to `hi`: at `hi` on axis `a` when bit `a` of `k` is set.
fn corner(lo: [f32; 3], hi: [f32; 3], k: usize) -> [f32; 3] {
    std::array::from_fn(|a| if k >> a & 1 == 1 { hi[a] } else { lo[a] })
}

/// The box in the scene, then its repeat in each region a splitting Mirror or Parallel plane
/// carries the deformer to (decision 0027, section 6): mirrored across a Mirror plane, moved
/// across a Parallel one, as `transform.apply` carries the points, in the part's own space.
fn repeats(frame: &Frame, d: &Deformer) -> Vec<[Vec3; 8]> {
    let (lo, hi) = (d.bounds.lo.to_array(), d.bounds.hi.to_array());
    let main: [Vec3; 8] = std::array::from_fn(|k| {
        frame.origin + frame.axes.rotate(Vec3::from_array(corner(lo, hi, k)))
    });
    let (s, placed) = (d.symmetry, d.placed);
    let o = placed.part_point(frame.origin);
    let clip = placed.part_length(s.clip);
    let plane = |a: usize, on: [bool; 3]| u32::from(on[a]) << a;
    let splits = |a: usize| (s.mirror[a] || s.parallel[a]) && o.to_array()[a].abs() > clip;
    let split: u32 = (0..3).filter(|&a| splits(a)).map(|a| 1 << a).sum();
    let mirrored = (0..3).map(|a| plane(a, s.mirror)).sum::<u32>() & split;
    let mut boxes = vec![main];
    for m in (1..8u32).filter(|m| m & !split == 0) {
        let r = m & mirrored;
        let carried = |c: Vec3| {
            let c = placed.part_point(c);
            placed.point(flip(c - o + flip(o, m & !r), r))
        };
        boxes.push(main.map(carried));
    }
    boxes
}

/// The frame as the camera sees it, while the handles are made.
struct View<'a> {
    camera: &'a Camera,
    size: [f32; 2],
    unit: f32,
    unit_px: f32,
    origin: Vec3,
    axes: [Vec3; 3],
    /// The box, in frame coordinates.
    lo: [f32; 3],
    hi: [f32; 3],
    /// From the box's middle toward the viewer.
    toward: Vec3,
    /// For each axis, 1 when its high face is toward the viewer, else -1.
    side: [f32; 3],
}

impl View<'_> {
    /// Frame coordinates `c` in the scene.
    fn scene(&self, c: [f32; 3]) -> Vec3 {
        let [x, y, z] = self.axes;
        self.origin + x * c[0] + y * c[1] + z * c[2]
    }

    fn project(&self, p: Vec3) -> Option<[f32; 2]> {
        self.camera.project(p.to_array(), self.size)
    }

    fn middle(&self) -> [f32; 3] {
        std::array::from_fn(|a| f32::midpoint(self.lo[a], self.hi[a]))
    }

    fn span(&self, a: usize) -> f32 {
        self.hi[a] - self.lo[a]
    }

    /// Where `s` lies along `a` from the origin, as a share of the box's span, as the
    /// deformers measure it ([`place`]).
    fn place(&self, a: usize, s: f32) -> f32 {
        let [lo, hi, s] = [self.lo[a], self.hi[a], s].map(f64::from);
        place(lo, hi, s) as f32
    }

    /// The middle of axis `a`'s face toward the viewer, in frame coordinates, and which face
    /// it is (1 high, -1 low); the opposite face when that one is nearly at the origin.
    fn face(&self, a: usize) -> (f32, [f32; 3]) {
        let at = |side: f32| {
            let mut c = self.middle();
            c[a] = if side > 0.0 { self.hi[a] } else { self.lo[a] };
            c
        };
        let side = self.side[a];
        if at(side)[a].abs() < NEAR * self.span(a) {
            (-side, at(-side))
        } else {
            (side, at(side))
        }
    }

    /// The box's twelve edges on screen.
    fn edges(&self, corners: &[Vec3; 8], repeat: bool) -> Vec<Line> {
        let pairs = (0..8).flat_map(|k| (0..3).map(move |a| (k, a)));
        (pairs.filter(|&(k, a)| k >> a & 1 == 0))
            .filter_map(|(k, a)| self.line(&[corners[k], corners[k | 1 << a]], repeat))
            .collect()
    }

    fn line(&self, points: &[Vec3], repeat: bool) -> Option<Line> {
        let points = points
            .iter()
            .map(|&p| self.project(p))
            .collect::<Option<_>>()?;
        Some(Line { points, repeat })
    }

    /// A cone on axis `a`'s face at `c` (frame coordinates), pointing along the unit `along`.
    fn cone(
        &self,
        a: usize,
        c: [f32; 3],
        along: Vec3,
        drive: Option<Drive>,
    ) -> Option<DeformerHandle> {
        let drive = drive?;
        let at = self.scene(c);
        let (base, tip) = (
            self.project(at)?,
            self.project(at + along * (CONE.0 * self.unit))?,
        );
        let (dx, dy) = (tip[0] - base[0], tip[1] - base[1]);
        let length = libm::hypotf(dx, dy);
        let n = if length > 0.0 {
            [-dy / length, dx / length]
        } else {
            [1.0, 0.0]
        };
        let h = CONE.1 / 2.0 * self.unit_px;
        let points = vec![
            tip,
            [base[0] + n[0] * h, base[1] + n[1] * h],
            [base[0] - n[0] * h, base[1] - n[1] * h],
        ];
        let fade = ramp(
            1.0 - looked_along(self.camera, at).dot(along).abs(),
            AXIS_FADE,
        );
        Some(DeformerHandle {
            kind: Kind::Cone,
            drive,
            axis: Some(a),
            fade,
            pickable: fade > 0.0,
            points,
            at,
            along,
        })
    }

    /// A cone on each axis's face toward the viewer, pointing out, driving `drive(a, f)`, `f`
    /// the face's distance from the origin.
    fn outward(&self, drive: impl Fn(usize, f32) -> Option<Drive>) -> Vec<DeformerHandle> {
        (0..3)
            .filter_map(|a| {
                let (side, c) = self.face(a);
                self.cone(a, c, self.axes[a] * side, drive(a, c[a].abs()))
            })
            .collect()
    }

    fn dot(&self, c: [f32; 3], drive: Drive) -> Option<DeformerHandle> {
        let at = self.scene(c);
        Some(DeformerHandle {
            kind: Kind::Dot,
            drive,
            axis: None,
            fade: 1.0,
            pickable: true,
            points: square(self.project(at)?, DOT / 2.0 * self.unit_px),
            at,
            along: Vec3::ZERO,
        })
    }

    fn handles(&self, op: &Op, lines: &mut Vec<Line>) -> Vec<DeformerHandle> {
        match *op {
            Op::Move(start) => self.outward(|_, _| Some(Drive::Move { start })),
            Op::Scale(s) => {
                self.outward(|a, f| number(SCALE[a], s[a], s[a] / f, [0.0, f32::MAX], Unit::Factor))
            }
            Op::Inflate(by) => self.outward(|_, _| number("/inflate", by, 1.0, ANY, Unit::Length)),
            Op::Rotate { axis, angle } => self.rotate(axis, angle).into_iter().collect(),
            Op::Deform(ref d) => self.deform(d, lines),
        }
    }

    /// A turn about one of the frame's axes: a cone on the face of the two others that faces
    /// the viewer most, pointing along the turn. None about another axis.
    fn rotate(&self, axis: Vec3, angle: f32) -> Option<DeformerHandle> {
        let a = (0..3).find(|&a| self.axes[a].dot(axis).abs() > 1.0 - 1e-6)?;
        let facing = |b: usize| self.axes[b].dot(self.toward).abs();
        let (p, q) = ((a + 1) % 3, (a + 2) % 3);
        let b = if facing(q) > facing(p) { q } else { p };
        let (_, c) = self.face(b);
        let arm = axis.cross(self.scene(c) - self.origin);
        let r = arm.len2().sqrt();
        let drive = number(
            "/rotate/degrees",
            angle.to_degrees(),
            (1.0 / r).to_degrees(),
            ANY,
            Unit::Angle,
        );
        self.cone(b, c, arm * (1.0 / r), drive)
    }

    fn deform(&self, d: &Deform, lines: &mut Vec<Line>) -> Vec<DeformerHandle> {
        let deg = f32::to_degrees;
        let cone = match *d {
            Deform::Bend {
                axis,
                toward,
                angle,
            } => {
                let (_, c) = self.face(axis);
                let l = c[axis] - 0.0f32.clamp(self.lo[axis], self.hi[axis]);
                let rate = deg(2.0 * self.span(axis) / (l * l));
                let drive = number(
                    "/bend/degrees",
                    deg(angle),
                    rate,
                    [-3600.0, 3600.0],
                    Unit::Angle,
                );
                self.cone(axis, c, self.axes[toward], drive)
            }
            Deform::Fold {
                axis,
                toward,
                angle,
            } => {
                let (_, c) = self.face(axis);
                let rate = deg(2.0 / c[axis].abs());
                let drive = number(
                    "/fold/degrees",
                    deg(angle),
                    rate,
                    [-360.0, 360.0],
                    Unit::Angle,
                );
                self.cone(axis, c, self.axes[toward], drive)
            }
            Deform::Twist { axis, angle } => self.ring(axis, angle),
            Deform::Taper {
                axis,
                amount,
                exponent,
            } => self.taper(axis, amount, exponent),
            Deform::Shear {
                axis,
                toward,
                amount,
                smooth,
            } => self.shear([axis, toward], amount, smooth),
            Deform::Clip { axis, depth } => {
                let (side, c) = self.face(axis);
                let range = if side > 0.0 {
                    [0.0, self.span(axis)]
                } else {
                    [-self.span(axis), 0.0]
                };
                let drive = number("/clip/depth", depth, side, range, Unit::Length);
                self.cone(axis, c, self.axes[axis] * -side, drive)
            }
            Deform::Spherize { amount, .. } => {
                return self.outward(|_, f| {
                    number(
                        "/spherize/amount",
                        amount,
                        1.0 / f,
                        [-1.0, 1.0],
                        Unit::Factor,
                    )
                });
            }
            Deform::Noise { amount, .. } => {
                return self
                    .outward(|_, _| number("/noise/amount", amount, 1.0, ANY, Unit::Length));
            }
            Deform::Lattice(ref l) => return self.cage(l, lines),
            Deform::Curve(ref c) => return self.curve(c, lines),
        };
        cone.into_iter().collect()
    }

    /// A ring about `axis` where it meets the face toward the viewer, through the box's
    /// farthest corner from it; the face turns with the pen.
    fn ring(&self, axis: usize, angle: f32) -> Option<DeformerHandle> {
        let (_, c) = self.face(axis);
        let mut centre = [0.0; 3];
        centre[axis] = c[axis];
        let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
        let far = |i: usize| self.lo[i].abs().max(self.hi[i].abs());
        let r = libm::hypotf(far(p), far(q));
        let rate = 1.0 / self.place(axis, c[axis]);
        let drive = number(
            "/twist/degrees",
            angle.to_degrees(),
            rate,
            [-3600.0, 3600.0],
            Unit::Angle,
        )?;
        let at = self.scene(centre);
        let points = (0..=RING_SAMPLES)
            .map(|i| {
                let t = std::f32::consts::TAU * (i % RING_SAMPLES) as f32 / RING_SAMPLES as f32;
                let w = self.axes[p] * libm::cosf(t) + self.axes[q] * libm::sinf(t);
                self.project(at + w * r)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(DeformerHandle {
            kind: Kind::Ring,
            drive,
            axis: Some(axis),
            fade: 1.0,
            pickable: true,
            points,
            at,
            along: self.axes[axis],
        })
    }

    /// A cone on the high face at its edge on the cross axis most across the view, pointing
    /// in: the edge follows the pen.
    fn taper(&self, axis: usize, amount: f32, exponent: f32) -> Option<DeformerHandle> {
        let across = |k: usize| self.axes[k].dot(self.toward).abs();
        let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
        let k = if across(q) < across(p) { q } else { p };
        let mut c = self.middle();
        c[axis] = self.hi[axis];
        c[k] = if self.side[k] > 0.0 {
            self.hi[k]
        } else {
            self.lo[k]
        };
        let u = self.place(axis, self.hi[axis]);
        let g = libm::powf(u.abs(), exponent).copysign(u);
        let drive = number(
            "/taper/amount",
            amount,
            1.0 / (c[k].abs() * g),
            [-1.0, 1.0],
            Unit::Factor,
        );
        self.cone(axis, c, self.axes[k] * -c[k].signum(), drive)
    }

    /// A cone on the face toward the viewer, pointing the way a positive amount slides it.
    fn shear(
        &self,
        [axis, toward]: [usize; 2],
        amount: f32,
        smooth: bool,
    ) -> Option<DeformerHandle> {
        let (_, c) = self.face(axis);
        let u = self.place(axis, c[axis]);
        let g = if smooth {
            (u * u * (3.0 - 2.0 * u.abs())).copysign(u)
        } else {
            u
        };
        let drive = number("/shear/amount", amount, 1.0 / g.abs(), ANY, Unit::Length);
        self.cone(axis, c, self.axes[toward] * g.signum(), drive)
    }

    /// A dot at each cage point where it rests plus its offset, and a line through each row.
    fn cage(&self, l: &Lattice, lines: &mut Vec<Line>) -> Vec<DeformerHandle> {
        let n = l.size;
        if n.iter().any(|&k| k < 2) {
            return Vec::new();
        }
        let by = |i: [u32; 3]| (l.offsets.iter().find(|o| o.at == i)).map_or([0.0; 3], |o| o.by);
        let point = |i: [u32; 3]| -> [f32; 3] {
            let b = by(i);
            std::array::from_fn(|a| {
                self.lo[a] + self.span(a) * i[a] as f32 / (n[a] - 1) as f32 + b[a]
            })
        };
        let index = |m: u32| [m / (n[1] * n[2]), m / n[2] % n[1], m % n[2]];
        let all = (0..n[0] * n[1] * n[2]).map(index);
        for a in 0..3 {
            let rows = all.clone().filter(|i| i[a] == 0);
            for start in rows {
                let row: Vec<Vec3> = (0..n[a])
                    .map(|t| {
                        let mut i = start;
                        i[a] = t;
                        self.scene(point(i))
                    })
                    .collect();
                lines.extend(self.line(&row, false));
            }
        }
        all.filter_map(|i| self.dot(point(i), Drive::Cage { at: i, by: by(i) }))
            .collect()
    }

    /// A dot at each curve point, or at the ends of the axis line while it has none, and the
    /// line through them in order.
    fn curve(&self, c: &Curve, lines: &mut Vec<Line>) -> Vec<DeformerHandle> {
        let ends = || {
            [self.lo[c.axis], self.hi[c.axis]].map(|s| {
                let mut p = [0.0; 3];
                p[c.axis] = s;
                p
            })
        };
        let points = c.points.clone().unwrap_or_else(|| ends().to_vec());
        let through: Vec<Vec3> = points.iter().map(|&p| self.scene(p)).collect();
        lines.extend(self.line(&through, false));
        (points.iter().enumerate())
            .filter_map(|(at, &place)| self.dot(place, Drive::Curve { at, place }))
            .collect()
    }
}

/// A number's drive, `None` where its rate is not finite (a face at the origin).
fn number(at: &'static str, start: f32, rate: f32, range: [f32; 2], unit: Unit) -> Option<Drive> {
    rate.is_finite().then_some(Drive::Number {
        at,
        start,
        rate,
        range,
        unit,
    })
}
