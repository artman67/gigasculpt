//! Test support, behind the `xvfb` feature: a private Xvfb display that is ready to take
//! connections, and running a program inside it. The one home for this; every test program that
//! needs a real X11 window uses it (gs-platform's own `x11` test includes this file by path).
//!
//! Under heavy parallel test load SDL has now and then refused a freshly started display with
//! "x11 not available" (2026-10-10). Xvfb announces its display number once it listens, so the
//! start waits until the display's socket really accepts a connection, and `run_inside` runs its
//! program once more on that exact error. The underlying cause was not found.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use gs_protocol::UnixStream;

/// SDL's message when it cannot open the X display.
const REFUSED: &str = "x11 not available";

/// A running private Xvfb, killed when dropped.
pub struct Xvfb {
    child: Child,
    /// The display name, such as `:3`, for `DISPLAY`.
    pub display: String,
}

impl Xvfb {
    /// Starts Xvfb on a free display with one `width` x `height` x 24 screen and no TCP, and
    /// waits up to a minute until the display's socket accepts a connection.
    pub fn start(width: u32, height: u32) -> Xvfb {
        let screen = format!("{width}x{height}x24");
        let mut child = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                &screen,
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("this test needs Xvfb: {e}"));
        let stdout = child
            .stdout
            .take()
            .unwrap_or_else(|| panic!("no Xvfb stdout"));
        let mut number = String::new();
        BufReader::new(stdout)
            .read_line(&mut number)
            .unwrap_or_else(|e| panic!("Xvfb: {e}"));
        let number = number.trim().to_string();
        let xvfb = Xvfb {
            child,
            display: format!(":{number}"),
        };
        let socket = format!("/tmp/.X11-unix/X{number}");
        // About a minute in 20 ms steps (no clock reads outside the engine's clock module).
        for _ in 0..3000 {
            if UnixStream::connect(&socket).is_ok() {
                return xvfb;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "Xvfb {} never accepted a connection at {socket}",
            xvfb.display
        );
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// Runs this test program again inside `xvfb`'s display, with the environment variable `role`
/// set to `value` so it knows to play the inside part, passing its output through to this
/// program's own. If it fails with SDL's "x11 not available", it runs once more. Returns the
/// last run's output.
pub fn run_inside(xvfb: &Xvfb, role: &str, value: &str) -> Output {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let run = || {
        let output = Command::new(&exe)
            .env(role, value)
            .env("DISPLAY", &xvfb.display)
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap_or_else(|e| panic!("cannot start the run inside Xvfb: {e}"));
        std::io::stdout().write_all(&output.stdout).ok();
        std::io::stderr().write_all(&output.stderr).ok();
        output
    };
    let first = run();
    if first.status.success() || !String::from_utf8_lossy(&first.stderr).contains(REFUSED) {
        return first;
    }
    eprintln!("SDL refused the display ({REFUSED}); running once more");
    run()
}
