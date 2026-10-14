//! Reversed float depth (decision 0019, research 14 section 1.4): with a part almost touching
//! the eye, which pulls the near plane in to a thousandth of a unit, two surfaces 1e-4 apart
//! a hundred units away still sort correctly. With the old depth (1 at the far plane) their
//! depths would differ by about 1e-11 near 1.0, far below a float's spacing there (6e-8).

use crate::common::lavapipe;
use crate::scene;
use gs_render::{Camera, Preset, RenderSize, Scene, Shading, UiLayer, UploadOptions, View};

/// A quad from (x0, y0) to (x1, y1) at depth z.
fn quad([x0, y0, x1, y1]: [f32; 4], z: f32) -> (Vec<[f32; 3]>, [u32; 4]) {
    (
        vec![[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
        [0, 1, 2, 3],
    )
}

#[test]
fn surfaces_a_ten_thousandth_apart_sort_at_a_hundred_units() {
    let r = lavapipe();
    // Behind: a big quad facing up and right; 1e-4 in front of it: a smaller quad facing the
    // eye. The normals view colors them differently.
    let (mut positions, _) = quad([-20.0, -20.0, 20.0, 20.0], -1e-4);
    let (front, _) = quad([-10.0, -10.0, 10.0, 10.0], 0.0);
    positions.extend(front);
    let faces = vec![[0, 1, 2, 3], [4, 5, 6, 7]];
    let tilted = [0.6, 0.0, 0.8];
    let normals = [
        tilted,
        tilted,
        tilted,
        tilted,
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];
    let pair = scene::plain(positions, faces);
    let pair = r
        .upload(
            gs_render::RenderMesh::from(&pair).with_normals(&normals),
            UploadOptions::default(),
        )
        .expect("uploads");
    // A speck a hundredth of a unit in front of the eye, in a corner of the view.
    let (speck, _) = quad([0.003, 0.003, 0.0035, 0.0035], 99.99);
    let speck = r
        .upload(
            &scene::plain(speck, vec![[0, 1, 2, 3]]),
            UploadOptions::default(),
        )
        .expect("uploads");
    let camera = Camera {
        distance: 100.0,
        ..Camera::preset(Preset::Front)
    };
    let size = RenderSize::square(128);
    let target = r.frame_target(size).expect("target");
    let scene = Scene {
        meshes: &[&pair, &speck],
        view: View::new(camera, Shading::Normals),
        viewport: [0, 0, 128, 128],
    };
    r.draw_frame(&target, &scene, &mut r.ui_painter(), &UiLayer::default())
        .expect("draws");
    let image = r.read_frame(&target).expect("reads");
    // At 100 units the frame is 82.8 units tall: the front quad covers pixels 49 to 79 and the
    // one behind 33 to 95.
    let facing = [128u8, 128, 255];
    let mut checked = 0;
    for y in 52..76 {
        for x in 52..76 {
            let p = image.pixel(x, y);
            let off = (0..3).map(|k| p[k].abs_diff(facing[k])).max();
            assert!(
                off <= Some(2),
                "pixel ({x}, {y}) shows the nearer quad, not the one behind it: {p:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 24 * 24);
    let behind = image.pixel(40, 64);
    assert!(
        behind[0] > 200 && behind[2] < 240,
        "the quad behind shows where nothing is in front of it: {behind:?}"
    );
}
