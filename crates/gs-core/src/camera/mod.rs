//! The camera: a turn (a unit quaternion), a target and a distance, a lens, and perspective or
//! orthographic projection; presets, fitting to points, the matrices the renderer draws with and
//! the ray through a point of the image. The moves navigation makes are in [`nav`]. The renderer
//! draws through it and the engine aims screen-space strokes and picks through it, so a pixel
//! means the same thing to all three (decision 0017).
//!
//! World convention: Y up, right-handed, the model's front faces +Z. View space looks down -Z
//! with Y up, and clip space is wgpu's (x and y in -1..1, depth in 0..1) with depth reversed:
//! 1 on the near plane, falling toward 0 with distance (decision 0019, research 14 section 1.4).
//!
//! Every field is a plain `f32`, so a camera written out as numbers and read back is the same
//! camera to the bit, and every operation uses IEEE arithmetic and libm, so the same calls give
//! the same camera on every machine.

mod nav;

pub use nav::{Axis, OrbitMode};

use crate::math::{Aabb, Quat, Vec3};
use crate::stroke::View;

/// Column-major 4x4 matrix, as WGSL `mat4x4<f32>` expects: `m[column][row]`.
pub type M4 = [[f32; 4]; 4];

/// Perspective or orthographic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    /// Parallel rays. The visible height is the one a perspective camera with the same
    /// distance and lens sees at the target, so switching keeps the framing.
    Orthographic,
}

/// The standard views. Front looks at the model's front (+Z side), Right from the +X side,
/// Top from above with the model's back at the top of the image, Iso from front-right-top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Iso,
}

impl Preset {
    pub const ALL: [Preset; 7] = [
        Preset::Front,
        Preset::Back,
        Preset::Left,
        Preset::Right,
        Preset::Top,
        Preset::Bottom,
        Preset::Iso,
    ];

    /// Lower-case name, used as the label on contact sheets.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Front => "front",
            Preset::Back => "back",
            Preset::Left => "left",
            Preset::Right => "right",
            Preset::Top => "top",
            Preset::Bottom => "bottom",
            Preset::Iso => "iso",
        }
    }

    /// Azimuth and elevation in radians, as [`Camera::turned`] takes them.
    pub fn angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        match self {
            Preset::Front => (0.0, 0.0),
            Preset::Back => (PI, 0.0),
            Preset::Left => (-FRAC_PI_2, 0.0),
            Preset::Right => (FRAC_PI_2, 0.0),
            Preset::Top => (0.0, FRAC_PI_2),
            Preset::Bottom => (0.0, -FRAC_PI_2),
            Preset::Iso => (FRAC_PI_4, ISO_ELEVATION),
        }
    }

    /// The view's turn. The six axis views are built from their exact axes; iso from its
    /// angles.
    pub fn orientation(self) -> Quat {
        let axes = |right: [f32; 3], up: [f32; 3], back: [f32; 3]| {
            let v = Vec3::from_array;
            Quat::from_axes(v(right), v(up), v(back))
        };
        match self {
            Preset::Front => Quat::IDENTITY,
            Preset::Back => axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
            Preset::Left => axes([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]),
            Preset::Right => axes([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            Preset::Top => axes([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            Preset::Bottom => axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
            Preset::Iso => {
                let (azimuth, elevation) = self.angles();
                Camera::turned(azimuth, elevation)
            }
        }
    }
}

/// Elevation of the cube diagonal, atan(1 / sqrt 2), written out to avoid a platform atan.
const ISO_ELEVATION: f32 = 0.615_479_7;

/// The cosine of one degree, written out to avoid a platform cos: how far from an axis view
/// [`Camera::side`] still names it. The toggle, a snap and the figure leave a view on it to a
/// rounding; a turn a person can see leaves it.
const SIDE_COS: f32 = 0.999_847_7;

/// The frame a focal length is measured on, in millimetres: 36 wide and 24 high, a full-frame
/// camera's.
pub const FRAME_MM: [f32; 2] = [36.0, 24.0];

/// Which side of the image the lens's frame spans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// The frame's 24 mm span the image's height, whatever its width.
    #[default]
    Vertical,
    /// The frame's 36 mm span the image's width.
    Horizontal,
}

/// A lens: a focal length on [`FRAME_MM`], the side of the image the frame spans, and a shift of
/// the image centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub focal_mm: f32,
    pub fit: Fit,
    /// Where the view axis meets the image, from its centre, in image heights: x to the right,
    /// y up. A shift lens's: the picture slides without the camera turning.
    pub shift: [f32; 2],
}

impl Lens {
    /// 45 degrees from top to bottom on a vertical fit, the field of view the camera had before
    /// it had a lens. `12 / DEFAULT_FOCAL_MM` is `tan(22.5 degrees)` as libm gives it in f32.
    pub const DEFAULT_FOCAL_MM: f32 = 28.970_562;

    /// The lens that sees `fov` radians across the side `fit` spans.
    pub fn from_fov(fov: f32, fit: Fit) -> Lens {
        let half = Lens::half_frame(fit);
        Lens {
            focal_mm: half / libm::tanf(fov * 0.5),
            fit,
            shift: [0.0; 2],
        }
    }

    /// Half the frame's side that `fit` spans, in millimetres.
    fn half_frame(fit: Fit) -> f32 {
        match fit {
            Fit::Vertical => FRAME_MM[1] * 0.5,
            Fit::Horizontal => FRAME_MM[0] * 0.5,
        }
    }

    /// The tangents of half the field of view across and up, for an image `aspect` times as
    /// wide as it is high.
    pub fn tan_half(&self, aspect: f32) -> [f32; 2] {
        let t = Lens::half_frame(self.fit) / self.focal_mm;
        match self.fit {
            Fit::Vertical => [t * aspect, t],
            Fit::Horizontal => [t, t / aspect],
        }
    }

    /// The field of view across the side the frame spans, in radians.
    pub fn fov(&self) -> f32 {
        2.0 * libm::atanf(Lens::half_frame(self.fit) / self.focal_mm)
    }

    /// The shift in normalized device coordinates for an image `aspect` wide per high.
    fn shift_ndc(&self, aspect: f32) -> [f32; 2] {
        [2.0 * self.shift[0] / aspect, 2.0 * self.shift[1]]
    }
}

impl Default for Lens {
    fn default() -> Self {
        Lens {
            focal_mm: Lens::DEFAULT_FOCAL_MM,
            fit: Fit::Vertical,
            shift: [0.0; 2],
        }
    }
}

/// A camera looking at `target` from `distance` away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The view's turn, view space to world: the identity is the front view.
    pub orientation: Quat,
    pub target: [f32; 3],
    pub distance: f32,
    pub lens: Lens,
    pub projection: Projection,
}

/// Fraction of the half-frame the fitted bounds may fill, leaving a 5% border on each side.
const FIT_MARGIN: f32 = 0.9;

impl Camera {
    /// A preset view of the origin from distance 1 through the default lens, in perspective.
    /// Call [`Camera::fit`] to frame a mesh.
    pub fn preset(preset: Preset) -> Camera {
        Camera {
            orientation: preset.orientation(),
            target: [0.0; 3],
            distance: 1.0,
            lens: Lens::default(),
            projection: Projection::Perspective,
        }
    }

    /// A preset view framing `points` for an image of width / height `aspect`: what
    /// [`Camera::fit`] does, starting from [`Camera::preset`].
    pub fn fitted(preset: Preset, points: &[[f32; 3]], aspect: f32) -> Camera {
        let mut camera = Camera::preset(preset);
        camera.fit(points, aspect);
        camera
    }

    /// The turn of a camera `azimuth` radians around the vertical axis from the front (pi/2
    /// looks from +X) and `elevation` radians above the horizon (pi/2 looks straight down).
    pub fn turned(azimuth: f32, elevation: f32) -> Quat {
        let around = Quat::about(Vec3::new(0.0, 1.0, 0.0), azimuth);
        let up = Quat::about(Vec3::new(1.0, 0.0, 0.0), -elevation);
        (around * up).normalized()
    }

    /// Unit vectors (right, up, forward) of the view in the world.
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let (right, up, back) = self.orientation.axes();
        (right, up, -back)
    }

    /// The eye position.
    pub fn eye(&self) -> [f32; 3] {
        let (_, _, forward) = self.axes();
        (Vec3::from_array(self.target) - forward * self.distance).to_array()
    }

    /// The axis view (front, back, left, right, top or bottom) this camera looks along, if its
    /// view direction is within a degree of one, whatever its roll; `None` between them, iso
    /// included. The window's corner and `view.get` both name the view from it, so they agree
    /// (decision 0017 as amended 2026-10-13).
    pub fn side(&self) -> Option<Preset> {
        let back = self.orientation.axes().2;
        let along = |p: &Preset| p.orientation().axes().2.dot(back) >= SIDE_COS;
        Preset::ALL[..6].iter().copied().find(along)
    }

    /// Moves the target and distance so every point is inside the frame with a 5% border,
    /// keeping the turn and the focal length; a lens shift goes back to zero, so the picture is
    /// centred. Points are typically the corners of the chunk bounds the renderer draws;
    /// everything inside their boxes is then in the frame too. A perspective eye also stays a
    /// tenth of the points' radius (half their bounding box diagonal) in front of every point,
    /// so a mesh pointing straight at the camera is not cut.
    pub fn fit(&mut self, points: &[[f32; 3]], aspect: f32) {
        let mut bounds = Aabb::EMPTY;
        for &p in points {
            bounds.grow(Vec3::from_array(p));
        }
        if points.is_empty() {
            return;
        }
        self.lens.shift = [0.0; 2];
        let center = (bounds.lo + bounds.hi) * 0.5;
        let radius = 0.5 * (bounds.hi - bounds.lo).len2().sqrt();
        let floor = radius * 0.1;
        let (right, up, forward) = self.axes();
        let [tx, ty] = self.lens.tan_half(aspect).map(|t| t * FIT_MARGIN);
        let mut need = floor;
        for &p in points {
            let d = Vec3::from_array(p) - center;
            let (x, y, z) = (d.dot(right).abs(), d.dot(up).abs(), -d.dot(forward));
            need = need.max(match self.projection {
                // Distance at which this point sits exactly on the frame edge, in front of the eye.
                Projection::Perspective => z + (x / tx).max(y / ty).max(floor),
                Projection::Orthographic => (x / tx).max(y / ty),
            });
        }
        self.target = center.to_array();
        self.distance = need;
    }

    /// World to view matrix.
    fn view_matrix(&self) -> M4 {
        let (r, u, f) = self.axes();
        let eye = Vec3::from_array(self.eye());
        [
            [r.x, u.x, -f.x, 0.0],
            [r.y, u.y, -f.y, 0.0],
            [r.z, u.z, -f.z, 0.0],
            [-r.dot(eye), -u.dot(eye), f.dot(eye), 1.0],
        ]
    }

    /// Near and far planes around the target and every point, padded by 1% of the larger of
    /// their depth extent and the camera distance (so a flat mesh facing the camera still gets
    /// a depth range). A perspective near plane stays above 1e-5 of the far plane.
    fn depth_range(&self, points: &[[f32; 3]]) -> (f32, f32) {
        let (_, _, forward) = self.axes();
        let eye = Vec3::from_array(self.eye());
        let depths = (points.iter()).map(|&p| (Vec3::from_array(p) - eye).dot(forward));
        let (lo, hi) = depths.fold((self.distance, self.distance), |(lo, hi), d| {
            (lo.min(d), hi.max(d))
        });
        let slack = (hi - lo).max(self.distance) * 0.01;
        let far = hi + slack;
        match self.projection {
            // A camera inside the bounds still needs a positive near plane.
            Projection::Perspective => ((lo - slack).max(far * 1e-5), far),
            Projection::Orthographic => (lo - slack, far),
        }
    }

    /// Reversed depth (decision 0019): perspective depth is `near / distance`, 1 on the near
    /// plane and 0 at infinity, so the far plane does not matter (Bevy's
    /// `perspective_infinite_reverse`); with a float depth buffer this keeps the same relative
    /// precision at every distance. Orthographic depth falls linearly from 1 at `near` to 0 at
    /// `far`.
    fn projection_matrix(&self, aspect: f32, near: f32, far: f32) -> M4 {
        let t = self.lens.tan_half(aspect)[1];
        let [sx, sy] = self.lens.shift_ndc(aspect);
        match self.projection {
            Projection::Perspective => {
                let f = 1.0 / t;
                [
                    [f / aspect, 0.0, 0.0, 0.0],
                    [0.0, f, 0.0, 0.0],
                    // The shift moves the image of the view axis: x_ndc = ... + sx at any depth.
                    [0.0 - sx, 0.0 - sy, 0.0, -1.0],
                    [0.0, 0.0, near, 0.0],
                ]
            }
            Projection::Orthographic => {
                let h = self.distance * t;
                let range = far - near;
                [
                    [1.0 / (h * aspect), 0.0, 0.0, 0.0],
                    [0.0, 1.0 / h, 0.0, 0.0],
                    [0.0, 0.0, 1.0 / range, 0.0],
                    [sx, sy, far / range, 1.0],
                ]
            }
        }
    }

    /// World to view, and view to clip with near and far planes around `depth_points`.
    pub fn matrices(&self, aspect: f32, depth_points: &[[f32; 3]]) -> (M4, M4) {
        let (near, far) = self.depth_range(depth_points);
        (
            self.view_matrix(),
            self.projection_matrix(aspect, near, far),
        )
    }

    /// World to clip, column-major, exactly as the renderer uses it. Depth planes enclose
    /// `depth_points`.
    pub fn view_proj(&self, aspect: f32, depth_points: &[[f32; 3]]) -> M4 {
        let (view, proj) = self.matrices(aspect, depth_points);
        mul(&proj, &view)
    }

    /// The ray through the point (`x`, `y`) of an image `size` pixels wide and high, measured
    /// from its top-left corner (the center of pixel column `c`, row `r` is `c + 0.5`,
    /// `r + 0.5`): its origin and unit direction. A perspective ray starts at the eye; an
    /// orthographic one on the plane through the eye, along the view.
    pub fn ray(&self, x: f32, y: f32, size: [f32; 2]) -> (Vec3, Vec3) {
        let (right, up, forward) = self.axes();
        let aspect = size[0] / size[1];
        let t = self.lens.tan_half(aspect)[1];
        let [sx, sy] = self.lens.shift_ndc(aspect);
        let (nx, ny) = (2.0 * x / size[0] - 1.0 - sx, 1.0 - 2.0 * y / size[1] - sy);
        let across = right * (nx * t * aspect) + up * (ny * t);
        let eye = Vec3::from_array(self.eye());
        match self.projection {
            Projection::Perspective => (eye, (forward + across).normalized()),
            Projection::Orthographic => (eye + across * self.distance, forward),
        }
    }

    /// Where `p` is drawn on an image `size` pixels wide and high, from its top-left corner:
    /// the inverse of [`Camera::ray`]. `None` for a point not in front of a perspective eye.
    pub fn project(&self, p: [f32; 3], size: [f32; 2]) -> Option<[f32; 2]> {
        let (right, up, forward) = self.axes();
        let d = Vec3::from_array(p) - Vec3::from_array(self.eye());
        let (x, y, z) = (d.dot(right), d.dot(up), d.dot(forward));
        let aspect = size[0] / size[1];
        let t = self.lens.tan_half(aspect)[1];
        let [sx, sy] = self.lens.shift_ndc(aspect);
        let scale = match self.projection {
            Projection::Perspective if z > 0.0 => z * t,
            Projection::Perspective => return None,
            Projection::Orthographic => self.distance * t,
        };
        let (nx, ny) = (x / (scale * aspect) + sx, y / scale + sy);
        Some([f32::midpoint(nx, 1.0) * size[0], (1.0 - ny) * 0.5 * size[1]])
    }

    /// World units per pixel at a point `depth` in front of the eye (along the view), in an
    /// image `size` pixels wide and high. Orthographic cameras have one scale at every depth.
    pub fn pixel_size(&self, depth: f32, size: [f32; 2]) -> f32 {
        let at = match self.projection {
            Projection::Perspective => depth,
            Projection::Orthographic => self.distance,
        };
        2.0 * at * self.lens.tan_half(size[0] / size[1])[1] / size[1]
    }

    /// How far `p` is in front of the eye, along the view.
    pub fn depth(&self, p: Vec3) -> f32 {
        (p - Vec3::from_array(self.eye())).dot(self.axes().2)
    }

    /// The camera as a stroke sees it: the view direction, and the eye of a perspective one.
    pub fn view(&self) -> View {
        View {
            dir: self.axes().2,
            eye: (self.projection == Projection::Perspective).then(|| Vec3::from_array(self.eye())),
        }
    }
}

/// `a * b` for column-major matrices.
fn mul(a: &M4, b: &M4) -> M4 {
    let mut out = [[0.0; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = a[0][r] * b[c][0] + a[1][r] * b[c][1] + a[2][r] * b[c][2] + a[3][r] * b[c][3];
        }
    }
    out
}
