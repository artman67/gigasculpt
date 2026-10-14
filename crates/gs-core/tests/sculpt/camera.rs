//! The camera's rays agree with its projection: the ray through the pixel where a point is
//! drawn passes through that point, pixels have the size the projection gives them at each
//! depth, and a fitted camera draws every point inside the frame; for any turn (rolled ones
//! too), lens, fit and shift, in both projections.

use gs_core::camera::{Camera, Fit, Lens, M4, Preset, Projection};
use gs_core::{Quat, Vec3};
use gs_testkit::SplitMix64;
use gs_testkit::measure::{cross, dot, norm, sub};
use gs_testkit::rng::signed_unit;

use crate::{point, wide};

const SIZE: [f32; 2] = [640.0, 400.0];

/// Where `m` (world to clip) draws `p` in an image of `SIZE`, from its top-left corner, in
/// `f64`; `None` behind the eye.
fn drawn(m: &M4, p: Vec3) -> Option<[f64; 2]> {
    let q = [p.x, p.y, p.z, 1.0].map(f64::from);
    let mut clip = [0.0f64; 4];
    for (column, weight) in m.iter().zip(q) {
        for (out, entry) in clip.iter_mut().zip(column) {
            *out += f64::from(*entry) * weight;
        }
    }
    let (w, h) = (f64::from(SIZE[0]), f64::from(SIZE[1]));
    (clip[3] > 0.0).then(|| {
        [
            f64::midpoint(clip[0] / clip[3], 1.0) * w,
            (1.0 - clip[1] / clip[3]) * 0.5 * h,
        ]
    })
}

/// A camera at a random turn (any axis, so rolled too), distance, target and lens.
pub fn random_camera(rng: &mut SplitMix64, projection: Projection) -> Camera {
    let mut u = || signed_unit(rng.next_u64());
    let axis = Vec3::new(u(), u(), u()).normalized();
    let turn = Quat::about(axis, 3.1 * u());
    let lens = Lens {
        focal_mm: 30.0 + 25.0 * (u() + 1.0),
        fit: if u() > 0.0 {
            Fit::Vertical
        } else {
            Fit::Horizontal
        },
        shift: [0.2 * u(), 0.2 * u()],
    };
    Camera {
        orientation: turn,
        target: [u(), u(), u()],
        distance: 3.0 + 2.0 * u(),
        lens,
        projection,
    }
}

#[test]
fn the_ray_through_a_drawn_point_passes_through_it() {
    let mut rng = SplitMix64::new(11);
    let mut checked = 0;
    for k in 0..200 {
        let projection = [Projection::Perspective, Projection::Orthographic][k % 2];
        let camera = random_camera(&mut rng, projection);
        let m = camera.view_proj(SIZE[0] / SIZE[1], &[]);
        for _ in 0..10 {
            let p = Vec3::from_array(camera.target) + point(&mut rng, 1.0);
            let Some([x, y]) = drawn(&m, p) else {
                continue;
            };
            let projected = camera.project(p.to_array(), SIZE).map(|q| q.map(f64::from));
            let off = projected.map_or(f64::INFINITY, |[px, py]| libm::hypot(px - x, py - y));
            let inside = (0.0..640.0).contains(&x) && (0.0..400.0).contains(&y);
            assert!(
                !inside || off < 2e-3,
                "{camera:?}: project draws {p:?} {off} px from the matrix"
            );
            let (o, d) = camera.ray(x as f32, y as f32, SIZE);
            assert!(
                (d.len2() - 1.0).abs() < 1e-5,
                "the direction is a unit vector"
            );
            let s = sub(wide(p), wide(o));
            let off = norm(cross(s, wide(d)));
            assert!(
                off < 2e-5 * (1.0 + norm(s)),
                "{camera:?}: the ray through ({x}, {y}) passes {off} from the point drawn there"
            );
            assert!(dot(s, wide(d)) > 0.0, "the point lies ahead of the ray");
            checked += 1;
        }
    }
    assert!(
        checked > 1000,
        "only {checked} points were in front of the eye"
    );
}

#[test]
fn pixels_have_the_size_the_projection_gives_them() {
    let mut rng = SplitMix64::new(12);
    for k in 0..100 {
        let projection = [Projection::Perspective, Projection::Orthographic][k % 2];
        let camera = random_camera(&mut rng, projection);
        let view = camera.view();
        let depth = camera.distance * (0.5 + signed_unit(rng.next_u64()).abs());
        // The point at `depth` on the ray through (x, y).
        let at = |x: f32, y: f32| {
            let (o, d) = camera.ray(x, y, SIZE);
            o + d * ((depth - camera.depth(o)) / d.dot(view.dir))
        };
        let (x, y) = (100.0 + 400.0 * signed_unit(rng.next_u64()).abs(), 200.0);
        let (a, b) = (at(x, y), at(x + 10.0, y));
        assert!(
            (camera.depth(a) - depth).abs() < 1e-4 * depth,
            "depth along the view"
        );
        let gap = (b - a).len2().sqrt();
        let want = 10.0 * camera.pixel_size(depth, SIZE);
        assert!(
            (gap - want).abs() < 1e-4 * want,
            "{camera:?}: ten pixels at depth {depth} span {gap}, not {want}"
        );
        let eye = view.eye.map(Vec3::to_array);
        let expected = (projection == Projection::Perspective).then(|| camera.eye());
        assert_eq!(
            eye, expected,
            "a stroke sees the eye of a perspective camera only"
        );
    }
}

#[test]
fn a_fitted_camera_draws_every_point_inside_the_frame() {
    let mut rng = SplitMix64::new(13);
    for k in 0..60 {
        let projection = [Projection::Perspective, Projection::Orthographic][k % 2];
        let center = point(&mut rng, 5.0);
        let points: Vec<[f32; 3]> = (0..20)
            .map(|_| (center + point(&mut rng, 1.0)).to_array())
            .collect();
        let mut camera = random_camera(&mut rng, projection);
        let aspect = SIZE[0] / SIZE[1];
        camera.fit(&points, aspect);
        let m = camera.view_proj(aspect, &points);
        let mut widest = 0.0f64;
        for &p in &points {
            let Some([x, y]) = drawn(&m, Vec3::from_array(p)) else {
                panic!("{camera:?}: point {p:?} is behind the eye");
            };
            let ndc = [
                2.0 * x / f64::from(SIZE[0]) - 1.0,
                1.0 - 2.0 * y / f64::from(SIZE[1]),
            ];
            let reach = ndc[0].abs().max(ndc[1].abs());
            assert!(
                reach <= 0.9 + 1e-4,
                "{camera:?}: {p:?} falls outside the frame"
            );
            widest = widest.max(reach);
        }
        assert!(
            widest > 0.9 - 1e-3,
            "{camera:?}: the fit leaves slack ({widest})"
        );
    }
}

/// Clip-space z over w of `p`: reversed depth, 1 on the near plane, toward 0 with distance.
fn depth_of(m: &M4, p: Vec3) -> f64 {
    let q = [p.x, p.y, p.z, 1.0].map(f64::from);
    let (mut z, mut w) = (0.0, 0.0);
    for (column, weight) in m.iter().zip(q) {
        z += f64::from(column[2]) * weight;
        w += f64::from(column[3]) * weight;
    }
    z / w
}

#[test]
fn the_depth_planes_enclose_the_points() {
    let mut rng = SplitMix64::new(14);
    for k in 0..100 {
        let projection = [Projection::Perspective, Projection::Orthographic][k % 2];
        let camera = random_camera(&mut rng, projection);
        let points: Vec<[f32; 3]> = (0..12)
            .map(|_| (Vec3::from_array(camera.target) + point(&mut rng, 1.5)).to_array())
            .collect();
        let m = camera.view_proj(SIZE[0] / SIZE[1], &points);
        let eye = Vec3::from_array(camera.eye());
        let ahead = |p: Vec3| (p - eye).dot(camera.view().dir) > 0.0;
        for p in points
            .iter()
            .map(|&p| Vec3::from_array(p))
            .filter(|&p| ahead(p))
        {
            let z = depth_of(&m, p);
            assert!((0.0..=1.0).contains(&z), "{camera:?}: {p:?} at depth {z}");
        }
    }
}

#[test]
fn a_fitted_perspective_eye_keeps_a_tenth_of_the_radius_in_front() {
    // Points along the view axis: the frame's edges are far away, so only the floor binds.
    let mut camera = Camera::preset(Preset::Front);
    let points: Vec<[f32; 3]> = (0..5).map(|k| [0.0, 0.0, k as f32 - 2.0]).collect();
    camera.fit(&points, 1.0);
    let nearest = (points.iter())
        .map(|&p| camera.depth(Vec3::from_array(p)))
        .fold(f32::INFINITY, f32::min);
    // Half the box's diagonal is 2; a tenth of it is 0.2.
    assert!(
        (nearest - 0.2).abs() < 1e-5,
        "the nearest point is {nearest} in front"
    );
    let half = (camera.ray(320.0, 0.0, SIZE).1).dot(camera.view().dir);
    let fov = 2.0 * libm::acosf(half);
    assert!(
        (fov - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
        "a preset sees 45 degrees from top to bottom, not {fov}"
    );
}

#[test]
fn each_preset_looks_from_its_named_side_with_up_at_the_top() {
    let (c, r, s) = (
        3f32.sqrt().recip(),
        2f32.sqrt().recip(),
        6f32.sqrt().recip(),
    );
    // Name, the side the eye is on, and the image's right and up in world space.
    let table = [
        (
            Preset::Front,
            "front",
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            Preset::Back,
            "back",
            [0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
        (
            Preset::Left,
            "left",
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        ),
        (
            Preset::Right,
            "right",
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
        ),
        (
            Preset::Top,
            "top",
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
        ),
        (
            Preset::Bottom,
            "bottom",
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        ),
        (
            Preset::Iso,
            "iso",
            [c, c, c],
            [r, 0.0, -r],
            [-s, 2.0 * s, -s],
        ),
    ];
    let close = |a: Vec3, b: [f32; 3]| (a - Vec3::from_array(b)).len2() < 1e-10;
    for (preset, name, side, right, up) in table {
        assert_eq!(preset.name(), name);
        let mut camera = Camera::preset(preset);
        assert!(
            close(Vec3::from_array(camera.eye()), side),
            "{name}: the eye is at {:?}",
            camera.eye()
        );
        // Orthographic ray origins move across the image plane exactly.
        camera.projection = Projection::Orthographic;
        let origin = |x, y| camera.ray(x, y, SIZE).0;
        let center = origin(320.0, 200.0);
        let along = |p: Vec3| {
            let d = p - center;
            d * d.len2().sqrt().recip()
        };
        assert!(close(along(origin(640.0, 200.0)), right), "{name}: right");
        assert!(close(along(origin(320.0, 0.0)), up), "{name}: up");
    }
}

#[test]
fn a_fitted_camera_aims_at_the_middle_of_the_points() {
    let points = [[1.0, 2.0, 3.0], [3.0, -2.0, 5.0], [2.0, 0.0, 4.0]];
    for preset in Preset::ALL {
        let camera = Camera::fitted(preset, &points, 1.5);
        assert_eq!(camera.target, [2.0, 0.0, 4.0], "{preset:?}");
    }
}
