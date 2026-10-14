//! Orbit camera: presets, fit-to-bounds, projections.

use crate::common::{BACKGROUND, lavapipe};
use gs_render::{Camera, Fit, Lens, Preset, Projection, RenderSize, Shading, UploadOptions, View};
use proptest::prelude::*;

/// Clip-space position of `p`, computed in f64 from the renderer's own matrix.
fn clip(vp: &[[f32; 4]; 4], p: [f32; 3]) -> [f64; 4] {
    let v = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), 1.0];
    std::array::from_fn(|r| (0..4).map(|c| f64::from(vp[c][r]) * v[c]).sum())
}

fn ndc(vp: &[[f32; 4]; 4], p: [f32; 3]) -> [f64; 3] {
    let c = clip(vp, p);
    assert!(
        c[3] > 0.0,
        "point {p:?} is in front of the camera (w = {})",
        c[3]
    );
    [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
}

fn any_preset() -> impl Strategy<Value = Preset> {
    proptest::sample::select(Preset::ALL.to_vec())
}

fn any_projection() -> impl Strategy<Value = Projection> {
    prop_oneof![
        Just(Projection::Perspective),
        Just(Projection::Orthographic)
    ]
}

/// 3 to 40 points in a box of random size and place.
fn point_cloud() -> impl Strategy<Value = Vec<[f32; 3]>> {
    (
        prop::collection::vec(prop::array::uniform3(-1.0f32..1.0), 3..40),
        0.01f32..1000.0,
        prop::array::uniform3(-100.0f32..100.0),
    )
        .prop_map(|(points, scale, offset)| {
            points
                .into_iter()
                .map(|p| std::array::from_fn(|k| p[k] * scale + offset[k]))
                .collect()
        })
}

/// Depth along the view of every point, from the camera's own eye and target, in f64.
fn depths(camera: &Camera, points: &[[f32; 3]]) -> Vec<f64> {
    let (eye, target) = (camera.eye().map(f64::from), camera.target.map(f64::from));
    let f: Vec<f64> = (0..3).map(|k| target[k] - eye[k]).collect();
    let len = f.iter().map(|c| c * c).sum::<f64>().sqrt();
    points
        .iter()
        .map(|p| {
            (0..3)
                .map(|k| (f64::from(p[k]) - eye[k]) * f[k] / len)
                .sum()
        })
        .collect()
}

/// The documented depth planes: around the target and the points, padded by 1% of the larger
/// of their depth extent and the camera distance; a perspective near plane above far / 1e5.
fn expected_planes(camera: &Camera, depths: &[f64]) -> (f64, f64) {
    let d = f64::from(camera.distance);
    let (lo, hi) = depths
        .iter()
        .fold((d, d), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    let pad = (hi - lo).max(d) * 0.01;
    let far = hi + pad;
    match camera.projection {
        Projection::Perspective => ((lo - pad).max(far * 1e-5), far),
        Projection::Orthographic => (lo - pad, far),
    }
}

/// Where a point at view depth `depth` lands in normalized depth, given the planes: reversed
/// (decision 0019), `near / depth` in perspective (no far plane), linear from 1 at the near
/// plane to 0 at the far one in orthographic.
fn expected_depth(camera: &Camera, (near, far): (f64, f64), depth: f64) -> f64 {
    match camera.projection {
        Projection::Perspective => near / depth,
        Projection::Orthographic => (far - depth) / (far - near),
    }
}

proptest! {
    /// Every point lands inside the frame, at the documented depth, and the frame is used: the
    /// outermost point sits on the 5% border unless the eye's distance floor (a tenth of the
    /// points' radius in front of every point) holds it back.
    #[test]
    fn fit_keeps_every_vertex_in_frame(
        points in point_cloud(),
        preset in any_preset(),
        projection in any_projection(),
        aspect in 0.5f32..2.0,
        fov_degrees in 10.0f32..120.0,
    ) {
        let mut camera = Camera { projection, lens: Lens::from_fov(fov_degrees.to_radians(), Fit::Vertical), ..Camera::preset(preset) };
        camera.fit(&points, aspect);
        let vp = camera.view_proj(aspect, &points);
        let depth = depths(&camera, &points);
        let planes = expected_planes(&camera, &depth);
        let mut widest = 0.0f64;
        for (&p, &d) in points.iter().zip(&depth) {
            let [x, y, z] = ndc(&vp, p);
            prop_assert!(x.abs() <= 1.0 && y.abs() <= 1.0, "{p:?} is inside the frame: ndc ({x}, {y})");
            prop_assert!((0.0..=1.0).contains(&z), "{p:?} is between the depth planes: {z}");
            prop_assert!((z - expected_depth(&camera, planes, d)).abs() < 1e-4, "{p:?} sits at the documented depth: {z}");
            widest = widest.max(x.abs()).max(y.abs());
        }
        let size: Vec<f64> = (0..3).map(|k| {
            let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(f64::from(p[k])), hi.max(f64::from(p[k]))));
            hi - lo
        }).collect();
        let floor = 0.05 * size.iter().map(|s| s * s).sum::<f64>().sqrt();
        let nearest = depth.iter().copied().fold(f64::MAX, f64::min);
        let on_floor = projection == Projection::Perspective && (nearest - floor).abs() < 1e-3 * floor;
        prop_assert!(projection == Projection::Orthographic || nearest > floor * (1.0 - 1e-3), "the eye is a tenth of the radius in front of every point");
        prop_assert!((widest - 0.9).abs() < 1e-3 || on_floor, "the outermost point is on the 5% border: {widest}");
    }

}

/// The same on the GPU: a fitted random mesh never touches the image border.
#[test]
fn fitted_render_never_touches_the_border() {
    // Opened before the runner, so a device that fails to open fails once, not per case.
    let r = lavapipe();
    let index = || any::<prop::sample::Index>();
    proptest!(|(
        positions in prop::collection::vec(prop::array::uniform3(-3.0f32..3.0), 4..30),
        picks in prop::collection::vec((index(), index(), index(), index(), any::<bool>()), 1..20),
        preset in any_preset(),
        projection in any_projection(),
    )| {
        let n = positions.len();
        let faces: Vec<[u32; 4]> = picks.iter().map(|(a, b, c, d, quad)| {
            let pick = |i: &prop::sample::Index| i.index(n) as u32;
            [pick(a), pick(b), pick(c), if *quad { pick(d) } else { u32::MAX }]
        }).collect();
        let mesh = r.upload(&crate::scene::plain(positions, faces), UploadOptions::default()).expect("valid mesh");
        let (w, h) = (64u32, 48u32);
        let mut camera = Camera { projection, ..Camera::preset(preset) };
        camera.fit(&mesh.fit_points(), w as f32 / h as f32);
        let view = View::new(camera, Shading::Clay);
        let image = r.snapshot(&mesh, &view, RenderSize { width: w, height: h, samples: 1 }).expect("renders");
        let border = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
        for (x, y) in border {
            prop_assert_eq!(image.pixel(x, y), BACKGROUND, "border pixel ({}, {}) is background", x, y);
        }
    });
}

fn scaled(v: [f64; 3], s: f64) -> [f32; 3] {
    v.map(|c| (c * s) as f32)
}

/// Preset, eye direction, screen up, screen right.
type PresetRow = (Preset, [f64; 3], [f64; 3], [f64; 3]);

fn preset_table() -> [PresetRow; 7] {
    let s = 1.0 / 3f64.sqrt();
    [
        (
            Preset::Front,
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ),
        (
            Preset::Back,
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
        ),
        (
            Preset::Left,
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ),
        (
            Preset::Right,
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
        ),
        (
            Preset::Top,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
        ),
        (
            Preset::Bottom,
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
        ),
        (
            Preset::Iso,
            [s, s, s],
            [
                -0.5 * s * 2f64.sqrt(),
                s * 2f64.sqrt(),
                -0.5 * s * 2f64.sqrt(),
            ],
            [2f64.sqrt() / 2.0, 0.0, -(2f64.sqrt()) / 2.0],
        ),
    ]
}

/// Each preset looks from its named side with the expected screen-up direction.
#[test]
fn presets_look_from_their_named_side() {
    let depth = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];
    for (preset, eye, up, right) in preset_table() {
        let camera = Camera {
            distance: 5.0,
            ..Camera::preset(preset)
        };
        let got = camera.eye();
        for k in 0..3 {
            assert!(
                (f64::from(got[k]) - 5.0 * eye[k]).abs() < 1e-5,
                "{preset:?}: eye {got:?} is 5 along {eye:?}"
            );
        }
        let vp = camera.view_proj(1.0, &depth);
        let [ux, uy, _] = ndc(&vp, scaled(up, 0.5));
        assert!(
            uy > 0.05 && ux.abs() < 1e-5,
            "{preset:?}: up {up:?} points up on screen, ndc ({ux}, {uy})"
        );
        let [rx, ry, _] = ndc(&vp, scaled(right, 0.5));
        assert!(
            rx > 0.05 && ry.abs() < 1e-5,
            "{preset:?}: right {right:?} points right on screen, ndc ({rx}, {ry})"
        );
        // Reversed depth: nearer is greater (decision 0019).
        let [_, _, near] = ndc(&vp, scaled(eye, 0.5));
        let [_, _, far] = ndc(&vp, scaled(eye, -0.5));
        assert!(near > far, "{preset:?}: the side facing the eye is nearer");
    }
    assert_eq!(
        Preset::ALL.map(Preset::name),
        ["front", "back", "left", "right", "top", "bottom", "iso"],
        "preset names label contact sheets"
    );
}

/// A mesh lying along the view direction would put its tip at the eye; the fit keeps the eye
/// a tenth of the mesh's radius in front of it.
#[test]
fn mesh_pointing_at_the_camera_keeps_its_distance() {
    let along_view = [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0]];
    let camera = Camera::fitted(Preset::Front, &along_view, 1.0);
    assert!(
        (camera.distance - 1.1).abs() < 1e-6,
        "eye at 1.1, a tenth of the radius past the tip: {}",
        camera.distance
    );
    let ortho = Camera {
        projection: Projection::Orthographic,
        ..Camera::preset(Preset::Front)
    };
    let mut fitted = ortho;
    fitted.fit(&along_view, 1.0);
    assert!(
        (fitted.distance - 0.1).abs() < 1e-6,
        "orthographic: the floor keeps the view open: {}",
        fitted.distance
    );
}

/// From inside the mesh, the perspective near plane stops at 1e-5 of the far plane.
#[test]
fn camera_inside_the_mesh_keeps_a_positive_near_plane() {
    let points = [[0.0, 0.0, 10.0], [0.0, 0.0, -10.0]];
    let camera = Camera {
        distance: 1.0,
        ..Camera::preset(Preset::Front)
    };
    let vp = camera.view_proj(1.0, &points);
    let (near, far) = expected_planes(&camera, &depths(&camera, &points));
    assert!((near - far * 1e-5).abs() < 1e-9, "near is clamped: {near}");
    for (depth, want) in [
        (near, 1.0),
        (2.0 * near, expected_depth(&camera, (near, far), 2.0 * near)),
    ] {
        let [_, _, z] = ndc(&vp, [0.0, 0.0, (1.0 - depth) as f32]);
        assert!((z - want).abs() < 1e-3, "depth {depth} maps to {want}: {z}");
    }
}

/// Orthographic rays are parallel and frame the target plane like the perspective camera does.
#[test]
fn orthographic_is_parallel_and_keeps_the_framing() {
    let depth = [[-2.0, -2.0, -2.0], [2.0, 2.0, 2.0]];
    let perspective = Camera {
        distance: 4.0,
        lens: Lens::from_fov(0.8, Fit::Vertical),
        ..Camera::preset(Preset::Front)
    };
    let ortho = Camera {
        projection: Projection::Orthographic,
        ..perspective
    };
    let (p_vp, o_vp) = (
        perspective.view_proj(1.5, &depth),
        ortho.view_proj(1.5, &depth),
    );
    let (near, far) = ([0.5f32, 0.5, 1.0], [0.5f32, 0.5, -1.0]);
    let (on, of) = (ndc(&o_vp, near), ndc(&o_vp, far));
    assert!(
        (on[0] - of[0]).abs() < 1e-6 && (on[1] - of[1]).abs() < 1e-6,
        "orthographic: depth does not move a point"
    );
    let (pn, pf) = (ndc(&p_vp, near), ndc(&p_vp, far));
    assert!(
        pn[0] > pf[0] * 1.2,
        "perspective: nearer points look bigger"
    );
    let half_height = 4.0 * libm::tan(0.4);
    for vp in [p_vp, o_vp] {
        let [x, y, _] = ndc(&vp, [(half_height * 1.5) as f32, half_height as f32, 0.0]);
        assert!(
            (x - 1.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5,
            "the target plane's corner is the frame's corner: ({x}, {y})"
        );
    }
}
