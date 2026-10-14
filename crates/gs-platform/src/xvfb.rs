//! Test support, behind the `xvfb` feature: a private Xvfb display that is ready to take
//! connections, and running a program inside it. The one home for this; every test program that
//! needs a real X11 window uses it (gs-platform's own `x11` test includes this file by path).
//!
//! **The display number is picked here, and a test never lets an X server choose a low one.**
//! Given `-displayfd` and no number, Xvfb tries :0, :1 and up and takes the first whose sockets
//! it can bind. `-displayfd` also turns its lock files off (xorg-server `os/utils.c`), and its
//! listener unlinks a display's path socket before binding its own (xtrans
//! `SocketUNIXCreateListener`); on Linux only an abstract socket of the same name stops it. A
//! desktop X server that listens on the path socket alone has none, so every Xvfb started that
//! way takes it: on 2026-10-13 one took the desktop's :0, and stopped with SIGKILL it left a
//! dead socket in its place. So the helper picks numbers from `BASE` up, starting at a place
//! spread by process id so parallel test programs rarely try the same one, skips any number whose
//! lock file or socket exists, and passes the number as `:<n>` without `-displayfd`. With a
//! number given (and no `-displayfd`, which would turn locking off again), Xvfb takes the
//! display's lock file before it makes any socket and exits if a running server holds it; the
//! helper then tries the next. A display is ready when its socket accepts a connection and its
//! lock file names this helper's Xvfb, never another server's.
//!
//! Dropping the helper stops Xvfb with SIGTERM, on which it removes its socket and lock file
//! (SIGKILL would leave both). Under heavy parallel test load SDL has now and then refused a
//! fresh display with "x11 not available" (2026-10-10, cause not found), so `run_inside` runs its
//! program once more on that exact error. With the number picked here, 872 displays with an SDL
//! window each, started 24 and 32 at a time on a loaded machine, had no refusal (2026-10-14); the
//! old way was not run again to compare, as it takes the desktop's display.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use gs_protocol::UnixStream;

/// SDL's message when it cannot open the X display.
const REFUSED: &str = "x11 not available";
/// The lowest display number the helper uses, well above any desktop's.
pub const BASE: u32 = 100;
/// How many display numbers, from `BASE` up, the helper uses.
pub const SPAN: u32 = 1000;
/// How many free numbers one start tries before it gives up.
const TRIES: usize = 20;
/// The step of every wait here (no clock reads outside the engine's clock module).
const STEP: Duration = Duration::from_millis(20);
/// About a minute in steps: how long a start waits for its display.
const START_STEPS: u32 = 3000;
/// About ten seconds in steps: how long a drop waits for Xvfb to end after SIGTERM.
const STOP_STEPS: u32 = 500;

/// A running private Xvfb, stopped when dropped.
pub struct Xvfb {
    child: Child,
    /// The display number, at or above [`BASE`].
    pub number: u32,
    /// The display name, such as `:103`, for `DISPLAY`.
    pub display: String,
}

impl Xvfb {
    /// Starts Xvfb with one `width` x `height` x 24 screen and no TCP on a free display number
    /// at or above [`BASE`], first trying this process's [`spread`], and waits up to a minute
    /// until the display's socket accepts a connection.
    pub fn start(width: u32, height: u32) -> Xvfb {
        Xvfb::start_at(spread(), width, height)
    }

    /// [`Xvfb::start`], trying the free [`numbers`] from `from` up.
    pub fn start_at(from: u32, width: u32, height: u32) -> Xvfb {
        let mut steps = START_STEPS;
        for number in numbers(from).filter(|&n| !taken(n)).take(TRIES) {
            if let Some(xvfb) = Xvfb::on(number, width, height, &mut steps) {
                return xvfb;
            }
            if steps == 0 {
                break;
            }
        }
        panic!("no Xvfb display from :{from} up took a connection within a minute (needs Xvfb)");
    }

    /// Starts Xvfb on display `number` and waits, using up `steps`, until it is ready. `None`
    /// if it ends first (the number was taken in the meantime) or the steps run out.
    fn on(number: u32, width: u32, height: u32, steps: &mut u32) -> Option<Xvfb> {
        let screen = format!("{width}x{height}x24");
        let display = format!(":{number}");
        let child = Command::new("Xvfb")
            .args([&display, "-screen", "0", &screen, "-nolisten", "tcp"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("this test needs Xvfb: {e}"));
        let mut xvfb = Xvfb {
            child,
            number,
            display,
        };
        while *steps > 0 {
            *steps -= 1;
            if !xvfb.running() {
                return None;
            }
            if UnixStream::connect(socket(number)).is_ok() && xvfb.owns_lock() {
                return Some(xvfb);
            }
            std::thread::sleep(STEP);
        }
        None
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Whether the display's lock file names this Xvfb.
    fn owns_lock(&self) -> bool {
        let pid = std::fs::read_to_string(lock(self.number)).unwrap_or_default();
        pid.trim().parse() == Ok(self.child.id())
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        // SIGTERM, on which Xvfb removes its socket and lock file. Never to an ended child,
        // whose process id may already belong to another process.
        if self.running() {
            let pid = self.child.id().to_string();
            Command::new("kill").args(["-TERM", &pid]).status().ok();
        }
        for _ in 0..STOP_STEPS {
            if !self.running() {
                return;
            }
            std::thread::sleep(STEP);
        }
        // It ignored SIGTERM: kill it and remove what it left, only if the display is its own.
        let own = self.owns_lock();
        self.child.kill().ok();
        self.child.wait().ok();
        if own {
            std::fs::remove_file(socket(self.number)).ok();
            std::fs::remove_file(lock(self.number)).ok();
        }
    }
}

/// Where this process starts looking: a number from [`BASE`] up, spread by process id so
/// parallel test programs rarely try the same one first.
pub fn spread() -> u32 {
    BASE + std::process::id() % SPAN
}

/// Every display number the helper uses, once each: from `from` up, then round from [`BASE`].
/// Never one below [`BASE`].
pub fn numbers(from: u32) -> impl Iterator<Item = u32> {
    let offset = from.saturating_sub(BASE);
    (0..SPAN).map(move |i| BASE + (offset + i) % SPAN)
}

/// Whether display `number` is in use or was left behind: its lock file or its socket exists.
pub fn taken(number: u32) -> bool {
    lock(number).exists() || socket(number).exists()
}

/// The lock file of display `number`, which holds its X server's process id.
pub fn lock(number: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/.X{number}-lock"))
}

/// The path socket of display `number`.
pub fn socket(number: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/.X11-unix/X{number}"))
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
