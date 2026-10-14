//! Recorded sessions, as the hidden holdout keeps them: a window log written with `--verify`
//! replays through `gs run` with every scene hash checked, and the files it opened are found by
//! their content wherever they were moved (`gs run --files`).

use std::fs;

use clap::Parser;
use gs_engine::server::Window;
use gs_protocol::{Outcome, PenSample, Request, RunParams};
use gs_ui::app::{WindowArgs, shell, start};
use gs_ui::{Host, Studio};
use serde_json::{Value, json};

use crate::support::Env;

/// Runs `command` as the human at the window, and its result; it must succeed.
fn human(window: &Window, command: &str, args: Value) -> Value {
    let request = Request::Run(RunParams::new(command, args));
    let response = window.execute(request);
    match response.map(|r| r.outcome) {
        Some(Outcome::Ok(ok)) => ok.result,
        Some(Outcome::Error(e)) => panic!("{command}: {}", e.message),
        None => panic!("{command}: the session stopped"),
    }
}

/// One frame's pen sample at (`x`, `y`), touching with `pressure` above 0, else hovering.
fn pen(window: &Window, t_ms: u64, x: f64, y: f64, pressure: f32) {
    let sample = PenSample {
        pressure,
        contact: pressure > 0.0,
        ..PenSample::hover(t_ms * 1_000_000, x, y)
    };
    human(window, "pen.input", json!({ "samples": [sample] }));
}

/// The log's call lines, oldest first.
fn records(log: &str) -> Vec<Value> {
    let text = fs::read_to_string(log).unwrap_or_else(|e| panic!("no log {log}: {e}"));
    (text.lines().skip(1))
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

/// A session in the window's own setup (`gigasculpt window --log PATH --verify`, with a window
/// that draws nothing) writes a hash on every line that changes the scene, and `gs run` replays
/// it into a headless session checking each one. The window's own lines replay too: the hover
/// as `pen.input`, the pen stroke as `brush.stroke`, a navigation gesture as `view.set`, and a
/// brush change keyed while the pen is still down, whose line comes before its stroke's.
pub fn window_logs_replay_with_every_hash_checked() {
    let env = Env::new();
    let log = env.dir.join("window.jsonl").display().to_string();
    let socket = env.dir.join("window.sock").display().to_string();
    let words = ["gigasculpt", "--session", "window", "--socket", &socket];
    let flags = ["--log", &log, "--verify", "--no-autosave"];
    let args =
        WindowArgs::try_parse_from(words.iter().chain(&flags)).unwrap_or_else(|e| panic!("{e}"));
    // Tall enough that, under the header's two rows at this width, the stroke across the
    // view's middle stays on the sphere (as gs-ui's sculpt rig).
    let host = Host::new(Studio::new(
        shell(&env.dir.join("config")),
        None,
        [640, 480],
        1.0,
    ));
    let server = start(&args, &host).unwrap_or_else(|e| panic!("{e}"));
    let window = server.window();
    human(&window, "primitive.sphere", json!({ "subdivisions": 16 }));
    let view = human(&window, "ui.describe", json!({}))["viewport"]["rect"].clone();
    let at = |i: usize| view[i].as_f64().unwrap_or_default();
    let (x, y) = (f64::midpoint(at(0), at(2)), f64::midpoint(at(1), at(3)));
    pen(&window, 0, x - 60.0, y, 0.0);
    for k in 0..=12_u32 {
        let f = f64::from(k) / 12.0;
        pen(
            &window,
            4 * u64::from(k + 1),
            x - 60.0 + 120.0 * f,
            y + 10.0 * f,
            0.6,
        );
        if k == 6 {
            human(&window, "brush.set", json!({ "size_step": 1 }));
        }
    }
    pen(&window, 60, x + 60.0, y + 10.0, 0.0);
    let turn = |end| json!({ "right_deg": 15, "gesture": 7, "end": end });
    human(&window, "view.orbit", turn(false));
    human(&window, "view.orbit", turn(true));
    let scene = human(&window, "mesh.hash", json!({}))["hash"].clone();
    window.close();
    server.wait().unwrap_or_else(|e| panic!("{e}"));

    let lines = records(&log);
    let commands: Vec<&str> = (lines.iter())
        .filter_map(|l| l["command"].as_str())
        .collect();
    for wanted in ["pen.input", "brush.set", "brush.stroke", "view.set"] {
        assert!(commands.contains(&wanted), "{wanted} in {commands:?}");
    }
    let line = |command: &str| lines.iter().find(|l| l["command"] == command).cloned();
    for edit in ["primitive.sphere", "brush.stroke"] {
        assert!(
            line(edit).is_some_and(|l| l["hash"].is_string()),
            "{edit} carries its hash"
        );
    }
    assert!(
        line("brush.set").is_some_and(|l| l.get("hash").is_none()),
        "keyed mid-stroke, its scene held half a stroke, which no replay holds"
    );
    let stroke = line("brush.stroke").unwrap_or_default();
    assert!(
        stroke["args"]["preset"].is_string(),
        "the first stroke carries its preset"
    );
    let last = lines.iter().rev().find_map(|l| l["hash"].as_str());
    assert_eq!(last, scene.as_str(), "the last hash is the scene's");

    env.spawn("replay");
    let replayed = env.ok(&["-s", "replay", "run", &log]);
    assert!(!replayed.stdout.contains("warning"), "{}", replayed.stdout);
    assert_eq!(
        json!(env.hash("replay")),
        scene,
        "the replay ends on the scene"
    );
}

/// A log whose first line opened a scene file replays after the file moved: `gs run --files`
/// reads the file in that folder with the content the line pinned, and a folder without it is
/// an error naming the hash and where the line read it.
pub fn opened_files_are_found_by_content() {
    let env = Env::new();
    let [then, now, empty] = ["then", "now", "empty"].map(|d| env.dir.join(d));
    for dir in [&then, &now, &empty] {
        fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
    }
    let saved = then.join("scene.gss").display().to_string();
    env.spawn_with("src", &["--verify"]);
    let call = |words: &[&str]| env.ok(&[&["-s", "src", "call"], words].concat());
    call(&["primitive.sphere", "subdivisions=3"]);
    call(&["file.save", &format!("path={saved}")]);
    call(&["file.open", &format!("path={saved}")]);
    call(&["mesh.jitter", "amount=1%"]);
    let log = env.log_of("src");
    let moved = now.join("moved.gss");
    fs::rename(&saved, &moved).unwrap_or_else(|e| panic!("{e}"));
    fs::write(now.join("other.txt"), "not the scene").unwrap_or_else(|e| panic!("{e}"));

    env.spawn("copy");
    let folder = now.display().to_string();
    let replayed = env.ok(&["-s", "copy", "run", &log, "--files", &folder]);
    let read = format!("# reads {} for {saved}", moved.display());
    assert!(replayed.stdout.contains(&read), "{}", replayed.stdout);
    assert_eq!(env.hash("copy"), env.hash("src"), "every hash checked");

    let hash = records(&log)[0]["args"]["hash"].clone();
    let missing = env.gs(&[
        "-s",
        "copy",
        "run",
        &log,
        "--files",
        &empty.display().to_string(),
    ]);
    assert_eq!(missing.code, 2, "{}", missing.stderr);
    let named = [hash.as_str().unwrap_or("no hash"), &saved];
    assert!(
        named.iter().all(|s| missing.stderr.contains(s)),
        "{}",
        missing.stderr
    );
}
