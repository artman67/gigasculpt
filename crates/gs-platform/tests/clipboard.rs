//! The system clipboard on a real display server: SDL's X11 driver inside a private Xvfb
//! display, so the desktop's clipboard is never touched. Needs `Xvfb`.
//!
//! One test, two roles, as in `x11.rs`: run normally it starts Xvfb and runs this program again
//! inside it (the display is an environment variable, and a test cannot change its own
//! environment safely). Its own program, because the run inside runs every test of the program.
#![cfg(target_os = "linux")]

use gs_platform::{Platform, WindowOptions};

#[path = "../src/xvfb.rs"]
mod xvfb;

const ROLE: &str = "GS_PLATFORM_CLIPBOARD_TEST";

#[test]
fn text_put_on_the_clipboard_reads_back_and_an_empty_one_reads_none() {
    if std::env::var_os(ROLE).is_some() {
        inside();
        return;
    }
    let display = xvfb::Xvfb::start(320, 200);
    let output = xvfb::run_inside(&display, ROLE, "1");
    assert!(
        output.status.success(),
        "the run inside Xvfb failed: {}",
        output.status
    );
}

fn inside() {
    let options = WindowOptions {
        title: "gs-platform clipboard test".into(),
        width: 200,
        height: 100,
        driver: Some("x11".into()),
    };
    let platform = Platform::open(&options).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(platform.driver(), "x11", "the driver asked for");
    assert_eq!(
        platform.clipboard_text(),
        None,
        "a new display's clipboard holds no text"
    );
    let path = "/models/head two.gsc";
    platform
        .set_clipboard_text(path)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        platform.clipboard_text().as_deref(),
        Some(path),
        "the text put there"
    );
    let lines = "first line\nsecond line";
    platform
        .set_clipboard_text(lines)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        platform.clipboard_text().as_deref(),
        Some(lines),
        "lines stay lines"
    );
}
