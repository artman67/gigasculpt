//! Live Boolean's live mode (decision 0057, section 8, as amended in step LB6): the
//! `live_boolean` setting, a run per group on each committed change, its cancel, the operand
//! cache, `session.state`'s `boolean.live`, and `boolean.apply` taking a settled result. An
//! in-process caller lands the runs with `Engine::tick`, which waits for them.

use std::path::{Path, PathBuf};

use gs_testkit::Session;
use serde_json::{Value, json};

use crate::boolean::primitives;
use crate::determinism::seeded;
use crate::support::{agent, log_records};
use crate::views::log_path;

fn jitter() -> Value {
    json!({ "part": "body", "amount": 0.01, "seed": 3 })
}

/// Moves the cutter (its transform: its points stay as they are).
fn shift(s: &mut Session, by: [f32; 3]) {
    s.ok(
        "transform.apply",
        json!({ "part": "cutter", "op": { "move": by } }),
    );
}

fn on(s: &mut Session) {
    s.ok("view.display", json!({ "live_boolean": true }));
}

fn live(s: &mut Session) -> Value {
    s.query("session.state", json!({}))["boolean.live"].clone()
}

/// Lands every run, waiting for those still working, and reads the live state.
fn settle(s: &mut Session) -> Value {
    let _ = s.engine.tick();
    live(s)
}

/// The hash of the part a fresh `boolean.apply` makes, in a session without the live mode,
/// after `build`.
fn applied(build: impl FnOnce(Session) -> Session) -> String {
    let mut s = build(agent());
    s.ok("boolean.apply", json!({}));
    hash(&mut s, "boolean")
}

fn hash(s: &mut Session, part: &str) -> String {
    let hash = s.query("mesh.hash", json!({ "part": part }))["hash"].clone();
    hash.as_str().unwrap_or_default().to_string()
}

fn remove(log: &Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn the_setting_is_one_logged_view_line_and_never_an_undo_step() {
    let log = log_path("live-setting");
    let mut s = primitives(seeded(0, Some(log.clone())));
    let state = |s: &mut Session| s.query("session.state", json!({}));
    let undos = state(&mut s)["history.undos"].clone();
    assert_eq!(
        state(&mut s)["view.live_boolean"],
        json!(false),
        "off at start"
    );
    on(&mut s);
    assert_eq!(state(&mut s)["view.live_boolean"], json!(true));
    s.ok("view.display", json!({ "live_boolean": false }));
    assert_eq!(state(&mut s)["view.live_boolean"], json!(false));
    assert_eq!(state(&mut s)["history.undos"], undos, "never an undo step");
    let (_, records) = log_records(&log);
    remove(&log);
    let lines: Vec<Value> = (records.iter())
        .filter(|r| r.command == "view.display")
        .map(|r| r.args.clone())
        .collect();
    assert_eq!(
        lines,
        [
            json!({ "live_boolean": true }),
            json!({ "live_boolean": false })
        ],
        "one logged line each"
    );
}

#[test]
fn a_committed_edit_starts_one_run_and_its_result_is_a_fresh_apply_bit_for_bit() {
    let mut s = primitives(agent());
    on(&mut s);
    assert_eq!(
        settle(&mut s)["runs"],
        json!(1),
        "turning it on runs the group"
    );
    s.ok("mesh.jitter", jitter());
    let live = settle(&mut s);
    assert_eq!(live["runs"], json!(2), "the edit started one run");
    let group = &live["groups"][0];
    assert_eq!(group["state"], json!("settled"), "{live}");
    assert_eq!(group["operands"], json!(["body", "cutter"]));
    assert_eq!(group["solver"], json!("manifold"));
    let fresh = applied(|s| {
        let mut s = primitives(s);
        s.ok("mesh.jitter", jitter());
        s
    });
    assert_eq!(
        group["hash"],
        json!(fresh),
        "the bits of a fresh boolean.apply"
    );
}

#[test]
fn a_second_change_before_the_run_lands_cancels_it_and_its_result_never_lands() {
    let mut s = primitives(agent());
    on(&mut s);
    settle(&mut s);
    shift(&mut s, [0.1, 0.0, 0.0]);
    shift(&mut s, [0.1, 0.0, 0.0]);
    let live = settle(&mut s);
    assert_eq!(
        [&live["runs"], &live["cancelled"], &live["landed"]],
        [&json!(3), &json!(1), &json!(2)],
        "the second move cancelled the first move's run, which never landed: {live}"
    );
    let moved = |times: usize| {
        applied(|s| {
            let mut s = primitives(s);
            for _ in 0..times {
                shift(&mut s, [0.1, 0.0, 0.0]);
            }
            s
        })
    };
    let (both, first) = (moved(2), moved(1));
    assert_ne!(both, first);
    assert_eq!(
        live["groups"][0]["hash"],
        json!(both),
        "the last change's result"
    );
}

#[test]
fn moving_a_cutter_imports_neither_operand_again() {
    let mut s = primitives(agent());
    on(&mut s);
    assert_eq!(
        settle(&mut s)["imports"],
        json!(2),
        "each operand imported once"
    );
    shift(&mut s, [0.1, 0.0, 0.0]);
    let live = settle(&mut s);
    assert_eq!(
        [&live["runs"], &live["imports"]],
        [&json!(2), &json!(2)],
        "the move ran the group again from the cached imports"
    );
    let moved = applied(|s| {
        let mut s = primitives(s);
        shift(&mut s, [0.1, 0.0, 0.0]);
        s
    });
    assert_eq!(live["groups"][0]["hash"], json!(moved));
    s.ok("mesh.jitter", jitter());
    assert_eq!(
        settle(&mut s)["imports"],
        json!(3),
        "an edited body is imported again"
    );
}

#[test]
fn an_undo_of_the_edit_gives_the_result_for_the_undone_parts() {
    let mut s = primitives(agent());
    on(&mut s);
    let before = settle(&mut s)["groups"][0]["hash"].clone();
    assert_eq!(before, json!(applied(primitives)));
    s.ok("mesh.jitter", jitter());
    let edited = settle(&mut s)["groups"][0]["hash"].clone();
    assert_ne!(edited, before);
    s.ok("history.undo", json!({}));
    let live = settle(&mut s);
    assert_eq!(
        live["groups"][0]["hash"], before,
        "the undone parts' result"
    );
    assert_eq!(live["runs"], json!(3));
}

/// A log line: its command, its arguments and the scene's hash after it.
type Line = (String, Value, Option<String>);

/// The scene's hash, its saved file and its log lines, but the setting's own. Both sessions set
/// the setting first, and log and save at the same paths, since a save records its log's path,
/// the command count and a scene id made from them.
fn kept(on_first: bool) -> (String, Vec<u8>, Vec<Line>) {
    let log = log_path("live-kept");
    let mut s = seeded(0, Some(log.clone()));
    s.ok("view.display", json!({ "live_boolean": on_first }));
    let mut s = primitives(s);
    s.ok("mesh.jitter", jitter());
    shift(&mut s, [0.1, 0.0, 0.0]);
    // On, each change ran the group and cancelled the run before it; only the last landed.
    let live = settle(&mut s);
    let counts = if on_first { [4, 3, 1] } else { [0; 3] };
    assert_eq!(
        [&live["runs"], &live["cancelled"], &live["landed"]],
        counts.map(|n| json!(n)).each_ref(),
        "{live}"
    );
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
fn the_scene_hash_its_save_and_its_log_are_the_same_with_the_setting_on_or_off() {
    let (on_hash, on_saved, on_lines) = kept(true);
    let (off_hash, off_saved, off_lines) = kept(false);
    assert_eq!(on_hash, off_hash, "the scene");
    assert!(on_saved == off_saved, "the saved file");
    assert_eq!(on_lines, off_lines, "the log");
}

#[test]
fn apply_after_a_settled_result_cuts_nothing_and_gives_a_fresh_calls_bits() {
    let mut s = primitives(agent());
    on(&mut s);
    settle(&mut s);
    let taken = s.ok("boolean.apply", json!({}));
    let mut fresh = primitives(agent());
    let cut = fresh.ok("boolean.apply", json!({}));
    let group = |out: &gs_protocol::Success, live: bool| {
        let g = &out.result["groups"][0];
        assert_eq!(g["live"], json!(live), "{g}");
        [&g["operands"], &g["solver"], &g["repairs"], &g["dropped"]].map(Value::clone)
    };
    assert_eq!(group(&taken, true), group(&cut, false), "the same report");
    assert_eq!(taken.warnings, cut.warnings);
    assert_eq!(
        hash(&mut s, "boolean"),
        hash(&mut fresh, "boolean"),
        "the same bits"
    );
    assert_eq!(s.hash(), fresh.hash(), "the same scene");
    let live = settle(&mut s);
    assert_eq!(
        live["runs"],
        json!(1),
        "no run after: each group is one part now"
    );
    assert_eq!(live["groups"], json!([]));
}

#[test]
fn a_refused_group_reports_its_reason() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "name": "body" }));
    let over = json!({ "name": "cutter", "size": 3 });
    s.ok("primitive.cube", over);
    s.ok(
        "part.set_operation",
        json!({ "part": "cutter", "operation": "difference" }),
    );
    on(&mut s);
    let live = settle(&mut s);
    let group = &live["groups"][0];
    assert_eq!(group["state"], json!("refused"), "{live}");
    let refused = s.err("boolean.apply", json!({}));
    assert_eq!(
        group["why"],
        json!(refused.message),
        "boolean.apply's own reason"
    );
    assert!(
        refused.message.contains("cuts to nothing"),
        "{}",
        refused.message
    );
}

#[test]
fn the_same_bits_at_one_thread_and_twenty_four() {
    let bigger = |s: &mut Session| {
        s.ok(
            "primitive.sphere",
            json!({ "name": "body", "subdivisions": 24 }),
        );
        let over = json!({ "name": "cutter", "size": 1, "center": [0.8, 0.1, 0.05] });
        s.ok("primitive.cube", over);
        s.ok(
            "part.set_operation",
            json!({ "part": "cutter", "operation": "difference" }),
        );
    };
    let run = |threads: usize| {
        gs_par::with_threads(threads, || {
            let mut s = agent();
            bigger(&mut s);
            on(&mut s);
            let live = settle(&mut s)["groups"][0]["hash"].clone();
            let fresh = applied(|mut s| {
                bigger(&mut s);
                s
            });
            (live, json!(fresh))
        })
    };
    let (one, one_fresh) = run(1);
    let (many, many_fresh) = run(24);
    assert!(one.is_string(), "{one}");
    assert_eq!([&one, &one_fresh, &many_fresh], [&many; 3]);
}

#[test]
fn with_the_setting_off_no_run_ever_starts() {
    let mut s = primitives(agent());
    s.ok("mesh.jitter", jitter());
    shift(&mut s, [0.1, 0.0, 0.0]);
    s.ok("history.undo", json!({}));
    let live = settle(&mut s);
    assert_eq!(
        [&live["runs"], &live["imports"], &live["groups"]],
        [&json!(0), &json!(0), &json!([])],
        "{live}"
    );
}
