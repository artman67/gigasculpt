//! The camera's navigation moves (research 14 phase A, decision 0017), as metamorphic
//! relations over random cameras of every kind: the point a move turns or scales about keeps
//! its pixel, a pan moves the pivot's image by exactly the drag, a turntable keeps the horizon
//! level, a snap lands on an axis view, switching projection keeps the framing, the turn stays
//! a rotation over a long run of moves, and the same drags on a scene a thousand times larger
//! draw the same pixels.

use gs_core::camera::{Axis, Camera, OrbitMode, Projection};
use gs_core::{Quat, Vec3};
use gs_testkit::SplitMix64;
use gs_testkit::rng::signed_unit;

use crate::camera::random_camera;

const SIZE: [f32; 2] = [640.0, 400.0];
const MODES: [OrbitMode; 4] = [
    OrbitMode::Free,
    OrbitMode::Axis(Axis::X),
    OrbitMode::Axis(Axis::Y),
    OrbitMode::Axis(Axis::Z),
];

fn u(rng: &mut SplitMix64) -> f32 {
    signed_unit(rng.next_u64())
}

/// A point the camera sees, about the target's depth, somewhere in the picture.
fn seen_point(rng: &mut SplitMix64, camera: &Camera) -> Vec3 {
    let (x, y) = (
        SIZE[0] * (0.2 + 0.6 * u(rng).abs()),
        SIZE[1] * (0.2 + 0.6 * u(rng).abs()),
    );
    let (o, d) = camera.ray(x, y, SIZE);
    o + d * (camera.distance * (0.6 + 0.8 * u(rng).abs()))
}

/// Pixels between where `camera` draws `p` and `at`.
fn moved(camera: &Camera, p: Vec3, at: [f32; 2]) -> f32 {
    let now = camera
        .project(p.to_array(), SIZE)
        .unwrap_or([f32::INFINITY; 2]);
    libm::hypotf(now[0] - at[0], now[1] - at[1])
}

fn projection(k: usize) -> Projection {
    [Projection::Perspective, Projection::Orthographic][k % 2]
}

#[test]
fn turning_or_scaling_about_a_point_keeps_it_on_its_pixel() {
    let mut rng = SplitMix64::new(21);
    for k in 0..400 {
        let mut camera = random_camera(&mut rng, projection(k));
        let pivot = seen_point(&mut rng, &camera);
        let Some(before) = camera.project(pivot.to_array(), SIZE) else {
            continue;
        };
        let (right, up) = (2.0 * u(&mut rng), 1.5 * u(&mut rng));
        match k % 3 {
            0 => camera.orbit(pivot, MODES[(k / 3) % 4], right, up),
            1 => camera.roll(pivot, right),
            _ => camera.dolly(pivot, 0.2 + 1.5 * u(&mut rng).abs()),
        }
        let off = moved(&camera, pivot, before);
        assert!(
            off < 0.02,
            "move {}: {camera:?} moved the pivot {off} px",
            k % 3
        );
    }
}

#[test]
fn a_pan_moves_the_pivot_by_exactly_the_drag() {
    let mut rng = SplitMix64::new(22);
    for k in 0..200 {
        let mut camera = random_camera(&mut rng, projection(k));
        let pivot = seen_point(&mut rng, &camera);
        let Some([x, y]) = camera.project(pivot.to_array(), SIZE) else {
            continue;
        };
        let (dx, dy) = (100.0 * u(&mut rng), 100.0 * u(&mut rng));
        camera.pan(pivot, dx, dy, SIZE);
        let off = moved(&camera, pivot, [x + dx, y + dy]);
        assert!(
            off < 0.02,
            "{camera:?}: the pivot is {off} px from the pointer"
        );
    }
}

#[test]
fn a_turntable_keeps_the_horizon_level() {
    let mut rng = SplitMix64::new(23);
    for k in 0..50 {
        let mut camera = random_camera(&mut rng, projection(k));
        // Start level: any heading and tilt, no roll.
        camera.orientation = Camera::turned(3.0 * u(&mut rng), 1.4 * u(&mut rng));
        for _ in 0..200 {
            let pivot = seen_point(&mut rng, &camera);
            let mode = OrbitMode::Axis(Axis::Y);
            camera.orbit(pivot, mode, 0.3 * u(&mut rng), 0.3 * u(&mut rng));
        }
        let right = camera.axes().0;
        assert!(
            right.y.abs() < 1e-5,
            "{camera:?}: the horizon tilts by {}",
            right.y
        );
    }
}

#[test]
fn a_snap_lands_on_an_axis_view_with_a_level_horizon() {
    let mut rng = SplitMix64::new(24);
    let axes = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    for k in 0..500 {
        let mut camera = random_camera(&mut rng, projection(k));
        let before = camera.axes();
        let pivot = Vec3::from_array(camera.target);
        camera.turn_to(camera.snapped(), pivot);
        let (_, up, forward) = camera.axes();
        let on_axis = |v: Vec3| axes.iter().any(|a| (v.dot(*a).abs() - 1.0).abs() < 1e-6);
        assert!(
            on_axis(forward) && on_axis(up),
            "{camera:?}: not an axis view"
        );
        assert!(
            forward.dot(before.2) > 0.5,
            "the nearest axis view, not another"
        );
        if forward.y.abs() < 0.5 {
            assert!(
                (up.y - 1.0).abs() < 1e-6,
                "looking sideways, up is +Y: {up:?}"
            );
        } else {
            assert!(
                up.dot(before.1) > 0.0,
                "looking up or down, up stays nearest: {up:?}"
            );
        }
    }
}

#[test]
fn switching_projection_keeps_the_framing_at_the_target() {
    let mut rng = SplitMix64::new(25);
    for _ in 0..100 {
        let perspective = random_camera(&mut rng, Projection::Perspective);
        let ortho = Camera {
            projection: Projection::Orthographic,
            ..perspective
        };
        // Points on the plane through the target that faces the camera.
        let (right, up, _) = perspective.axes();
        let target = Vec3::from_array(perspective.target);
        for _ in 0..10 {
            let s = perspective.distance;
            let p = target + right * (s * 0.3 * u(&mut rng)) + up * (s * 0.3 * u(&mut rng));
            let a = perspective.project(p.to_array(), SIZE);
            let b = ortho.project(p.to_array(), SIZE);
            let off = a.zip(b).map_or(f32::INFINITY, |(a, b)| {
                libm::hypotf(a[0] - b[0], a[1] - b[1])
            });
            assert!(off < 0.02, "{perspective:?}: {off} px apart");
        }
    }
}

#[test]
fn the_turn_stays_a_rotation_over_a_long_run_of_moves() {
    let mut rng = SplitMix64::new(26);
    let mut camera = random_camera(&mut rng, Projection::Perspective);
    for k in 0..100_000 {
        let pivot = Vec3::new(u(&mut rng), u(&mut rng), u(&mut rng));
        match k % 4 {
            0 => camera.orbit(pivot, MODES[k % 4], u(&mut rng), u(&mut rng)),
            1 => camera.roll(pivot, u(&mut rng)),
            2 => camera.turn_to(camera.snapped(), pivot),
            _ => {
                let axis = Vec3::new(u(&mut rng), u(&mut rng), u(&mut rng)).normalized();
                camera.turn_to(Quat::about(axis, 3.0 * u(&mut rng)), pivot);
            }
        }
        // Keep the camera near the origin so the target does not wander off.
        camera.target = camera.target.map(|t| t.clamp(-2.0, 2.0));
        let q = camera.orientation;
        let n = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
        assert!(
            (n.sqrt() - 1.0).abs() < 1e-6,
            "after {k} moves the turn has length {n}"
        );
    }
}

#[test]
fn the_same_drags_on_a_scene_a_thousand_times_larger_draw_the_same_pixels() {
    let mut rng = SplitMix64::new(27);
    for k in 0..40 {
        let small = random_camera(&mut rng, projection(k));
        let mut large = small;
        large.target = small.target.map(|t| t * 1000.0);
        large.distance = small.distance * 1000.0;
        let points: Vec<Vec3> = (0..8).map(|_| seen_point(&mut rng, &small)).collect();
        let (mut small, mut large) = (small, large);
        for step in 0..12 {
            let pivot = points[step % points.len()];
            let (a, b) = (u(&mut rng), u(&mut rng));
            match step % 3 {
                0 => {
                    small.orbit(pivot, OrbitMode::Free, a, b);
                    large.orbit(pivot * 1000.0, OrbitMode::Free, a, b);
                }
                1 => {
                    small.pan(pivot, 50.0 * a, 50.0 * b, SIZE);
                    large.pan(pivot * 1000.0, 50.0 * a, 50.0 * b, SIZE);
                }
                _ => {
                    let factor = 1.0 + 0.3 * a;
                    small.dolly(pivot, factor);
                    large.dolly(pivot * 1000.0, factor);
                }
            }
        }
        for p in &points {
            let a = small.project(p.to_array(), SIZE);
            let b = large.project((*p * 1000.0).to_array(), SIZE);
            let off = a
                .zip(b)
                .map_or(0.0, |(a, b)| libm::hypotf(a[0] - b[0], a[1] - b[1]));
            assert!(off < 0.05, "{small:?}: the large scene draws {off} px away");
        }
    }
}
