//! Where the pen meets what a held handle moves along (decision 0027, section 4): the point of
//! a line nearest the pen's ray, where the ray meets a plane, and the angle the pen sweeps
//! around a point on screen. The gizmo's drags and the deformers' handles measure alike.

use super::handles::looked_along;
use crate::camera::Camera;
use crate::math::Vec3;

/// How far along the unit `axis` from `origin`, in scene units, the point nearest the pen's
/// ray at `at` is (the view's pixels, in a view `size` pixels wide and high); `None` while the
/// axis runs along the ray.
pub fn along_ray(
    camera: &Camera,
    size: [f32; 2],
    (origin, axis): (Vec3, Vec3),
    at: [f32; 2],
) -> Option<f32> {
    let (e, d) = camera.ray(at[0], at[1], size);
    let w = origin - e;
    let b = axis.dot(d);
    let denom = 1.0 - b * b;
    (denom > 1e-6).then(|| (b * d.dot(w) - axis.dot(w)) / denom)
}

/// Where the pen's ray at `at` meets the plane through `origin` across the unit `normal`;
/// `None` while the plane is seen edge on.
pub fn on_plane(
    camera: &Camera,
    size: [f32; 2],
    (origin, normal): (Vec3, Vec3),
    at: [f32; 2],
) -> Option<Vec3> {
    let (e, d) = camera.ray(at[0], at[1], size);
    let facing = normal.dot(d);
    (facing.abs() > 1e-6).then(|| e + d * (normal.dot(origin - e) / facing))
}

/// The angle in radians the pen swept from `last` to `at` around `centre` on screen,
/// counterclockwise as the viewer sees it; `None` while either is within a pixel of the
/// centre, too near to say which way it went.
pub fn sweep(centre: [f32; 2], last: [f32; 2], at: [f32; 2]) -> Option<f32> {
    let (a, b) = (
        [last[0] - centre[0], last[1] - centre[1]],
        [at[0] - centre[0], at[1] - centre[1]],
    );
    let off = libm::hypotf(a[0], a[1]) >= 1.0 && libm::hypotf(b[0], b[1]) >= 1.0;
    off.then(|| {
        // Screen y runs down, so counterclockwise on screen is a negative cross product.
        let cross = a[0] * b[1] - a[1] * b[0];
        libm::atan2f(-cross, a[0] * b[0] + a[1] * b[1])
    })
}

/// The unit direction from `p` toward the viewer: toward the eye in perspective, against the
/// view in orthographic. A turn about an axis pointing away from the viewer runs clockwise on
/// screen.
pub fn toward_viewer(camera: &Camera, p: Vec3) -> Vec3 {
    -looked_along(camera, p)
}
