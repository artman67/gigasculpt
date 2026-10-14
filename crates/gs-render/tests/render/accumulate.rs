//! Still-camera accumulation (decision 0019, research 11 section 8): jittered samples average
//! toward the exact coverage of each pixel, the picture stops changing once every sample is in,
//! the window's frames and a snapshot of the same samples are the same bytes, and moving the
//! camera starts again from one sample.

use crate::common::{BACKGROUND, covered, fitted, lavapipe, snap, upload};
use crate::scene;
use gs_render::display::decode;
use gs_render::{
    Camera, Fit, FrameTarget, GpuMesh, Image, Lens, Preset, Projection, RenderSize, Renderer,
    Scene, Shading, UiLayer, View, Wire,
};

const SIZE: u32 = 64;

/// A camera-facing triangle whose long edge crosses the frame at a slant, seen in parallel:
/// x and y from -1 to 1.
fn wedge() -> gs_protocol::PlainMesh {
    scene::plain(
        vec![[-2.0, -1.7, 0.0], [2.0, 0.9, 0.0], [-2.0, 2.0, 0.0]],
        vec![[0, 1, 2, u32::MAX]],
    )
}

fn view() -> View<'static> {
    let camera = Camera {
        distance: 1.0,
        lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
        projection: Projection::Orthographic,
        ..Camera::preset(Preset::Front)
    };
    View::new(camera, Shading::Clay)
}

/// The fraction of pixel (px, py) the triangle covers, by 64 by 64 points in f64.
fn coverage(px: u32, py: u32) -> f64 {
    let (a, b, c) = ([-2.0, -1.7], [2.0, 0.9], [-2.0, 2.0]);
    let side = |p: [f64; 2], q: [f64; 2], s: [f64; 2]| {
        (q[0] - p[0]) * (s[1] - p[1]) - (q[1] - p[1]) * (s[0] - p[0])
    };
    let n = 64;
    let mut inside = 0;
    for j in 0..n {
        for i in 0..n {
            let x = (f64::from(px) + (f64::from(i) + 0.5) / f64::from(n)) / f64::from(SIZE);
            let y = (f64::from(py) + (f64::from(j) + 0.5) / f64::from(n)) / f64::from(SIZE);
            let s = [x * 2.0 - 1.0, 1.0 - y * 2.0];
            let signs = [side(a, b, s), side(b, c, s), side(c, a, s)];
            inside += u32::from(signs.iter().all(|&d| d >= 0.0) || signs.iter().all(|&d| d <= 0.0));
        }
    }
    f64::from(inside) / f64::from(n * n)
}

/// Mean and largest difference, in display-linear light, between the image and the exact
/// coverage-weighted mix of the surface's color and the background, over pixels the edge
/// crosses.
fn error(image: &Image, surface: f64, background: f64) -> (f64, f64) {
    let (mut sum, mut worst, mut count) = (0.0, 0.0f64, 0);
    for py in 0..SIZE {
        for px in 0..SIZE {
            let cover = coverage(px, py);
            if cover <= 0.0 || cover >= 1.0 {
                continue;
            }
            let want = cover * surface + (1.0 - cover) * background;
            let got = f64::from(decode([image.pixel(px, py)[0], 0, 0])[0]);
            sum += (got - want).abs();
            worst = worst.max((got - want).abs());
            count += 1;
        }
    }
    (sum / f64::from(count), worst)
}

fn shot(r: &Renderer, mesh: &GpuMesh, view: &View, samples: u32) -> Image {
    let size = RenderSize {
        samples,
        ..RenderSize::square(SIZE)
    };
    r.snapshot(mesh, view, size).expect("renders")
}

#[test]
fn accumulation_converges_to_the_exact_coverage() {
    let r = lavapipe();
    let mesh = upload(r, &wedge());
    let v = view();
    let one = shot(r, &mesh, &v, 1);
    // Clay on a camera-facing flat triangle is one color; the background is fixed.
    let surface = f64::from(decode([one.pixel(4, 4)[0], 0, 0])[0]);
    let background = f64::from(decode([BACKGROUND[0], 0, 0])[0]);
    assert_ne!(one.pixel(4, 4), BACKGROUND, "the corner is covered");
    let (mean1, _) = error(&one, surface, background);
    let (mean4, _) = error(&shot(r, &mesh, &v, 4), surface, background);
    let (mean16, worst16) = error(&shot(r, &mesh, &v, 16), surface, background);
    assert!(
        mean4 < mean1 && mean16 < 0.35 * mean1,
        "the error shrinks with samples: {mean1} at 1, {mean4} at 4, {mean16} at 16"
    );
    // Sixteen points cannot tell coverages closer than 1/16 apart, and a point set's uneven
    // spread across a pixel adds to that.
    assert!(
        worst16 < 0.25 * (surface - background).abs(),
        "16 samples are within a quarter of the step at every edge pixel: {worst16}"
    );
}

fn draw(r: &Renderer, target: &FrameTarget, mesh: &GpuMesh, view: View) -> Image {
    let scene = Scene {
        meshes: &[mesh],
        view,
        viewport: [0, 0, SIZE, SIZE],
    };
    r.draw_frame(target, &scene, &mut r.ui_painter(), &UiLayer::default())
        .expect("draws");
    r.read_frame(target).expect("reads")
}

#[test]
fn a_still_view_settles_and_a_moved_one_starts_over() {
    let r = lavapipe();
    let mesh = upload(r, &wedge());
    let still = view();
    let size = RenderSize {
        samples: 8,
        ..RenderSize::square(SIZE)
    };
    let target = r.frame_target(size).expect("target");
    let mut frames = Vec::new();
    for _ in 0..8 {
        assert!(!target.settled(), "not settled before every sample is in");
        frames.push(draw(r, &target, &mesh, still));
    }
    assert!(target.settled(), "settled after 8 frames");
    assert_eq!(
        frames[7],
        shot(r, &mesh, &still, 8),
        "the window's eighth frame is the 8-sample snapshot"
    );
    assert_ne!(frames[0], frames[7], "the samples changed the picture");
    assert_eq!(
        draw(r, &target, &mesh, still),
        frames[7],
        "once settled, frames stop changing"
    );
    let mut moved = still;
    moved.camera.orientation = Camera::turned(0.05, 0.0);
    let after = draw(r, &target, &mesh, moved);
    assert!(!target.settled(), "a moved camera starts again");
    assert_eq!(
        after,
        shot(r, &mesh, &moved, 1),
        "the first frame after the move is one unjittered sample of the new view"
    );
}

/// Samples a fraction of a pixel apart move smooth shading inside a surface by a level or two,
/// and smooth its silhouette by much more.
#[test]
fn samples_smooth_silhouettes_and_barely_touch_insides() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(1));
    let view = fitted(&mesh, Preset::Front, Shading::Clay, Wire::Off);
    let four = r
        .snapshot(
            &mesh,
            &view,
            RenderSize {
                width: 64,
                height: 64,
                samples: 4,
            },
        )
        .expect("4 accumulated samples render");
    let single = snap(r, &mesh, &view, 64);
    // Samples a fraction of a pixel apart move smooth shading inside the surface by a level or
    // two, and smooth its silhouette by much more (decision 0019).
    let (mut inside, mut edge) = (0u8, 0u8);
    for y in 1..63 {
        for x in 1..63 {
            let ring = (0..9).map(|k| single.pixel(x + k % 3 - 1, y + k / 3 - 1));
            let filled = ring.clone().all(|p| p != BACKGROUND);
            let empty = ring.clone().any(|p| p == BACKGROUND);
            let d = (0..3)
                .map(|k| four.pixel(x, y)[k].abs_diff(single.pixel(x, y)[k]))
                .max()
                .unwrap_or(0);
            if filled {
                inside = inside.max(d);
            } else if empty {
                edge = edge.max(d);
            }
        }
    }
    assert!(
        inside <= 3 && edge > 20,
        "accumulation moves the inside by at most 3 levels ({inside}) and edges by more ({edge})"
    );
    assert!(covered(&single) > 1000, "the sphere fills the image");
}

/// An edit that reaches the GPU starts the accumulation over, as a camera move does.
#[test]
fn an_edit_starts_over() {
    use gs_core::{ColumnSet, NodeId};
    let r = lavapipe();
    let mut store = crate::store::sphere_store(16, 6, 40);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let view = fitted(&mesh, Preset::Front, Shading::Clay, Wire::Off);
    let size = RenderSize {
        samples: 4,
        ..RenderSize::square(SIZE)
    };
    let target = r.frame_target(size).expect("target");
    for _ in 0..4 {
        draw(r, &target, &mesh, view);
    }
    assert!(target.settled(), "settled after 4 frames");
    let nodes: Vec<NodeId> = vec![0];
    store.write(&nodes, ColumnSet::POSITION).each(|m| {
        for p in m.positions() {
            *p = *p * 0.97;
        }
    });
    let dirty = store.take_dirty();
    r.update_store(&mut mesh, &store, &dirty);
    let after = draw(r, &target, &mesh, view);
    assert!(!target.settled(), "an edit starts again");
    assert_eq!(
        after,
        shot(r, &mesh, &view, 1),
        "the first frame after the edit is one unjittered sample"
    );
}
