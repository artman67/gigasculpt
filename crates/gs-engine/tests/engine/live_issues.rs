//! Live Boolean's issue marks (decision 0057, section 9, as amended for step LB7):
//! `view.display`'s `show_issues` and `show_coplanar`, off at start, are each one logged View
//! line, never an undo step, and leave the scene's hash, its save and its log as they were.
//! Turning one on runs each group again for its marks; turning it off keeps the runs.

use std::path::{Path, PathBuf};

use gs_testkit::Session;
use serde_json::{Value, json};

use crate::boolean::primitives;
use crate::determinism::seeded;
use crate::support::{agent, log_records};
use crate::views::log_path;

const SETTINGS: [&str; 2] = ["show_issues", "show_coplanar"];

fn state(s: &mut Session, key: &str) -> Value {
    s.query("session.state", json!({}))[key].clone()
}

/// Lands every run and reads the live state.
fn settle(s: &mut Session) -> Value {
    let _ = s.engine.tick();
    state(s, "boolean.live")
}

fn remove(log: &Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn each_setting_is_one_logged_view_line_and_never_an_undo_step() {
    let log = log_path("live-issues-setting");
    let mut s = primitives(seeded(0, Some(log.clone())));
    let undos = state(&mut s, "history.undos");
    for name in SETTINGS {
        let key = format!("view.{name}");
        assert_eq!(state(&mut s, &key), json!(false), "{name} is off at start");
        s.ok("view.display", json!({ name: true }));
        assert_eq!(state(&mut s, &key), json!(true), "{name}");
        s.ok("view.display", json!({ name: false }));
        assert_eq!(state(&mut s, &key), json!(false), "{name}");
    }
    assert_eq!(state(&mut s, "history.undos"), undos, "never an undo step");
    let (_, records) = log_records(&log);
    remove(&log);
    let lines: Vec<Value> = (records.iter())
        .filter(|r| r.command == "view.display")
        .map(|r| r.args.clone())
        .collect();
    let each = SETTINGS.map(|name| [json!({ name: true }), json!({ name: false })]);
    assert_eq!(lines, each.concat(), "one logged line each");
}

#[test]
fn turning_a_setting_on_runs_each_group_again_and_turning_it_off_keeps_the_result() {
    let mut s = primitives(agent());
    s.ok("view.display", json!({ "live_boolean": true }));
    let first = settle(&mut s);
    assert_eq!(first["runs"], json!(1), "one run");
    for (runs, name) in [(2, "show_issues"), (3, "show_coplanar")] {
        s.ok("view.display", json!({ name: true }));
        let live = settle(&mut s);
        assert_eq!(live["runs"], json!(runs), "{name} on ran the group again");
        let hash = &first["groups"][0]["hash"];
        assert_eq!(&live["groups"][0]["hash"], hash, "{name}: the same result");
    }
    let off = json!({ "show_issues": false, "show_coplanar": false });
    s.ok("view.display", off);
    assert_eq!(
        settle(&mut s)["runs"],
        json!(3),
        "turning them off kept the run"
    );
}

/// A log line: its command, its arguments and the scene's hash after it.
type Line = (String, Value, Option<String>);

/// The scene's hash, its saved file and its log lines but the settings', with the live mode
/// on and the marks asked for or not. Both log and save at the same paths, since a save
/// records its log's path.
fn kept(marks: bool) -> (String, Vec<u8>, Vec<Line>) {
    let log = log_path("live-issues-kept");
    let mut s = seeded(0, Some(log.clone()));
    let display = json!({ "live_boolean": true, "show_issues": marks, "show_coplanar": marks });
    s.ok("view.display", display);
    let mut s = primitives(s);
    let moved = json!({ "part": "cutter", "op": { "move": [0.1, 0.0, 0.0] } });
    s.ok("transform.apply", moved);
    let state = settle(&mut s)["groups"][0]["state"].clone();
    assert_eq!(state, json!("settled"), "the group settles");
    let path: PathBuf = log.with_extension("gss");
    s.ok("file.save", json!({ "path": path.display().to_string() }));
    let saved = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    let (_, records) = log_records(&log);
    remove(&log);
    let lines = (records.into_iter())
        .filter(|r| r.command != "view.display")
        .map(|r| (r.command, r.args, r.hash))
        .collect();
    (s.hash(), saved, lines)
}

#[test]
fn the_scene_hash_its_save_and_its_log_are_the_same_with_the_marks_on_or_off() {
    let (on_hash, on_saved, on_lines) = kept(true);
    let (off_hash, off_saved, off_lines) = kept(false);
    assert_eq!(on_hash, off_hash, "the scene");
    assert!(on_saved == off_saved, "the saved file");
    assert_eq!(on_lines, off_lines, "the log");
}
