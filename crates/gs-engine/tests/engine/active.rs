//! The active part (decision 0033): what makes a part active, that a command naming no part
//! works on it, and how undo, a failed call and Isolate treat it. That the scene file keeps it
//! is in `scene::autosave` (`isolate_comes_back_through_open_and_recovery`), and that a log
//! replays it in `parts` (`a_log_of_list_commands_replays_to_the_same_scene`).

use crate::parts::three;
use gs_protocol::{ErrorCode, Outcome, Request, RunParams};
use gs_testkit::Session;
use serde_json::{Value, json};

/// The active part, as `session.state` reports it.
fn active(s: &mut Session) -> Value {
    s.query("session.state", json!({}))["parts.active"].clone()
}

/// The parts `part.list` marks with `key`, in list order.
fn marked(s: &mut Session, key: &str) -> Vec<String> {
    let list = s.query("part.list", json!({}));
    (list["parts"].as_array().into_iter().flatten())
        .filter(|p| p[key] == json!(true))
        .map(|p| p["name"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn the_part_added_last_is_active_and_a_command_naming_none_works_on_it() {
    let mut s = Session::new();
    assert_eq!(active(&mut s), json!(null), "an empty scene has none");
    let e = s.err("part.duplicate", json!({}));
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    let mut s = three();
    assert_eq!(active(&mut s), json!("head"));
    assert_eq!(
        marked(&mut s, "active"),
        ["head"],
        "part.list marks it alone"
    );
    s.ok(
        "mesh.translate",
        json!({ "direction": [0, 1, 0], "distance": 1 }),
    );
    let steps = s.history()["steps"].clone();
    let step = (steps.as_array().into_iter().flatten()).find(|r| r["command"] == "mesh.translate");
    assert_eq!(
        step.map(|r| r["args"]["part"].clone()),
        Some(json!("head")),
        "the step records the part's name, so a log replays without the window"
    );
    let named = s.query("part.get", json!({ "part": "head" }));
    assert_eq!(
        s.query("part.get", json!({ "part": "@active" })),
        named,
        "@active names it"
    );
}

#[test]
fn select_takes_any_reference_or_a_neighbor_and_is_never_an_undo_step() {
    let mut s = three();
    let steps = s.history()["undoable"].clone();
    for (args, want) in [
        (json!({ "part": "sphere" }), "sphere"),
        (json!({ "part": "#2" }), "cube"),
        (json!({ "part": "@2" }), "head"),
        (json!({ "neighbor": "previous" }), "cube"),
        (json!({ "neighbor": "previous" }), "sphere"),
        (json!({ "neighbor": "previous" }), "sphere"),
        (json!({ "neighbor": "next" }), "cube"),
        (json!({ "neighbor": "next" }), "head"),
        (json!({ "neighbor": "next" }), "head"),
    ] {
        let done = s.ok("part.select", args.clone());
        assert_eq!(done.result["part"], json!(want), "{args}");
        assert_eq!(active(&mut s), json!(want), "{args}");
    }
    assert_eq!(s.history()["undoable"], steps, "no undo step");
    let both = s.err("part.select", json!({ "part": "cube", "neighbor": "next" }));
    assert_eq!(both.code, ErrorCode::ArgsInvalid, "{both:?}");
    let missing = s.err("part.select", json!({ "part": "nose" }));
    assert_eq!(missing.code, ErrorCode::NotFound, "{missing:?}");
}

#[test]
fn a_neighbor_takes_over_from_a_deleted_part_and_undo_puts_back_the_active_part() {
    let mut s = three();
    s.ok("part.select", json!({ "part": "cube" }));
    s.ok("part.delete", json!({}));
    assert_eq!(
        active(&mut s),
        json!("head"),
        "the part that took its place"
    );
    s.ok("history.undo", json!({}));
    assert_eq!(
        active(&mut s),
        json!("cube"),
        "undo: the part active before"
    );
    s.ok("history.redo", json!({}));
    assert_eq!(active(&mut s), json!("head"), "redo: the one active after");
    s.ok("part.delete", json!({}));
    assert_eq!(
        active(&mut s),
        json!("sphere"),
        "the last part: the one above"
    );
    s.ok("part.duplicate", json!({}));
    assert_eq!(active(&mut s), json!("sphere-2"), "a copy is active");
    s.ok("history.undo", json!({}));
    assert_eq!(active(&mut s), json!("sphere"), "undoing it: the source");
    s.ok("part.delete", json!({}));
    assert_eq!(active(&mut s), json!(null), "no part, none active");
}

#[test]
fn a_failed_batch_or_a_dry_run_leaves_the_active_part() {
    let mut s = three();
    s.ok("part.select", json!({ "part": "sphere" }));
    let failed = s.batch(&[
        ("part.duplicate", json!({})),
        ("part.rename", json!({ "part": "nowhere", "name": "x" })),
    ]);
    assert!(matches!(failed.outcome, Outcome::Error(_)), "{failed:?}");
    assert_eq!(
        active(&mut s),
        json!("sphere"),
        "the copy went with the batch"
    );
    let dry = s.request(Request::Run(RunParams::new("part.delete", json!({})).dry()));
    assert!(matches!(dry.outcome, Outcome::Ok(_)), "{dry:?}");
    assert_eq!(active(&mut s), json!("sphere"), "a dry run deletes nothing");
}

#[test]
fn the_active_part_shows_whatever_its_eye_says_and_isolate_follows_it() {
    let mut s = three();
    s.ok(
        "part.set_visible",
        json!({ "parts": ["head"], "visible": false }),
    );
    let all = ["sphere", "cube", "head"];
    assert_eq!(marked(&mut s, "shown"), all, "the active head shows");
    s.ok("part.select", json!({ "part": "cube" }));
    assert_eq!(
        marked(&mut s, "shown"),
        ["sphere", "cube"],
        "its eye takes effect once another part is active"
    );
    s.ok("part.isolate", json!({ "parts": ["@active"] }));
    assert_eq!(marked(&mut s, "shown"), ["cube"]);
    let moved = s.ok("part.select", json!({ "part": "sphere" }));
    assert_eq!(moved.result["isolated"], json!(true));
    assert_eq!(
        marked(&mut s, "shown"),
        ["sphere"],
        "selecting moves Isolate"
    );
    s.ok("part.isolate", json!({ "parts": ["cube", "head"] }));
    assert_eq!(
        active(&mut s),
        json!("cube"),
        "isolating parts without the active one makes the first of them active"
    );
}
