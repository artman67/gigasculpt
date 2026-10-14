//! The window's frame path on lavapipe, without a window: the model in its viewport, the
//! interface layer over it (textures, clip rectangles, premultiplied colors), byte-identical
//! repeats, and a snapshot being exactly a frame with no interface.

#[path = "../render/scene.rs"]
#[expect(dead_code, reason = "shared test meshes; this target uses one of them")]
mod scene;

use std::sync::OnceLock;

use gs_render::{
    AdapterKind, BACKGROUND, Camera, GpuMesh, Image, Preset, RenderSize, Renderer, Scene, Shading,
    UiImage, UiLayer, UiMesh, UiTexture, UiVertex, UploadOptions, View,
};

const W: u32 = 200;
const H: u32 = 120;

fn renderer() -> &'static Renderer {
    static R: OnceLock<Renderer> = OnceLock::new();
    R.get_or_init(|| Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}")))
}

fn sphere() -> GpuMesh {
    renderer()
        .upload(&scene::icosphere(3), UploadOptions::default())
        .unwrap_or_else(|e| panic!("{e}"))
}

fn view(mesh: &GpuMesh, aspect: f32) -> View<'static> {
    View::new(
        Camera::fitted(Preset::Front, &mesh.fit_points(), aspect),
        Shading::Clay,
    )
}

fn size() -> RenderSize {
    RenderSize {
        width: W,
        height: H,
        samples: 1,
    }
}

/// A rectangle of two triangles from (x0, y0) to (x1, y1), uv 0..1, one color.
fn rect(texture: u64, [x0, y0, x1, y1]: [f32; 4], clip: [f32; 4], color: [u8; 4]) -> UiMesh {
    let v = |x, y, u, w| UiVertex {
        pos: [x, y],
        uv: [u, w],
        color,
    };
    UiMesh {
        clip,
        texture,
        vertices: vec![
            v(x0, y0, 0.0, 0.0),
            v(x1, y0, 1.0, 0.0),
            v(x1, y1, 1.0, 1.0),
            v(x0, y1, 0.0, 1.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn image(width: u32, height: u32, rgba: [u8; 4]) -> UiImage {
    UiImage {
        width,
        height,
        rgba: rgba.repeat((width * height) as usize),
        smooth: false,
    }
}

fn set(id: u64, image: UiImage) -> UiTexture {
    UiTexture::Set {
        id,
        at: None,
        image,
    }
}

const ALL: [f32; 4] = [0.0, 0.0, W as f32, H as f32];
const WHITE: [u8; 4] = [255; 4];

/// Draws the sphere in `viewport` with `ui` over it and reads the frame back.
fn frame(mesh: &GpuMesh, viewport: [u32; 4], ui: &[UiLayer]) -> Image {
    let r = renderer();
    let target = r.frame_target(size()).unwrap_or_else(|e| panic!("{e}"));
    let mut painter = r.ui_painter();
    let aspect = viewport[2].max(1) as f32 / viewport[3].max(1) as f32;
    let scene = Scene {
        meshes: &[mesh],
        view: view(mesh, aspect),
        viewport,
    };
    for layer in ui {
        r.draw_frame(&target, &scene, &mut painter, layer)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    r.read_frame(&target).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_interface_draws_over_the_model_in_its_viewport() {
    let mesh = sphere();
    let ui = UiLayer {
        textures: vec![set(1, image(2, 2, WHITE))],
        meshes: vec![
            rect(1, [10.0, 10.0, 50.0, 30.0], ALL, [200, 0, 0, 255]),
            // Only the part inside the clip rectangle shows.
            rect(1, ALL, [100.4, 100.6, 110.0, 110.0], [0, 0, 255, 255]),
            // Clip rectangles reaching past the frame are cut at its edges.
            rect(1, ALL, [-50.0, 115.0, 9.0, 500.0], [0, 255, 0, 255]),
            // Empty clip: nothing.
            rect(1, ALL, [30.0, 30.0, 30.0, 90.0], [255, 255, 0, 255]),
            // Unknown texture: skipped.
            rect(9, ALL, ALL, [255, 0, 255, 255]),
        ],
    };
    let shot = frame(&mesh, [60, 0, 140, 120], &[ui]);
    assert_eq!(shot.pixel(20, 20), [200, 0, 0, 255], "red rectangle");
    assert_eq!(shot.pixel(100, 100), [0, 0, 255, 255], "clip floor");
    assert_eq!(shot.pixel(109, 109), [0, 0, 255, 255], "clip inside");
    assert_eq!(shot.pixel(110, 105), BACKGROUND, "clip right edge");
    assert_eq!(
        shot.pixel(0, 119),
        [0, 255, 0, 255],
        "clip cut at the frame"
    );
    assert_eq!(shot.pixel(9, 119), BACKGROUND, "clip max x");
    assert_eq!(shot.pixel(30, 60), BACKGROUND, "empty clip draws nothing");
    assert_eq!(shot.pixel(5, 60), BACKGROUND, "left of the viewport");
    assert_ne!(
        shot.pixel(130, 60),
        BACKGROUND,
        "the model fills its viewport's center"
    );
    assert_eq!(
        shot.pixel(59, 60),
        BACKGROUND,
        "the model never leaves its viewport"
    );
    let again = frame(
        &mesh,
        [60, 0, 140, 120],
        &[UiLayer {
            textures: vec![set(1, image(2, 2, WHITE))],
            meshes: vec![
                rect(1, [10.0, 10.0, 50.0, 30.0], ALL, [200, 0, 0, 255]),
                rect(1, ALL, [100.4, 100.6, 110.0, 110.0], [0, 0, 255, 255]),
                rect(1, ALL, [-50.0, 115.0, 9.0, 500.0], [0, 255, 0, 255]),
                rect(1, ALL, [30.0, 30.0, 30.0, 90.0], [255, 255, 0, 255]),
                rect(9, ALL, ALL, [255, 0, 255, 255]),
            ],
        }],
    );
    assert_eq!(shot, again, "byte-identical on lavapipe");
}

#[test]
fn colors_are_premultiplied_and_blend_on_encoded_values() {
    let mesh = sphere();
    let half_red = [128, 0, 0, 128];
    let ui = UiLayer {
        textures: vec![set(1, image(1, 1, WHITE))],
        meshes: vec![rect(1, [0.0, 0.0, 20.0, 20.0], ALL, half_red)],
    };
    let shot = frame(&mesh, [100, 0, 100, 120], &[ui]);
    let keep = 1.0 - 128.0 / 255.0;
    let expect = [
        128.0 + f64::from(BACKGROUND[0]) * keep,
        f64::from(BACKGROUND[1]) * keep,
        f64::from(BACKGROUND[2]) * keep,
    ];
    let got = shot.pixel(10, 10);
    for c in 0..3 {
        assert!(
            (f64::from(got[c]) - expect[c]).abs() <= 1.0,
            "channel {c}: {got:?} vs {expect:?}"
        );
    }
}

#[test]
fn textures_are_created_patched_and_freed() {
    let r = renderer();
    let mut painter = r.ui_painter();
    let target = r.frame_target(size()).unwrap_or_else(|e| panic!("{e}"));
    let mesh = sphere();
    let scene = Scene {
        meshes: &[&mesh],
        view: view(&mesh, 1.0),
        viewport: [0, 0, 0, 0],
    };
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let mut draw = |textures: Vec<UiTexture>, meshes: Vec<UiMesh>| {
        r.draw_frame(&target, &scene, &mut painter, &UiLayer { textures, meshes })
    };
    let quad = || vec![rect(2, [0.0, 0.0, 20.0, 20.0], ALL, WHITE)];
    let patch = |at, image| UiTexture::Set {
        id: 2,
        at: Some(at),
        image,
    };
    draw(vec![set(2, image(2, 2, red))], quad()).unwrap_or_else(|e| panic!("{e}"));
    draw(vec![patch([1, 1], image(1, 1, green))], quad()).unwrap_or_else(|e| panic!("{e}"));
    let shot = r.read_frame(&target).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(shot.pixel(5, 5), red, "top-left texel kept");
    assert_eq!(shot.pixel(15, 15), green, "bottom-right texel patched");
    // A patch flush with the far edge fits; one texel further does not.
    draw(vec![patch([0, 1], image(2, 1, green))], vec![]).unwrap_or_else(|e| panic!("{e}"));
    for bad in [
        patch([1, 0], image(2, 1, green)),
        patch([0, 1], image(1, 2, green)),
        patch(
            [0, 0],
            UiImage {
                rgba: vec![0; 3],
                ..image(1, 1, green)
            },
        ),
        UiTexture::Set {
            id: 5,
            at: Some([0, 0]),
            image: image(1, 1, green),
        },
    ] {
        assert!(draw(vec![bad.clone()], vec![]).is_err(), "refused: {bad:?}");
    }
    draw(vec![set(3, image(0, 0, red)), UiTexture::Free(2)], quad())
        .unwrap_or_else(|e| panic!("{e}"));
    let shot = r.read_frame(&target).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        shot.pixel(5, 5),
        BACKGROUND,
        "a freed texture draws nothing"
    );
    assert_eq!(
        painter.texture_ids(),
        vec![3],
        "an empty image still makes a texture"
    );
}

#[test]
fn a_snapshot_is_a_frame_without_interface() {
    let r = renderer();
    let mesh = sphere();
    let v = view(&mesh, W as f32 / H as f32);
    let snapshot = r
        .snapshot(&mesh, &v, size())
        .unwrap_or_else(|e| panic!("{e}"));
    let framed = frame(&mesh, [0, 0, W, H], &[UiLayer::default()]);
    assert_eq!(snapshot, framed);
    // A viewport hanging off the frame is cut at its edge, not refused.
    let cut = frame(&mesh, [150, 0, 100, 120], &[UiLayer::default()]);
    assert_ne!(cut.pixel(175, 60), BACKGROUND, "model in the cut viewport");
    assert_eq!(cut.pixel(149, 60), BACKGROUND, "nothing left of it");
    let low = frame(&mesh, [0, 60, W, 100], &[UiLayer::default()]);
    assert!(
        low.rgba[..(60 * W * 4) as usize]
            .chunks(4)
            .all(|p| p == BACKGROUND),
        "nothing above a viewport that starts at row 60"
    );
    assert_ne!(
        low.pixel(W / 2, 90),
        BACKGROUND,
        "the model fills what is left of it"
    );
    let none = frame(&mesh, [W, 0, 50, H], &[UiLayer::default()]);
    assert!(
        none.rgba.chunks(4).all(|p| p == BACKGROUND),
        "a viewport outside the frame draws nothing"
    );
    let samples = RenderSize {
        samples: 0,
        ..size()
    };
    assert!(r.frame_target(samples).is_err(), "no samples per pixel");
}
