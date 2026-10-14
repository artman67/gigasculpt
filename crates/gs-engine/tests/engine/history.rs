//! The undo history: what steps hold and the byte budget, checkpoints, revisions and expected
//! step ids. (The model test is in `model.rs`, failures in `atomic.rs`, effects in `effects.rs`,
//! the log and replay in `replay.rs`.)

use gs_engine::{EngineConfig, Request};
use gs_protocol::{BatchParams, Role, RunParams, Step};
use serde_json::{Value, json};

use gs_testkit::{Session, code, failure, success, text};

use crate::support::{agent, tagged_triangle};

fn budget_script(s: &mut Session) {
    s.ok(
        "primitive.sphere",
        json!({ "name": "a", "subdivisions": 1 }),
    );
    s.ok(
        "part.create",
        json!({ "name": "t", "mesh": tagged_triangle() }),
    );
    s.ok("primitive.cube", json!({ "name": "c", "subdivisions": 3 }));
    s.ok(
        "mesh.translate",
        json!({ "part": "a", "direction": [1, 0, 0], "distance": 1 }),
    );
    s.ok("mesh.jitter", json!({ "part": "c", "amount": 0.1 }));
}

/// Each step's bytes, oldest first.
fn step_bytes(s: &mut Session) -> Vec<u64> {
    let steps = s.history()["steps"].clone();
    let mut bytes: Vec<u64> = (steps.as_array().into_iter().flatten())
        .map(|step| step["bytes"].as_u64().unwrap_or_default())
        .collect();
    bytes.reverse();
    bytes
}

/// What each step of `budget_script` holds, oldest first, by an independent model: a new part
/// costs the history nothing (the scene holds it), and an edit keeps the old positions and
/// normals of every node it moved, 12 bytes each per vertex. These parts are one node each.
fn held(s: &mut Session) -> [u64; 5] {
    let vertices = |s: &mut Session, part: &str| s.mesh(part).positions.len() as u64;
    let (sphere, cube) = (vertices(s, "a"), vertices(s, "c"));
    [0, 0, 0, 24 * sphere, 24 * cube]
}

/// Bytes the newest `keep` steps hold.
fn model_bytes(held: &[u64], keep: usize) -> u64 {
    held[held.len() - keep..].iter().sum()
}

/// Steps hold only what they changed, and the totals come from the mesh stores.
#[test]
fn steps_hold_only_what_they_changed() {
    let mut s = agent();
    budget_script(&mut s);
    let held = held(&mut s);
    let history = s.history();
    assert_eq!(history["bytes"], json!(model_bytes(&held, 5)), "the total");
    assert_eq!(step_bytes(&mut s), held, "each step");
    assert_eq!(history["checkpoint_bytes"], json!(0), "no checkpoints");
}

#[test]
fn the_history_keeps_the_newest_steps_that_fit_the_budget() {
    let mut full = agent();
    budget_script(&mut full);
    let held = held(&mut full);
    // The most steps whose bytes fit in `budget`; the newest step is always kept.
    let fits = |budget: u64| {
        (1..=held.len())
            .rev()
            .find(|&k| model_bytes(&held, k) <= budget)
            .unwrap_or(1)
    };
    let config = |budget: u64| EngineConfig {
        history_budget_bytes: budget,
        ..EngineConfig::default()
    };
    for keep in 1..=held.len() {
        let total = model_bytes(&held, keep);
        for budget in [total, total.saturating_sub(1)] {
            let kept = fits(budget);
            let mut s = Session::with(config(budget), Role::Agent);
            budget_script(&mut s);
            let history = s.history();
            assert_eq!(history["undoable"], json!(kept), "budget {budget}");
            assert_eq!(
                history["bytes"],
                json!(model_bytes(&held, kept)),
                "bytes at budget {budget}"
            );
            assert_eq!(history["budget_bytes"], json!(budget), "reported budget");
            for _ in 0..kept {
                s.ok("history.undo", json!({}));
            }
            assert_eq!(
                code(s.err("history.undo", json!({})).code),
                "E_PRECONDITION",
                "nothing older"
            );
        }
    }
}

/// Dropping an old edit frees its store step even when a later step has removed the part and
/// holds it (here after another part was added, so the search has to pass that step). Each step
/// lists what it alone holds: the edit its old positions and normals, the removal both parts.
#[test]
fn dropping_an_edit_reaches_a_part_a_later_step_holds() {
    let script = |s: &mut Session| {
        s.ok("history.checkpoint", json!({ "name": "empty" }));
        s.ok("primitive.cube", json!({ "name": "a", "subdivisions": 2 }));
        s.ok(
            "mesh.translate",
            json!({ "part": "a", "direction": [0, 1, 0], "distance": 1 }),
        );
        s.ok("primitive.cube", json!({ "name": "b" }));
        s.ok("history.restore", json!({ "name": "empty" }));
    };
    let mut full = agent();
    script(&mut full);
    full.ok("history.undo", json!({}));
    let moved = full.mesh("a");
    let edit = 24 * moved.positions.len() as u64;
    full.ok("history.redo", json!({}));
    let steps = step_bytes(&mut full);
    assert_eq!(
        steps[1], edit,
        "the edit holds the old positions and normals"
    );
    let (removal, total) = (steps[3], steps.iter().sum::<u64>());
    assert!(removal > edit, "the removal holds both parts");
    let config = EngineConfig {
        history_budget_bytes: total - 1,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    script(&mut s);
    let history = s.history();
    assert_eq!(
        history["undoable"],
        json!(2),
        "b's step and the removal fit"
    );
    assert_eq!(
        history["bytes"],
        json!(removal),
        "the removed part a no longer holds its edit"
    );
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    assert_eq!(s.mesh("a").positions, moved.positions, "back, still moved");
    let err = s.err("history.undo", json!({}));
    assert_eq!(code(err.code), "E_PRECONDITION", "and nothing older");
}

/// Checkpoints share the scene's data (decision 0010): free while the scene holds the same
/// nodes, then holding what only they keep, each allocation once, outside the budget.
#[test]
fn checkpoint_memory_is_reported_apart() {
    let config = EngineConfig {
        history_budget_bytes: 1,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok(
        "primitive.sphere",
        json!({ "name": "a", "subdivisions": 2 }),
    );
    s.ok("history.checkpoint", json!({ "name": "x" }));
    s.ok("primitive.cube", json!({ "name": "c" }));
    let history = s.history();
    assert_eq!(
        history["bytes"],
        json!(0),
        "new parts cost the history nothing"
    );
    assert_eq!(
        history["checkpoint_bytes"],
        json!(0),
        "the checkpoint shares the unchanged sphere"
    );
    let up = json!({ "part": "a", "direction": [0, 1, 0], "distance": 1 });
    s.ok("mesh.translate", up.clone());
    assert_eq!(
        s.history()["checkpoint_bytes"],
        json!(0),
        "the newest step holds the sphere's old nodes"
    );
    s.ok("mesh.translate", up);
    // The budget kept only the newest step, so the first positions and normals are the
    // checkpoint's alone: 24 bytes a vertex.
    let only = 24 * s.mesh("a").positions.len() as u64;
    assert_eq!(
        s.history()["checkpoint_bytes"],
        json!(only),
        "what only x keeps"
    );
    s.ok("history.checkpoint", json!({ "name": "y" }));
    assert_eq!(
        s.history()["checkpoint_bytes"],
        json!(only),
        "a second checkpoint of the scene adds nothing"
    );
    let text = text(&s.query("scene.describe", json!({}))["text"]);
    assert!(
        text.contains(&format!("({only} more bytes)")),
        "describe reports it:\n{text}"
    );
}

#[test]
fn the_revision_counts_document_changes_only() {
    let mut s = agent();
    let rev = |s: &mut Session, command: &str, args: Value| s.call(command, args).rev;
    assert_eq!(
        rev(&mut s, "mesh.stats", json!({})),
        0,
        "queries change nothing"
    );
    assert_eq!(rev(&mut s, "primitive.cube", json!({})), 1, "an edit");
    assert_eq!(
        rev(
            &mut s,
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 0 })
        ),
        1,
        "a no-op edit"
    );
    assert_eq!(
        rev(
            &mut s,
            "mesh.translate",
            json!({ "part": "zz", "direction": [1, 0, 0], "distance": 1 })
        ),
        1,
        "a failure"
    );
    assert_eq!(
        rev(&mut s, "history.checkpoint", json!({ "name": "c" })),
        1,
        "a checkpoint"
    );
    assert_eq!(rev(&mut s, "history.undo", json!({})), 2, "undo");
    assert_eq!(rev(&mut s, "history.redo", json!({})), 3, "redo");
    assert_eq!(
        rev(&mut s, "primitive.sphere", json!({})),
        4,
        "another edit"
    );
    assert_eq!(
        rev(&mut s, "history.restore", json!({ "name": "c" })),
        5,
        "a restore"
    );
}

#[test]
fn stale_expectations_are_refused() {
    let mut s = agent();
    let made = s.ok("primitive.cube", json!({}));
    let at = |rev: u64, batch: bool| {
        let args = json!({ "direction": [1, 0, 0], "distance": 1 });
        if batch {
            let commands = vec![Step {
                command: "mesh.translate".into(),
                args,
            }];
            Request::Batch(BatchParams {
                commands,
                if_rev: Some(rev),
            })
        } else {
            Request::Run(RunParams {
                if_rev: Some(rev),
                ..RunParams::new("mesh.translate", args)
            })
        }
    };
    for batch in [false, true] {
        let rev = s.call("mesh.stats", json!({})).rev;
        let err = failure(s.request(at(rev + 1, batch)), "stale");
        assert_eq!(code(err.code), "E_STALE", "wrong revision, batch {batch}");
        assert!(err.hint.is_some(), "says to look again");
        success(s.request(at(rev, batch)), "current revision");
    }
    let err = s.err("history.undo", json!({ "expect": made.history_id }));
    assert_eq!(
        code(err.code),
        "E_STALE",
        "the newest step is a move, not the cube"
    );
    let newest = s.history()["steps"][0]["id"].clone();
    s.ok("history.undo", json!({ "expect": newest }));
    let err = s.err("history.redo", json!({ "expect": made.history_id }));
    assert_eq!(code(err.code), "E_STALE", "the step to redo is the move");
    assert!(err.hint.is_some(), "says how to see the steps");
}

/// `session.state`'s `history.undo_id` is the id of the step `history.undo` would undo, which
/// the window's adjust panel knows its step by (decision 0060, as amended 2026-10-13, step H2):
/// null with none, and new when the history drops its oldest step to make room for a new one,
/// though the count of steps to undo stays the same.
#[test]
fn the_undo_id_names_the_step_an_undo_would_undo() {
    let undo_id = |s: &mut Session| s.query("session.state", json!({}))["history.undo_id"].clone();
    let mut s = agent();
    assert_eq!(undo_id(&mut s), Value::Null, "nothing to undo");
    let made = s.ok("primitive.cube", json!({ "name": "c", "subdivisions": 2 }));
    assert_eq!(undo_id(&mut s), json!(made.history_id), "the step made");
    let shift = json!({ "part": "c", "direction": [1, 0, 0], "distance": 1 });
    s.ok("mesh.translate", shift.clone());
    // A budget that holds one move: each new move drops the one before it.
    let config = EngineConfig {
        history_budget_bytes: step_bytes(&mut s)[1],
        ..EngineConfig::default()
    };
    s.ok("history.undo", json!({}));
    assert_eq!(
        undo_id(&mut s),
        json!(made.history_id),
        "undone to the cube"
    );
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.cube", json!({ "name": "c", "subdivisions": 2 }));
    s.ok("mesh.translate", shift.clone());
    s.ok("mesh.translate", shift.clone());
    let (before, count) = (undo_id(&mut s), s.history()["undoable"].clone());
    let made = s.ok("mesh.translate", shift);
    assert_eq!(s.history()["undoable"], count, "the count stays the same");
    assert_ne!(undo_id(&mut s), before, "a new step");
    assert_eq!(undo_id(&mut s), json!(made.history_id), "the one made");
}

/// The ends of the history are named for a person (`session.state`'s `history.undo` and
/// `history.redo`, which the window's History buttons read): a command by its title, a batch by
/// its first command and how many follow.
#[test]
fn the_ends_are_named_by_title() {
    let mut s = agent();
    let ends = |s: &mut Session| {
        let state = s.query("session.state", json!({}));
        (state["history.undo"].clone(), state["history.redo"].clone())
    };
    s.ok("primitive.sphere", json!({ "subdivisions": 1 }));
    assert_eq!(
        ends(&mut s),
        (json!("Add sphere"), Value::Null),
        "one command"
    );
    let steps = [
        ("primitive.cube", json!({})),
        (
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 1 }),
        ),
        ("mesh.stats", json!({})),
    ];
    success(s.batch(&steps), "batch");
    s.ok("history.undo", json!({}));
    assert_eq!(
        ends(&mut s),
        (json!("Add sphere"), json!("Add cube and 2 more")),
        "a batch, undone"
    );
}
