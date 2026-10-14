//! Zebra sensitivity (ARCHITECTURE 10.3, docs/spikes/golden.md): on the spike's scene, a
//! slightly weaker stroke moves the zebra image far past the cross-renderer threshold (FLIP max
//! 0.30), while rendering the same stroke on another GPU stays under it.

use crate::common::{diff, flip, hardware, lavapipe, snap, upload};
use crate::scene;
use gs_render::{Camera, Image, Occlusion, Preset, Renderer, Shading, View};

/// The cross-renderer pass limit for zebra images.
const ZEBRA_LIMIT: f32 = 0.30;

/// The spike's camera: eye at (2.2, 1.4, 2.8) looking at the origin, 45 degree field of view.
fn spike_camera() -> Camera {
    let distance = (2.2f32 * 2.2 + 1.4 * 1.4 + 2.8 * 2.8).sqrt();
    Camera {
        orientation: Camera::turned(libm::atan2f(2.2, 2.8), libm::asinf(1.4 / distance)),
        distance,
        ..Camera::preset(Preset::Front)
    }
}

/// The spike's icosphere (81,920 triangles) with a stroke `depth` deep, at 512 x 512.
fn render(r: &Renderer, depth: f32, shading: Shading) -> Image {
    render_with(r, depth, View::new(spike_camera(), shading))
}

fn render_with(r: &Renderer, depth: f32, view: View) -> Image {
    let mut sphere = scene::icosphere(6);
    scene::dent(&mut sphere, depth);
    let mesh = upload(r, &sphere);
    snap(r, &mesh, &view, 512)
}

#[test]
fn zebra_sees_a_small_dent_far_above_gpu_noise() {
    let r = lavapipe();
    let base = render(r, 0.12, Shading::Zebra);
    for (label, depth) in [("4% weaker", 0.1152), ("0.5% weaker", 0.1194)] {
        let (worst, _) = flip(&base, &render(r, depth, Shading::Zebra));
        assert!(
            worst > ZEBRA_LIMIT,
            "zebra, {label} stroke: FLIP max {worst} fails the cross-renderer rule (spike: 0.98 and 0.67)"
        );
    }
    let (clay_worst, _) = flip(
        &render(r, 0.12, Shading::Clay),
        &render(r, 0.1152, Shading::Clay),
    );
    assert!(
        clay_worst < ZEBRA_LIMIT / 2.0,
        "plain clay barely sees the 4% stroke (FLIP max {clay_worst}; spike: 0.061)"
    );
    for (kind, gpu) in hardware() {
        let (worst, _) = flip(&base, &render(gpu, 0.12, Shading::Zebra));
        assert!(
            worst <= ZEBRA_LIMIT,
            "{kind:?} renders the same stroke within the zebra rule (FLIP max {worst}; spike noise 0.10)"
        );
        // Without occlusion (whose quality is the adapter's tier) clay differs from lavapipe by
        // rounding only. (On the Radeon those 1-level differences still put about 100 dark
        // pixels at FLIP 0.052, just over the look rule's 0.05.)
        let plain = View {
            occlusion: Occlusion::OFF,
            ..View::new(spike_camera(), Shading::Clay)
        };
        let (levels, _) = diff(&render_with(r, 0.12, plain), &render_with(gpu, 0.12, plain));
        assert!(
            levels <= 1,
            "{kind:?} renders clay within 1 level of lavapipe (max {levels})"
        );
    }
}
