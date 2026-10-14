//! The window on a real display server, without showing anything on the desktop: SDL's X11
//! driver inside a private Xvfb display. There the window has native handles to give a GPU
//! surface, on its own thread only. Also the private display helper itself (`src/xvfb.rs`):
//! it picks display numbers from its base up, never one already in use. Needs `Xvfb`.
//!
//! One program, two roles: run normally it starts Xvfb and runs itself again inside it (the
//! display is an environment variable, and a test cannot change its own environment safely).

use std::process::ExitCode;
use std::sync::Barrier;
use std::time::Duration;

use gs_platform::{Platform, WindowOptions};
use gs_protocol::UnixStream;
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
    // Each case listed and run the way the standard harness does.
    let trials = vec![
        Trial::test("an_x11_window_has_native_handles_on_its_thread", || {
            outside();
            Ok(())
        }),
        Trial::test(
            "two_displays_started_at_once_get_numbers_of_their_own",
            || {
                two_at_once();
                Ok(())
            },
        ),
        Trial::test("a_number_with_a_lock_file_is_skipped", || {
            locked_number_is_skipped();
            Ok(())
        }),
        Trial::test("a_dropped_display_leaves_no_socket_or_lock", || {
            dropped_display_leaves_nothing();
            Ok(())
        }),
    ];
    // Xvfb is Linux's: elsewhere this program has no case.
    let trials = if cfg!(target_os = "linux") {
        trials
    } else {
        Vec::new()
    };
    libtest_mimic::run(&Arguments::from_args(), trials).exit_code()
}

/// Two displays started at the same moment, both trying the same number first, get different
/// numbers, both at or above the base, and both take connections.
fn two_at_once() {
    let first = xvfb::spread();
    let together = Barrier::new(2);
    let start = || {
        together.wait();
        xvfb::Xvfb::start_at(first, 64, 64)
    };
    let (a, b) = std::thread::scope(|s| {
        let (a, b) = (s.spawn(start), s.spawn(start));
        let join = |h: std::thread::ScopedJoinHandle<'_, xvfb::Xvfb>| {
            h.join().unwrap_or_else(|_| panic!("a start panicked"))
        };
        (join(a), join(b))
    });
    assert_ne!(a.number, b.number, "two displays, two numbers");
    for x in [&a, &b] {
        assert!(
            (xvfb::BASE..xvfb::BASE + xvfb::SPAN).contains(&x.number),
            "{} is within the helper's numbers, from :{} up",
            x.display,
            xvfb::BASE
        );
        assert_eq!(x.display, format!(":{}", x.number), "its display name");
        assert!(
            UnixStream::connect(xvfb::socket(x.number)).is_ok(),
            "{} takes connections",
            x.display
        );
    }
}

/// A free number this test locks itself, with an empty lock file (which Xvfb alone would take
/// for a stale lock and replace): the helper skips it and leaves the file alone.
fn locked_number_is_skipped() {
    let mine = OwnLock::take();
    let x = xvfb::Xvfb::start_at(mine.0, 64, 64);
    assert_ne!(
        x.number, mine.0,
        "{} is not the locked :{}",
        x.display, mine.0
    );
    assert_eq!(
        std::fs::read(xvfb::lock(mine.0)).ok(),
        Some(Vec::new()),
        "the lock file of :{} is untouched",
        mine.0
    );
}

/// Dropping the helper stops its Xvfb, and the display's socket and lock file are gone with it,
/// so the number is free again and no dead socket is left behind.
fn dropped_display_leaves_nothing() {
    // Away from where this program's other cases start (a plain `cargo test` runs them in one
    // process), so none of them takes the number again right after the drop.
    let x = xvfb::Xvfb::start_at(xvfb::spread() + xvfb::SPAN / 2, 64, 64);
    let number = x.number;
    assert!(
        xvfb::lock(number).exists() && xvfb::socket(number).exists(),
        "a running :{number} has its lock file and socket"
    );
    drop(x);
    assert!(
        !xvfb::lock(number).exists(),
        "the lock of :{number} is gone"
    );
    assert!(
        !xvfb::socket(number).exists(),
        "the socket of :{number} is gone"
    );
}

/// An empty lock file this test made for a free display number at or above the base, removed
/// when dropped.
struct OwnLock(u32);

impl OwnLock {
    fn take() -> OwnLock {
        // Away from where this program's other cases start, as above.
        let number = xvfb::numbers(xvfb::spread() + xvfb::SPAN / 4)
            .find(|&n| !xvfb::taken(n))
            .unwrap_or_else(|| panic!("no free display number"));
        std::fs::File::create_new(xvfb::lock(number))
            .unwrap_or_else(|e| panic!("cannot lock :{number}: {e}"));
        OwnLock(number)
    }
}

impl Drop for OwnLock {
    fn drop(&mut self) {
        std::fs::remove_file(xvfb::lock(self.0)).ok();
    }
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
