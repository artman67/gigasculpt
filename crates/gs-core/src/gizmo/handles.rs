//! The gizmo on screen (decision 0027, section 3): one pure function turns a pose, a camera
//! and an image size into every handle's outline in pixels, how faded it is and whether it can
//! be grabbed. The window draws with it, picks with it ([`pick`]), and `gizmo.describe`
//! reports it, so what is drawn is exactly what can be grabbed.
//!
//! The gizmo keeps one size on screen, and an axis handle fades out as it turns toward the
//! viewer, as Blender's does (`editors/transform/transform_gizmo_3d.cc`: the size from the
//! world size of a pixel at the centre; `g_tw_axis_range`, 0.02 to 0.1 for axes and 0.175 to
//! 0.25 for planes).

use super::{Pose, Style};
use crate::camera::{Camera, Projection};
use crate::math::Vec3;

/// A handle, in the order ties are broken in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HandleId {
    MoveX,
    MoveY,
    MoveZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    MoveYz,
    MoveZx,
    MoveXy,
    /// The centre square: a move in the screen plane (decision 0027, section 14).
    MoveView,
    /// The grip: an even scale.
    Scale,
    RotateX,
    RotateY,
    RotateZ,
    RotateView,
}

/// What a handle looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A segment along an axis: `points` is its start and its tip.
    Arrow,
    /// A filled four-sided outline: `points` is its corners in order.
    Square,
    /// An open or closed line: `points` is its samples in order.
    Ring,
}

/// Every handle, with its shape, axis and place: arrows from 0.45 to 1.0 gizmo units along
/// the axis, scale boxes centred at 0.3 (half side 0.06), plane squares from 0.5 to 0.7, the
/// centre square (half side 0.1), the grip 1.7 out on the screen's up-right diagonal (half
/// side 0.08), rings of radius 1.2 and the view ring of radius 1.4.
const ALL: [(HandleId, &str); 15] = [
    (HandleId::MoveX, "move_x"),
    (HandleId::MoveY, "move_y"),
    (HandleId::MoveZ, "move_z"),
    (HandleId::ScaleX, "scale_x"),
    (HandleId::ScaleY, "scale_y"),
    (HandleId::ScaleZ, "scale_z"),
    (HandleId::MoveYz, "move_yz"),
    (HandleId::MoveZx, "move_zx"),
    (HandleId::MoveXy, "move_xy"),
    (HandleId::MoveView, "move_view"),
    (HandleId::Scale, "scale"),
    (HandleId::RotateX, "rotate_x"),
    (HandleId::RotateY, "rotate_y"),
    (HandleId::RotateZ, "rotate_z"),
    (HandleId::RotateView, "rotate_view"),
];

const ARROW: (f32, f32) = (0.45, 1.0);
const BOX_AT: f32 = 0.3;
const BOX_HALF: f32 = 0.06;
const PLANE: (f32, f32) = (0.5, 0.7);
const CENTRE_HALF: f32 = 0.1;
/// The grip's distance from the origin on screen, along the up-right diagonal.
const GRIP_AT: f32 = 1.7;
const GRIP_HALF: f32 = 0.08;
const RING: f32 = 1.2;
const VIEW_RING: f32 = 1.4;
/// Samples around a full ring.
const RING_SAMPLES: usize = 64;
/// An axis handle is hidden at or below the first and fully drawn from the second, by
/// `1 - |cos|` of its axis to the direction looked along.
pub(super) const AXIS_FADE: (f32, f32) = (0.02, 0.1);
/// A plane handle, by `|cos|` of its normal axis to the direction looked along.
const PLANE_FADE: (f32, f32) = (0.175, 0.25);

impl HandleId {
    /// Its name, as `gizmo.describe` reports it (`move_x`, `rotate_view`, ...).
    pub fn name(self) -> &'static str {
        ALL[self as usize].1
    }

    /// The frame axis it moves, scales or turns along (0 = X); for a plane handle, the axis
    /// normal to it. `None` for the centre, the grip and the view ring.
    pub fn axis(self) -> Option<usize> {
        use HandleId as H;
        match self {
            H::MoveX | H::ScaleX | H::MoveYz | H::RotateX => Some(0),
            H::MoveY | H::ScaleY | H::MoveZx | H::RotateY => Some(1),
            H::MoveZ | H::ScaleZ | H::MoveXy | H::RotateZ => Some(2),
            H::MoveView | H::Scale | H::RotateView => None,
        }
    }

    pub fn shape(self) -> Shape {
        match self.kind() {
            "arrow" => Shape::Arrow,
            "ring" | "view_ring" => Shape::Ring,
            _ => Shape::Square,
        }
    }

    /// What it is, as `gizmo.describe` reports it: `arrow`, `box`, `plane`, `centre`, `grip`,
    /// `ring` or `view_ring`.
    pub fn kind(self) -> &'static str {
        use HandleId as H;
        match self {
            H::MoveX | H::MoveY | H::MoveZ => "arrow",
            H::ScaleX | H::ScaleY | H::ScaleZ => "box",
            H::MoveYz | H::MoveZx | H::MoveXy => "plane",
            H::MoveView => "centre",
            H::Scale => "grip",
            H::RotateX | H::RotateY | H::RotateZ => "ring",
            H::RotateView => "view_ring",
        }
    }
}

/// One handle on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Handle {
    pub id: HandleId,
    /// How strongly it is drawn, 0 to 1.
    pub fade: f32,
    /// Whether a press can grab it.
    pub pickable: bool,
    /// Its outline in pixels, origin top-left (see [`Shape`]).
    pub points: Vec<[f32; 2]>,
}

/// Every handle of a pose seen through a camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Handles {
    /// One gizmo unit in world units at the origin: the size in pixels times a pixel's size.
    pub unit: f32,
    /// One gizmo unit in pixels.
    pub unit_px: f32,
    /// The handles that are on screen, in [`HandleId`] order. Empty when the origin is behind
    /// the eye.
    pub handles: Vec<Handle>,
}

/// The handles of `pose` seen through `camera` in an image `size` pixels wide and high.
pub fn handles(pose: &Pose, camera: &Camera, size: [f32; 2], style: &Style) -> Handles {
    let unit_px = style.size_pt * style.px_per_pt;
    let o = pose.origin;
    let unit = unit_px * camera.pixel_size(camera.depth(o), size);
    let mut out = Handles {
        unit,
        unit_px,
        handles: Vec::new(),
    };
    let Some(centre) = camera.project(o.to_array(), size) else {
        return out;
    };
    let view = Seen {
        camera,
        size,
        pose,
        unit,
        centre,
        unit_px,
        dir: looked_along(camera, o),
    };
    out.handles = (ALL.iter())
        .filter_map(|&(id, _)| view.handle(id))
        .collect();
    out
}

/// The direction from the eye toward `p`: along the eye ray in perspective, the view's
/// forward axis in orthographic.
pub(super) fn looked_along(camera: &Camera, p: Vec3) -> Vec3 {
    let forward = camera.axes().2;
    match camera.projection {
        Projection::Perspective => {
            let d = p - Vec3::from_array(camera.eye());
            if d.len2() > 0.0 {
                d.normalized()
            } else {
                forward
            }
        }
        Projection::Orthographic => forward,
    }
}

/// A pose seen through a camera, while its handles are made.
struct Seen<'a> {
    camera: &'a Camera,
    size: [f32; 2],
    pose: &'a Pose,
    unit: f32,
    unit_px: f32,
    centre: [f32; 2],
    dir: Vec3,
}

impl Seen<'_> {
    fn axes(&self) -> [Vec3; 3] {
        let (x, y, z) = self.pose.axes.axes();
        [x, y, z]
    }

    /// The world point at `t` gizmo units along `v` from the origin, on screen.
    fn at(&self, v: Vec3, t: f32) -> Option<[f32; 2]> {
        let p = self.pose.origin + v * (t * self.unit);
        self.camera.project(p.to_array(), self.size)
    }

    fn handle(&self, id: HandleId) -> Option<Handle> {
        let axes = self.axes();
        let (fade, points) = match (id.shape(), id.axis()) {
            (Shape::Arrow, Some(a)) => {
                let points = vec![self.at(axes[a], ARROW.0)?, self.at(axes[a], ARROW.1)?];
                (self.axis_fade(axes[a]), points)
            }
            (Shape::Square, Some(a)) if id <= HandleId::ScaleZ => {
                let c = self.at(axes[a], BOX_AT)?;
                (self.axis_fade(axes[a]), square(c, BOX_HALF * self.unit_px))
            }
            (Shape::Square, Some(a)) => {
                let (u, v) = (axes[(a + 1) % 3], axes[(a + 2) % 3]);
                (self.plane_fade(axes[a]), self.plane(u, v)?)
            }
            (Shape::Square, None) if id == HandleId::MoveView => {
                (1.0, square(self.centre, CENTRE_HALF * self.unit_px))
            }
            (Shape::Square, None) => {
                // Up-right on screen: y grows downward.
                let d = GRIP_AT * self.unit_px * std::f32::consts::FRAC_1_SQRT_2;
                let c = [self.centre[0] + d, self.centre[1] - d];
                (1.0, square(c, GRIP_HALF * self.unit_px))
            }
            (Shape::Ring, Some(a)) => (1.0, self.ring(axes[(a + 1) % 3], axes[(a + 2) % 3])?),
            (Shape::Ring, None) => (1.0, self.view_ring()),
            (Shape::Arrow, None) => return None,
        };
        Some(Handle {
            id,
            fade,
            pickable: fade > 0.0,
            points,
        })
    }

    fn axis_fade(&self, axis: Vec3) -> f32 {
        ramp(1.0 - self.dir.dot(axis).abs(), AXIS_FADE)
    }

    fn plane_fade(&self, normal: Vec3) -> f32 {
        ramp(self.dir.dot(normal).abs(), PLANE_FADE)
    }

    /// The square from `PLANE.0` to `PLANE.1` on the axes `u` and `v`, on screen.
    fn plane(&self, u: Vec3, v: Vec3) -> Option<Vec<[f32; 2]>> {
        let (a, b) = PLANE;
        [(a, a), (b, a), (b, b), (a, b)]
            .iter()
            .map(|&(s, t)| self.at(u * s + v * t, 1.0))
            .collect()
    }

    /// The near half of the ring of radius `RING` in the plane of `u` and `v`: its samples no
    /// farther along the view than the origin, as one run (the whole ring when all are).
    fn ring(&self, u: Vec3, v: Vec3) -> Option<Vec<[f32; 2]>> {
        let depth = self.camera.depth(self.pose.origin);
        let samples: Vec<(Vec3, bool)> = (0..RING_SAMPLES)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / RING_SAMPLES as f32;
                let w = u * libm::cosf(t) + v * libm::sinf(t);
                let p = self.pose.origin + w * (RING * self.unit);
                (w, self.camera.depth(p) <= depth)
            })
            .collect();
        let n = samples.len();
        // Start just after a far sample, so the near ones come as one run.
        let start = (0..n).find(|&i| !samples[i].1).map_or(0, |i| {
            (1..=n)
                .map(|k| (i + k) % n)
                .find(|&j| samples[j].1 && !samples[(j + n - 1) % n].1)
                .unwrap_or(i)
        });
        let all = samples.iter().all(|s| s.1);
        let mut points = Vec::new();
        for k in 0..n + usize::from(all) {
            let (w, near) = samples[(start + k) % n];
            if !near && !all {
                break;
            }
            points.push(self.at(w, RING)?);
        }
        Some(points)
    }

    /// A screen circle of radius `VIEW_RING` gizmo units around the origin, closed.
    fn view_ring(&self) -> Vec<[f32; 2]> {
        let r = VIEW_RING * self.unit_px;
        (0..=RING_SAMPLES)
            .map(|i| {
                let t = std::f32::consts::TAU * (i % RING_SAMPLES) as f32 / RING_SAMPLES as f32;
                [
                    self.centre[0] + r * libm::cosf(t),
                    self.centre[1] + r * libm::sinf(t),
                ]
            })
            .collect()
    }
}

/// 0 at or below `lo`, 1 from `hi`, linear between.
pub(super) fn ramp(x: f32, (lo, hi): (f32, f32)) -> f32 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// The screen square of half side `h` around `c`.
pub(super) fn square(c: [f32; 2], h: f32) -> Vec<[f32; 2]> {
    let [x, y] = c;
    vec![
        [x - h, y - h],
        [x + h, y - h],
        [x + h, y + h],
        [x - h, y + h],
    ]
}

/// The handle a press at `at` grabs: the nearest pickable one within `radius` pixels, the
/// earliest on a tie. Distances are to an arrow's segment, a ring's line, or 0 inside a
/// square.
pub fn pick(handles: &[Handle], at: [f32; 2], radius: f32) -> Option<HandleId> {
    let found = (handles.iter().filter(|h| h.pickable)).map(|h| (h.id, distance(h, at)));
    nearest(found, radius)
}

/// The nearest of `found` within `radius`, the earliest on a tie.
pub(super) fn nearest<T>(found: impl Iterator<Item = (T, f32)>, radius: f32) -> Option<T> {
    let mut best: Option<(f32, T)> = None;
    for (t, d) in found {
        if d <= radius && best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, t));
        }
    }
    best.map(|(_, t)| t)
}

/// How far `at` is from handle `h` on screen, in pixels.
pub fn distance(h: &Handle, at: [f32; 2]) -> f32 {
    outline_distance(h.id.shape(), &h.points, at)
}

/// How far `at` is from an outline `p` of `shape` on screen, in pixels.
pub(super) fn outline_distance(shape: Shape, p: &[[f32; 2]], at: [f32; 2]) -> f32 {
    let line = |n: usize| (0..n).map(|i| to_segment(at, p[i], p[(i + 1) % p.len()]));
    match shape {
        Shape::Arrow => to_segment(at, p[0], p[1]),
        Shape::Square if inside(p, at) => 0.0,
        Shape::Square => line(p.len()).fold(f32::INFINITY, f32::min),
        Shape::Ring if p.len() == 1 => to_segment(at, p[0], p[0]),
        Shape::Ring => line(p.len().saturating_sub(1)).fold(f32::INFINITY, f32::min),
    }
}

/// Whether `at` is inside the convex outline `p`, its edge included.
fn inside(p: &[[f32; 2]], at: [f32; 2]) -> bool {
    let side = |i: usize| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        (b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0])
    };
    let sides: Vec<f32> = (0..p.len()).map(side).collect();
    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
}

/// The distance from `at` to the segment from `a` to `b`.
fn to_segment(at: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a[0] + t * dx - at[0], a[1] + t * dy - at[1]);
    (x * x + y * y).sqrt()
}
