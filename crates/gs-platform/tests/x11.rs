//! The window on a real display server, without showing anything on the desktop: SDL's X11
//! driver inside a private Xvfb display. There the window has native handles to give a GPU
//! surface, on its own thread only. Needs `Xvfb`.
//!
//! One program, two roles: run normally it starts Xvfb and runs itself again inside it (the
//! display is an environment variable, and a test cannot change its own environment safely).

use std::process::ExitCode;
use std::time::Duration;

use gs_platform::{Platform, WindowOptions};
use libtest_mimic::{Arguments, Trial};

#[path = "../src/xvfb.rs"]
mod xvfb;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

const ROLE: &str = "GS_PLATFORM_X11_TEST";

fn main() -> ExitCode {
    if std::env::var_os(ROLE).is_some() {
        inside();
        return ExitCode::SUCCESS;
    }
    // One case, listed and run the way the standard harness does.
    let trial = Trial::test("an_x11_window_has_native_handles_on_its_thread", || {
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
    let display = xvfb::Xvfb::start(640, 480);
    let output = xvfb::run_inside(&display, ROLE, "1");
    assert!(
        output.status.success(),
        "the run inside Xvfb failed: {}",
        output.status
    );
}

fn inside() {
    let options = WindowOptions {
        title: "gs-platform x11 test".into(),
        width: 300,
        height: 200,
        driver: Some("x11".into()),
    };
    let mut platform = Platform::open(&options).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(platform.driver(), "x11", "the driver asked for");
    let handle = platform.surface_handle();
    let window = handle.window_handle().map(|h| h.as_raw());
    assert!(
        matches!(window, Ok(RawWindowHandle::Xlib(_))),
        "an X11 window handle on the window's thread: {window:?}"
    );
    let display = handle.display_handle().map(|h| h.as_raw());
    assert!(
        matches!(display, Ok(RawDisplayHandle::Xlib(_))),
        "and its display: {display:?}"
    );
    let elsewhere = std::thread::spawn(move || handle.window_handle().is_err())
        .join()
        .unwrap_or_else(|_| panic!("thread"));
    assert!(elsewhere, "never on another thread");
    let pumped = platform.pump(Duration::from_millis(50));
    assert!(
        (pumped.density - 1.0).abs() < f32::EPSILON,
        "X11 has one pixel per point: {}",
        pumped.density
    );
    assert_eq!(platform.size_in_pixels(), (300, 200), "the size asked for");
}
