//! The window surface for real, without showing anything on the desktop: a private Xvfb
//! display, an SDL window on it, a lavapipe device presenting to it, and the presented pixels
//! read back from the X server with the `import` tool of `ImageMagick`. Needs `Xvfb` and `import`.
//!
//! One program, two roles: run normally it starts Xvfb and runs itself again inside it (the
//! display is an environment variable, and a test cannot change its own environment safely).

use std::process::{Command, ExitCode};
use std::time::Duration;

use gs_platform::xvfb::{Xvfb, run_inside};
use gs_platform::{Input, Platform, WindowOptions};
use gs_render::{
    AdapterKind, Camera, FrameTarget, Preset, RenderSize, Renderer, Scene, Shading, UiImage,
    UiLayer, UiMesh, UiTexture, UiVertex, View, WindowSurface,
};
use libtest_mimic::{Arguments, Trial};

const ROLE: &str = "GS_SURFACE_TEST_DISPLAY";
/// A color nothing else on the display has.
const PAINT: [u8; 4] = [17, 99, 201, 255];

fn main() -> ExitCode {
    if let Ok(display) = std::env::var(ROLE) {
        inside(&display);
        return ExitCode::SUCCESS;
    }
    // One case, listed and run the way the standard harness does.
    let trial = Trial::test("the_surface_presents_exactly_what_was_drawn", || {
        outside();
        Ok(())
    });
    // Xvfb is Linux's: elsewhere this program has no case.
    let trials = cfg!(target_os = "linux")
        .then_some(trial)
        .into_iter()
        .collect();
    libtest_mimic::run(&Arguments::from_args(), trials).exit_code()
}

fn outside() {
    let xvfb = Xvfb::start(640, 480);
    let output = run_inside(&xvfb, ROLE, &xvfb.display);
    assert!(
        output.status.success(),
        "the run inside Xvfb failed: {}",
        output.status
    );
}

fn inside(display: &str) {
    let options = WindowOptions {
        title: "gs-render surface test".into(),
        width: 160,
        height: 100,
        driver: Some("x11".into()),
    };
    let mut platform = Platform::open(&options).unwrap_or_else(|e| panic!("{e}"));
    let (w, h) = platform.size_in_pixels();
    let (renderer, mut surface) =
        Renderer::for_window(platform.surface_handle(), Some(AdapterKind::Lavapipe), w, h)
            .unwrap_or_else(|e| panic!("{e}"));
    let info = surface.info();
    assert!(info.adapter.contains("llvmpipe"), "{info:?}");
    assert_eq!(
        info.present_mode, "Mailbox",
        "the lowest-latency mode offered"
    );
    assert_eq!(info.frame_latency, 1, "one frame queued at most");
    assert_eq!(info.size, [w, h], "the size asked for");
    let target = presented(&renderer, &mut surface, w, h);
    let painted = renderer
        .read_frame(&target)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(painted.pixel(w / 2, h / 2), PAINT, "the frame itself");
    wait_for_paint(display, w * h);

    // Resizing reconfigures the surface; the next frame presents at the new size.
    surface.resize(&renderer, w / 2, h / 2);
    assert_eq!(surface.info().size, [w / 2, h / 2], "resized");
    presented(&renderer, &mut surface, w / 2, h / 2);
    let pumped = platform.pump(Duration::from_millis(10));
    assert!(
        !pumped.inputs.contains(&Input::Close),
        "nobody asked to close"
    );
}

/// Paints the whole frame one color and presents it.
fn presented(renderer: &Renderer, surface: &mut WindowSurface, w: u32, h: u32) -> FrameTarget {
    let size = RenderSize {
        width: w,
        height: h,
        samples: 1,
    };
    let target = renderer
        .frame_target(size)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut painter = renderer.ui_painter();
    let all = [0.0, 0.0, w as f32, h as f32];
    let v = |x, y| UiVertex {
        pos: [x, y],
        uv: [0.0, 0.0],
        color: PAINT,
    };
    let ui = UiLayer {
        textures: vec![UiTexture::Set {
            id: 0,
            at: None,
            image: UiImage {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
                smooth: false,
            },
        }],
        meshes: vec![UiMesh {
            clip: all,
            texture: 0,
            vertices: vec![
                v(0.0, 0.0),
                v(all[2], 0.0),
                v(all[2], all[3]),
                v(0.0, all[3]),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        }],
    };
    let scene = Scene {
        meshes: &[],
        view: View::new(Camera::preset(Preset::Front), Shading::Clay),
        viewport: [0, 0, w, h],
    };
    renderer
        .draw_frame(&target, &scene, &mut painter, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut presented = false;
    for _ in 0..20 {
        presented = surface.present(renderer, &target);
        if presented {
            break;
        }
    }
    assert!(presented, "the surface presented a frame");
    target
}

/// Reads the X server's screen until exactly `pixels` pixels have the paint color: the copy to
/// the surface passed every byte through unchanged.
fn wait_for_paint(display: &str, pixels: u32) {
    let mut found = 0;
    for _ in 0..50 {
        let png = Command::new("import")
            .args(["-display", display, "-window", "root", "png24:-"])
            .output()
            .unwrap_or_else(|e| panic!("this test needs ImageMagick's import: {e}"));
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png.stdout));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("{e}"));
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
        let frame = reader
            .next_frame(&mut buf)
            .unwrap_or_else(|e| panic!("{e}"));
        let step = frame.color_type.samples();
        found = buf[..frame.buffer_size()]
            .chunks(step)
            .filter(|p| p[..3] == PAINT[..3])
            .count() as u32;
        if found == pixels {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("{found} pixels of the paint color on screen, expected {pixels}");
}
