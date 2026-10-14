//! The real window, end to end, without showing anything on the desktop: `gigasculpt window`
//! (this crate's `app`) inside a private Xvfb display on lavapipe, with an agent connected
//! through the socket. The agent injects pen samples, reads `ui.describe`, opens the palette,
//! clicks palette rows and the pause toggle with the pen (so the window itself runs commands),
//! opens a Reference View beside the 3D view and checks the screenshot shows both,
//! takes screenshots, and asks the window to close the way the desktop does (SIGTERM, which
//! SDL turns into a quit). A second window closes by itself after `--close-after`. Needs
//! `Xvfb` and `kill`.
//!
//! One program, two roles: run normally it starts Xvfb and runs itself again inside it.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus};
use std::time::Duration;

use gs_platform::xvfb::Xvfb;
use gs_protocol::UnixStream;
use libtest_mimic::{Arguments, Trial};
use serde_json::{Value, json};

/// Which window to be, when run inside Xvfb: `driven` or `brief`.
const ROLE: &str = "GS_WINDOW_TEST_ROLE";
/// The folder the windows put their sockets in, chosen by the outer run.
const SOCKETS: &str = "GS_WINDOW_TEST_SOCKETS";

fn main() -> ExitCode {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("window-e2e");
    let sockets = || PathBuf::from(std::env::var_os(SOCKETS).unwrap_or_default());
    match std::env::var(ROLE).as_deref() {
        Ok("driven") => gs_ui::app::main(&window_args(&dir, &sockets(), "driven", "600")),
        Ok(_) => gs_ui::app::main(&window_args(&dir, &sockets(), "brief", "1.5")),
        // One case, listed and run the way the standard harness does.
        Err(_) => {
            let trial = Trial::test("the_window_runs_end_to_end_under_xvfb", move || {
                // Under $XDG_RUNTIME_DIR, not the target folder: mutation testing copies the
                // workspace deep under target/, past the 108-byte limit of a socket path.
                let base = std::env::var_os("XDG_RUNTIME_DIR")
                    .map_or_else(std::env::temp_dir, PathBuf::from);
                let sockets = Gone(base.join(format!("gs-window-{}", std::process::id())));
                outside(&dir, &sockets.0);
                Ok(())
            });
            // Xvfb is Linux's: elsewhere this program has no case.
            let trials = cfg!(target_os = "linux")
                .then_some(trial)
                .into_iter()
                .collect();
            libtest_mimic::run(&Arguments::from_args(), trials).exit_code()
        }
    }
}

/// A folder removed when the test ends, however it ends.
struct Gone(PathBuf);

impl Drop for Gone {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn window_args(dir: &Path, sockets: &Path, session: &str, close_after: &str) -> Vec<OsString> {
    let socket = sockets
        .join(format!("{session}.sock"))
        .display()
        .to_string();
    let log = dir.join(format!("{session}.jsonl")).display().to_string();
    let config = dir.join("config").display().to_string();
    let args = [
        "gigasculpt",
        "window",
        "--session",
        session,
        "--driver",
        "x11",
        "--adapter",
        "lavapipe",
        "--socket",
        &socket,
        "--log",
        &log,
        "--config",
        &config,
        "--width",
        "900",
        "--height",
        "600",
        "--close-after",
        close_after,
        // Autosave writes into the home folder's state folder; a test writes only its own.
        "--no-autosave",
    ];
    args.iter().map(OsString::from).collect()
}

/// A child process, killed if the test fails before it ends.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

impl Kill {
    /// Waits up to `seconds` for the process to end.
    fn ends_within(&mut self, seconds: u64) -> ExitStatus {
        for _ in 0..seconds * 20 {
            if let Some(status) = self.0.try_wait().unwrap_or_else(|e| panic!("{e}")) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the window was still open after {seconds} s");
    }
}

fn window(role: &str, display: &str, sockets: &Path) -> Kill {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let child = Command::new(exe)
        .env(ROLE, role)
        .env(SOCKETS, sockets)
        .env("DISPLAY", display)
        .env_remove("WAYLAND_DISPLAY")
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    Kill(child)
}

fn outside(dir: &Path, sockets: &Path) {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
    std::fs::remove_file(dir.join("config").join("keymap.toml")).ok();
    std::fs::remove_file(dir.join("driven.jsonl")).ok();
    let xvfb = Xvfb::start(1280, 800);
    let display = xvfb.display.clone();
    let mut driven = window("driven", &display, sockets);
    let socket = sockets.join("driven.sock");
    let mut agent = connect(&socket, &mut driven, &|| {
        window("driven", &display, sockets)
    });
    drive(&mut agent, dir);
    reference_shows_beside_main(&mut agent);
    window_runs_its_own_commands(&mut agent);
    let pid = driven.0.id().to_string();
    let killed = Command::new("kill").args(["-TERM", &pid]).status();
    assert!(killed.is_ok_and(|s| s.success()), "SIGTERM sent");
    let status = driven.ends_within(60);
    assert!(status.success(), "the window closed cleanly: {status}");
    assert!(!socket.exists(), "and took its socket with it");
    let log = std::fs::read_to_string(dir.join("driven.jsonl")).unwrap_or_default();
    assert!(
        log.contains("\"pen.input\""),
        "the session log records pen input"
    );
    // A brief window that ends in failure starts once more, as `connect` restarts one: SDL now
    // and then refuses the fresh display ("x11 not available").
    let brief = || window("brief", &display, sockets).ends_within(60);
    let mut status = brief();
    if !status.success() {
        eprintln!("the brief window ended ({status}); starting it again");
        status = brief();
    }
    assert!(
        status.success(),
        "--close-after closed it by itself: {status}"
    );
}

/// Connects to the window's socket. Starting a window on lavapipe can take well over 30 s on a
/// loaded machine, so this waits up to 3 minutes while the window lives. If the window ends
/// before opening the socket, `restart` starts it once more: under heavy parallel test load
/// SDL has now and then refused the fresh Xvfb display ("x11 not available"; cause not found,
/// 2026-10-10). A second early end fails the test with its exit status.
fn connect(socket: &Path, window: &mut Kill, restart: &dyn Fn() -> Kill) -> Agent {
    let mut restarted = false;
    for _ in 0..3600 {
        if let Some(status) = window.0.try_wait().unwrap_or_else(|e| panic!("{e}")) {
            assert!(
                !restarted,
                "the window ended ({status}) before opening its socket, twice"
            );
            eprintln!("the window ended ({status}) before opening its socket; starting it again");
            *window = restart();
            restarted = true;
            continue;
        }
        if let Ok(stream) = UnixStream::connect(socket) {
            let writer = stream.try_clone().unwrap_or_else(|e| panic!("{e}"));
            let mut agent = Agent {
                reader: BufReader::new(stream),
                writer,
                id: 0,
            };
            agent.rpc("engine.hello", &json!({ "client": "e2e", "role": "agent" }));
            return agent;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the window never opened its socket at {}", socket.display());
}

struct Agent {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    id: u64,
}

impl Agent {
    fn rpc(&mut self, method: &str, params: &Value) -> Value {
        self.id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        writeln!(self.writer, "{line}").unwrap_or_else(|e| panic!("write: {e}"));
        let mut reply = String::new();
        self.reader
            .read_line(&mut reply)
            .unwrap_or_else(|e| panic!("read: {e}"));
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("bad reply {reply:?}: {e}"))
    }

    /// The command's outcome: `ok` or `error`.
    fn call(&mut self, command: &str, args: &Value) -> Value {
        self.rpc("cmd.run", &json!({ "command": command, "args": args }))["result"].clone()
    }

    fn ok(&mut self, command: &str, args: &Value) -> Value {
        let outcome = self.call(command, args);
        assert!(!outcome["ok"].is_null(), "{command} failed: {outcome}");
        outcome["ok"]["result"].clone()
    }

    /// Taps the pen at the center of a rectangle in pixels: hover, touch, lift, a frame apart.
    fn tap(&mut self, rect: &Value) {
        let f = |i: usize| rect[i].as_f64().unwrap_or_default();
        let (x, y) = (f64::midpoint(f(0), f(2)), f64::midpoint(f(1), f(3)));
        for contact in [false, true, false] {
            let samples = json!({ "samples": [{ "x": x, "y": y, "contact": contact }] });
            self.ok("pen.input", &samples);
            self.ok("ui.describe", &json!({}));
        }
    }

    /// The labels in the status bar.
    fn status(&mut self) -> Vec<String> {
        let d = self.ok("ui.describe", &json!({}));
        d["panels"][0]["widgets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w["label"].as_str().map(String::from))
            .collect()
    }
}

/// The rectangle of the widget labelled `label` in panel `panel`, if it shows.
fn find(d: &Value, panel: &str, label: &str) -> Option<Value> {
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let found = panels.iter().find(|p| p["id"] == panel).and_then(|p| {
        let widgets = p["widgets"].as_array()?;
        widgets.iter().find(|w| w["label"] == label).cloned()
    });
    found.map(|w| w["rect"].clone())
}

/// The rectangle of the widget labelled `label` in panel `panel`.
fn widget(d: &Value, panel: &str, label: &str) -> Value {
    find(d, panel, label).unwrap_or_else(|| panic!("no {label} in {panel}: {d}"))
}

/// A base64 PNG screenshot's RGBA rows.
fn decode_png(base64: &str) -> Vec<u8> {
    use base64::Engine as _;
    let bytes = (base64::engine::general_purpose::STANDARD.decode(base64))
        .unwrap_or_else(|e| panic!("{e}"));
    let image = gs_render::Image::from_png(&bytes).unwrap_or_else(|e| panic!("{e}"));
    image.rgba
}

/// In a screenshot `width` pixels wide, the share of the pixels in `rect` that are red, and the
/// share that differ from the rectangle's top-left pixel (its background).
fn shares(rgba: &[u8], width: usize, rect: &Value) -> (f64, f64) {
    let r = |i: usize| rect[i].as_f64().unwrap_or_default().round() as usize;
    let at = |x: usize, y: usize| &rgba[(y * width + x) * 4..][..3];
    let corner = at(r(0) + 2, r(1) + 2).to_vec();
    let (mut red, mut drawn, mut all) = (0_u32, 0_u32, 0_u32);
    for y in (r(1) + 2..r(3).saturating_sub(2)).step_by(2) {
        for x in (r(0) + 2..r(2).saturating_sub(2)).step_by(2) {
            let p = at(x, y);
            let [pr, pg, pb] = [p[0], p[1], p[2]].map(i32::from);
            red += u32::from(pr > pg + 40 && pr > pb + 40);
            let apart: i32 = (p.iter().zip(&corner))
                .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
                .sum();
            drawn += u32::from(apart > 30);
            all += 1;
        }
    }
    let all = f64::from(all.max(1));
    (f64::from(red) / all, f64::from(drawn) / all)
}

/// The agent opens a Reference View of the sphere in red wax, docked beside the 3D view: the
/// screenshot shows both views, the gray sculpt in one and the red reference in the other. Then
/// it closes the reference.
fn reference_shows_beside_main(agent: &mut Agent) {
    let open = json!({ "parts": "sphere", "look": { "matcap": "red-wax" } });
    agent.ok("reference.open", &open);
    // The window places the picture and frames the reference in it over its next frames, which a
    // busy machine delays: wait up to three minutes (it ends as soon as both show).
    let mut seen = String::new();
    let both = (0..4500).any(|_| {
        std::thread::sleep(Duration::from_millis(40));
        let d = agent.ok("ui.describe", &json!({}));
        let Some(picture) = find(&d, "reference", "ref:1") else {
            return false;
        };
        let shot = agent.ok("ui.screenshot", &json!({}));
        let width = shot["width"].as_u64().unwrap_or_default() as usize;
        let rgba = decode_png(shot["png_base64"].as_str().unwrap_or_default());
        let main = shares(&rgba, width, &d["viewport"]["rect"]);
        let reference = shares(&rgba, width, &picture);
        seen = format!("main (red, drawn) {main:?}, reference {reference:?}");
        // The orientation figure's red axis is under 1% of the 3D view.
        main.0 < 0.03 && main.1 > 0.05 && reference.0 > 0.1
    });
    assert!(
        both,
        "the sculpt in gray and the reference in red wax: {seen}"
    );
    let state = agent.ok("session.state", &json!({}));
    assert_eq!(state["reference"], json!("ref:1"), "{state}");
    agent.ok("reference.close", &json!({}));
}

fn drive(agent: &mut Agent, dir: &Path) {
    agent.ok("primitive.sphere", &json!({ "subdivisions": 10 }));
    let pen = json!({ "samples": [
        { "t_ns": 1_000_000, "x": 500.0, "y": 300.0 },
        { "t_ns": 5_000_000, "x": 504.0, "y": 300.0, "contact": true, "pressure": 0.4 }
    ] });
    let taken = agent.ok("pen.input", &pen)["window"].clone();
    assert_eq!(taken, json!(true), "the window took the samples");
    // The window times a frame only once it has drawn one, which a busy machine delays: ask
    // again for up to three minutes (it ends as soon as a frame is timed).
    let mut d = agent.ok("ui.describe", &json!({}));
    for _ in 0..3600 {
        if d["perf"]["frame"]["count"].as_u64().unwrap_or_default() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        d = agent.ok("ui.describe", &json!({}));
    }
    assert_eq!(d["cursor"]["x"], json!(504.0), "the window's cursor");
    assert_eq!(
        d["viewport"]["parts"],
        json!(["sphere"]),
        "the view draws the sphere"
    );
    assert_eq!(d["size"], json!([900, 600]), "x11 at density 1");
    let perf = &d["perf"];
    let adapter = perf["adapter"].as_str().unwrap_or_default();
    assert!(adapter.contains("llvmpipe"), "{perf}");
    assert_eq!(
        perf["present_mode"],
        json!("Mailbox"),
        "lowest latency offered"
    );
    let frame_ms = perf["frame"]["p50_ms"].as_f64().unwrap_or_default();
    assert!(
        (0.01..1000.0).contains(&frame_ms),
        "a frame takes a sane time: {perf}"
    );
    // The session's name ends the status line, which a 900-wide window leaves out (decision
    // 0016, section 17, item 10); tests/ui/palette.rs reads it in a window with room.
    // An idle window stops drawing: after a change it adds jittered samples until its picture
    // has settled (16 of them, decision 0019), then draws nothing more. Each look below is one
    // frame for the call it reports.
    let frames = |agent: &mut Agent| {
        let d = agent.ok("ui.describe", &json!({}));
        d["perf"]["frames"].as_u64().unwrap_or_default()
    };
    let mut drawn = u64::MAX;
    for _ in 0..25 {
        let before = frames(agent);
        std::thread::sleep(Duration::from_millis(400));
        drawn = frames(agent) - before;
        if drawn <= 3 {
            break;
        }
    }
    assert!(
        drawn <= 3,
        "an idle window stops drawing once its picture has settled: {drawn} frames in 400 ms"
    );
    let shot = agent.ok("ui.screenshot", &json!({}));
    let size = (shot["width"].clone(), shot["height"].clone());
    assert_eq!(size, (json!(900), json!(600)), "the whole window");
    let png = shot["png_base64"].as_str().unwrap_or_default();
    assert!(png.starts_with("iVBORw0KGgo"), "a PNG, as base64");
    let keymap = std::fs::read_to_string(dir.join("config").join("keymap.toml"));
    assert!(
        keymap.is_ok_and(|k| k.ends_with("version = 2\n")),
        "the keymap file was written, with no changes"
    );
    assert!(
        dir.join("config").join("layout.toml").exists(),
        "and the layout file"
    );
}

/// The agent taps the interface with the pen; the window runs what was tapped as the human.
fn window_runs_its_own_commands(agent: &mut Agent) {
    // Nothing was undone, so redo has nothing to do and fails.
    let open = agent.ok("ui.palette", &json!({ "query": "history.redo" }));
    assert_eq!(open["matches"][0], json!("history.redo"), "palette search");
    let d = agent.ok("ui.describe", &json!({}));
    agent.tap(&widget(&d, "palette", "history.redo"));
    // The window answers taps on its next frames, which a busy machine delays: wait up to three
    // minutes for each change (it ends as soon as the change shows; one minute ran out at a load
    // average of 351).
    let shown = (0..9000).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        agent.status().iter().any(|l| l.starts_with("Problem: E_"))
    });
    assert!(
        shown,
        "a failed command shows in the status bar: {:?}",
        agent.status()
    );
    let d = agent.ok("ui.describe", &json!({}));
    agent.tap(&widget(&d, "status", "Pause agents"));
    let paused = (0..9000).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        agent.call("primitive.cube", &json!({}))["error"]["code"] == "E_PAUSED"
    });
    assert!(paused, "tapping Pause agents paused this agent");
}
