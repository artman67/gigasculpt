//! Shading modes against analytic expectations, and the snapshot's output size rules.

use crate::common::{BACKGROUND, diff, fitted, front_ortho, lavapipe, shipped, snap, upload};
use crate::scene;
use gs_render::display::encode;
use gs_render::matcap::{Bake, MatCapImage, Options, Recipe, bake_under};
use gs_render::shading::Environment;
use gs_render::{
    Camera, Display, Fit, Image, Lens, MAX_SAMPLES, Preset, Projection, RenderError, RenderSize,
    Shading, View, ViewTransform, Wire,
};

/// The unit-sphere normal (and its image position) seen at the center of pixel (px, py).
fn sphere_at(image: &Image, px: u32, py: u32) -> Option<[f64; 3]> {
    let x = ((f64::from(px) + 0.5) / f64::from(image.width) * 2.0 - 1.0) * 1.1;
    let y = (1.0 - (f64::from(py) + 0.5) / f64::from(image.height) * 2.0) * 1.1;
    let r2 = x * x + y * y;
    (r2 < 0.85).then(|| [x, y, (1.0 - r2).sqrt()])
}

fn byte(v: f64) -> f64 {
    (v * 255.0).round()
}

#[test]
fn normals_view_matches_the_sphere() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let image = snap(r, &mesh, &front_ortho(Shading::Normals), 128);
    let mut checked = 0;
    for (px, py) in (0..128).flat_map(|y| (0..128).map(move |x| (x, y))) {
        let Some(n) = sphere_at(&image, px, py) else {
            continue;
        };
        for (k, (got, n)) in image.pixel(px, py).into_iter().zip(n).enumerate() {
            let want = byte(n * 0.5 + 0.5);
            assert!(
                (f64::from(got) - want).abs() <= 3.0,
                "pixel ({px}, {py}) channel {k}: {got} against the sphere's {want}"
            );
        }
        checked += 1;
    }
    assert!(
        checked > 8000,
        "most of the sphere was checked ({checked} pixels)"
    );
}

/// A MatCap whose red and green channels are its own column and row, so a pixel's color says
/// where in the image it was looked up.
fn position_matcap(size: u32) -> MatCapImage {
    let level = |i: u32| (u32::from(u16::MAX) * i / (size - 1)) as u16;
    let pixels = (0..size)
        .flat_map(|row| (0..size).map(move |col| [level(col), level(row), 0, u16::MAX]))
        .collect();
    MatCapImage { size, pixels }
}

/// The 0..255 level the position MatCap holds at image position `t` (0..1) along an axis.
fn position_level(t: f64) -> f64 {
    byte((t * 64.0 - 0.5) / 63.0)
}

/// In orthographic views the lookup is the view-space normal, scaled by 0.496 about the
/// image center (material.matcap-shading).
#[test]
fn matcap_lookup_uses_the_view_space_normal() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let matcap = r.upload_matcap(&position_matcap(64));
    let mut view = front_ortho(Shading::MatCap(&matcap));
    view.display.dither = false;
    let image = snap(r, &mesh, &view, 128);
    for (px, py) in (8..128)
        .step_by(16)
        .flat_map(|y| (8..128).step_by(16).map(move |x| (x, y)))
    {
        let Some(n) = sphere_at(&image, px, py) else {
            continue;
        };
        let want_r = position_level(0.5 + 0.496 * n[0]);
        let want_g = position_level(0.5 - 0.496 * n[1]);
        let got = image.pixel(px, py);
        assert!(
            (f64::from(got[0]) - want_r).abs() <= 3.0,
            "({px}, {py}) looks up column {want_r}, got {}",
            got[0]
        );
        assert!(
            (f64::from(got[1]) - want_g).abs() <= 3.0,
            "({px}, {py}) looks up row {want_g}, got {}",
            got[1]
        );
    }
}

/// In perspective the lookup turns with the view direction (Blender's view-corrected basis):
/// the point of an off-center sphere that faces the eye, which projects onto the sphere's
/// center, reads the image center although its view-space normal points sideways.
#[test]
fn a_surface_facing_the_eye_reads_the_image_center_anywhere_on_screen() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let matcap = r.upload_matcap(&position_matcap(64));
    let camera = Camera {
        distance: 4.0,
        target: [1.6, 0.9, 0.0],
        lens: Lens::from_fov(1.2, Fit::Vertical),
        projection: Projection::Perspective,
        ..Camera::preset(Preset::Front)
    };
    let mut view = View::new(camera, Shading::MatCap(&matcap));
    view.display.dither = false;
    let image = snap(r, &mesh, &view, 256);
    // Where the sphere's center projects: column 3 of the view-projection matrix.
    let m = camera.view_proj(1.0, &mesh.fit_points());
    let (x, y) = (m[3][0] / m[3][3], m[3][1] / m[3][3]);
    let (px, py) = (((x + 1.0) * 128.0) as u32, ((1.0 - y) * 128.0) as u32);
    assert!(
        px < 100 && py > 156,
        "the sphere is off center, at ({px}, {py})"
    );
    let got = image.pixel(px, py);
    let center = position_level(0.5);
    for k in 0..2 {
        assert!(
            (f64::from(got[k]) - center).abs() <= 3.0,
            "channel {k} at ({px}, {py}): {got:?}, not the image center {center}"
        );
    }
}

/// material.matcap-shading and the display step: a camera-facing sphere shows at its center
/// pixel the look's `albedo x D + S` at the image center, through every view transform at two
/// exposures, as the CPU twins of the transforms compute it (no dither), within one level.
#[test]
fn matcap_center_pixel_is_the_bake_center() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let env = Environment::shipped("studio").expect("the studio builds");
    for name in ["gray", "red-wax", "chrome-a"] {
        let recipe = Recipe {
            bake: Bake {
                size: 64,
                ..Bake::default()
            },
            ..shipped(name)
        };
        let baked = bake_under(&recipe, &env, Options::default());
        let matcap = r.upload_bake(&baked);
        // The image center falls between four texels; bilinear lookup averages them.
        let four = [(31, 31), (32, 31), (31, 32), (32, 32)].map(|(x, y)| baked.radiance(x, y));
        let mean: [f32; 3] = std::array::from_fn(|k| four.iter().map(|c| c[k]).sum::<f32>() / 4.0);
        for (transform, exposure) in ViewTransform::ALL
            .into_iter()
            .flat_map(|t| [(t, 0.0), (t, 1.5)])
        {
            let display = Display {
                exposure,
                transform,
                dither: false,
            };
            let view = View {
                display,
                ..front_ortho(Shading::MatCap(&matcap))
            };
            let image = snap(r, &mesh, &view, 129);
            let want = encode(display.show(mean));
            let got = image.pixel(64, 64);
            for k in 0..3 {
                assert!(
                    got[k].abs_diff(want[k]) <= 1,
                    "{name} {transform:?} at {exposure}, channel {k}: center pixel {got:?} \
                     against the bake's {want:?}"
                );
            }
        }
    }
}

/// With parallel rays the view direction is constant, so moving the camera back (and widening
/// the view to keep the framing) changes nothing. In perspective it changes the reflections.
#[test]
fn orthographic_shading_ignores_distance() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let at = |distance: f32, projection| {
        let fov_y = 2.0 * libm::atanf(1.1 / distance);
        let camera = Camera {
            distance,
            lens: Lens::from_fov(fov_y, Fit::Vertical),
            projection,
            ..Camera::preset(Preset::Front)
        };
        snap(r, &mesh, &View::new(camera, Shading::Zebra), 96)
    };
    let (max, count) = diff(
        &at(3.0, Projection::Orthographic),
        &at(30.0, Projection::Orthographic),
    );
    assert!(
        max <= 2 && count < 96,
        "orthographic: same picture from 3 and 30 away (max {max}, {count} pixels differ)"
    );
    let (max, _) = diff(
        &at(3.0, Projection::Perspective),
        &at(30.0, Projection::Perspective),
    );
    assert!(
        max > 50,
        "perspective: the reflections move with the camera (max {max})"
    );
}

#[test]
fn odd_sizes_read_back_without_shear() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(3));
    let mut camera = Camera::preset(Preset::Front);
    camera.fit(&mesh.fit_points(), 100.0 / 61.0);
    let size = RenderSize {
        width: 100,
        height: 61,
        samples: 1,
    };
    let image = r
        .snapshot(&mesh, &View::new(camera, Shading::Clay), size)
        .expect("renders");
    assert_eq!(
        (image.width, image.height, image.rgba.len()),
        (100, 61, 100 * 61 * 4),
        "tightly packed rows"
    );
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, p) in image.rgba.chunks(4).enumerate() {
        if p != BACKGROUND {
            sx += (i % 100) as f64 + 0.5;
            sy += (i / 100) as f64 + 0.5;
            n += 1.0;
        }
    }
    assert!(n > 1000.0, "the sphere is drawn");
    assert!(
        (sx / n - 50.0).abs() < 1.0 && (sy / n - 30.5).abs() < 1.0,
        "the sphere is centered: ({}, {})",
        sx / n,
        sy / n
    );
}

#[test]
fn snapshot_sizes_are_checked() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(1));
    let view = fitted(&mesh, Preset::Front, Shading::Clay, Wire::Off);
    let max = 16384;
    for (width, height, samples) in [
        (0, 8, 1),
        (8, 0, 1),
        (max + 1, 8, 1),
        (8, max + 1, 1),
        (8, 8, MAX_SAMPLES + 1),
        (8, 8, 0),
    ] {
        let result = r.snapshot(
            &mesh,
            &view,
            RenderSize {
                width,
                height,
                samples,
            },
        );
        assert!(
            matches!(result, Err(RenderError::Unsupported(_))),
            "{width}x{height} at {samples} samples is refused"
        );
    }
    let edge = r
        .snapshot(
            &mesh,
            &view,
            RenderSize {
                width: max,
                height: 1,
                samples: 4,
            },
        )
        .expect("the largest width renders");
    assert_eq!(edge.rgba.len(), max as usize * 4, "one full row");
    let tall = r
        .snapshot(
            &mesh,
            &view,
            RenderSize {
                width: 1,
                height: max,
                samples: 1,
            },
        )
        .expect("the largest height renders");
    assert_eq!(tall.rgba.len(), max as usize * 4, "one full column");
}
