//! Atomicity: a failed, refused, crashing or dry call leaves the scene, the history and the
//! revision exactly as they were, alone or inside a batch, and the engine keeps working.

use gs_engine::Request;
use gs_protocol::RunParams;
use serde_json::{Value, json};

use gs_testkit::{code, failure, success};

use crate::support::agent;

#[test]
fn failed_commands_leave_everything_as_it_was() {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "name": "a", "subdivisions": 2 }),
    );
    s.ok(
        "mesh.translate",
        json!({ "part": "a", "direction": [1, 0, 0], "distance": 1 }),
    );
    s.ok("history.checkpoint", json!({ "name": "base" }));
    let broken = json!({ "positions": [[0, 0, 0]], "faces": [[0, 1, 2, 3]] });
    // (command, arguments, error code, whether the error says how to fix it)
    let failures: [(&str, Value, &str, bool); 7] = [
        ("debug.panic", json!({}), "E_INTERNAL", false),
        (
            "mesh.translate",
            json!({ "part": "zz", "direction": [1, 0, 0], "distance": 1 }),
            "E_NOT_FOUND",
            false,
        ),
        (
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 3.0e38 }),
            "E_ARGS_INVALID",
            true,
        ),
        (
            "mesh.translate",
            json!({ "direction": [0, 0, 0], "distance": 1 }),
            "E_ARGS_INVALID",
            false,
        ),
        (
            "part.create",
            json!({ "name": "bad", "mesh": broken }),
            "E_ARGS_INVALID",
            true,
        ),
        (
            "history.restore",
            json!({ "name": "nope" }),
            "E_NOT_FOUND",
            false,
        ),
        ("history.undo", json!({ "expect": 999 }), "E_STALE", true),
    ];
    for (command, args, wanted, hinted) in failures {
        let (hash, history, rev) = (s.hash(), s.history(), s.call("session.info", json!({})).rev);
        let response = s.call(command, args);
        let rev_after = response.rev;
        let err = failure(response, command);
        assert_eq!(code(err.code), wanted, "{command}: {}", err.message);
        assert!(
            !hinted || err.hint.is_some(),
            "{command}: says how to fix it"
        );
        assert_eq!(
            (s.hash(), s.history(), rev_after),
            (hash, history, rev),
            "{command} left traces"
        );
    }
    let err = s.err("debug.panic", json!({ "part": "a" }));
    assert!(
        err.message.contains("panicked"),
        "a crash says so: {}",
        err.message
    );
    s.ok(
        "mesh.translate",
        json!({ "part": "a", "direction": [1, 0, 0], "distance": 1 }),
    );
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    let first = s.mesh("a").positions;
    s.ok("history.redo", json!({}));
    assert_ne!(s.mesh("a").positions, first, "the history still works");
}

#[test]
fn a_failing_batch_rolls_back_every_step() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "name": "a" }));
    let (hash, history) = (s.hash(), s.history());
    let response = s.batch(&[
        (
            "mesh.translate",
            json!({ "part": "a", "direction": [1, 0, 0], "distance": 1 }),
        ),
        ("primitive.cube", json!({ "name": "b" })),
        (
            "mesh.translate",
            json!({ "part": "zz", "direction": [1, 0, 0], "distance": 1 }),
        ),
    ]);
    let err = failure(response, "batch");
    assert_eq!(code(err.code), "E_NOT_FOUND", "step 2 fails");
    assert!(
        err.path
            .as_deref()
            .is_some_and(|p| p.starts_with("/commands/2")),
        "{:?}",
        err.path
    );
    assert!(err.message.starts_with("step 2"), "{}", err.message);
    assert_eq!(
        (s.hash(), s.history()),
        (hash.clone(), history),
        "the first two steps were undone"
    );
    let err = failure(s.batch(&[("history.undo", json!({}))]), "undo in a batch");
    assert_eq!(
        err.path.as_deref(),
        Some("/commands/0/command"),
        "only edits and queries batch"
    );
    let err = failure(
        s.batch(&[("primitive.cube", json!({})), ("nope", json!({}))]),
        "unknown",
    );
    assert_eq!(
        code(err.code),
        "E_UNKNOWN_COMMAND",
        "checked before anything runs"
    );
    assert_eq!(s.hash(), hash, "nothing ran");
}

#[test]
fn a_part_made_and_removed_in_one_batch_changes_nothing() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("history.checkpoint", json!({ "name": "base" }));
    let (hash, steps) = (s.hash(), s.history()["undoable"].clone());
    let done = success(
        s.batch(&[
            ("primitive.cube", json!({ "name": "b" })),
            ("history.restore", json!({ "name": "base" })),
        ]),
        "make and remove",
    );
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("a batch reports its effect"));
    assert!(
        effect.parts.is_empty() && effect.noop_reason.is_some(),
        "nothing changed in the end"
    );
    assert!(done.history_id.is_none(), "so no step");
    assert_eq!(
        (s.hash(), s.history()["undoable"].clone()),
        (hash, steps),
        "same scene, same history"
    );
}

/// A part moved and then restored in one call: the move is dropped, and the part's own history
/// stays usable.
#[test]
fn an_edit_then_a_restore_in_one_batch() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "name": "a", "subdivisions": 2 }));
    s.ok("history.checkpoint", json!({ "name": "base" }));
    let (base, steps) = (s.hash(), s.history()["undoable"].clone());
    let translate = |d: f64| json!({ "part": "a", "direction": [1, 0, 0], "distance": d });
    let back = success(
        s.batch(&[
            ("mesh.translate", translate(5.0)),
            ("history.restore", json!({ "name": "base" })),
        ]),
        "move and restore",
    );
    assert!(back.history_id.is_none(), "back where it was: no step");
    assert_eq!(
        (s.hash(), s.history()["undoable"].clone()),
        (base.clone(), steps)
    );
    s.ok("mesh.translate", translate(1.0));
    s.ok("history.checkpoint", json!({ "name": "moved" }));
    let moved = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), base, "the part's own step undoes");
    let done = success(
        s.batch(&[
            ("mesh.translate", translate(5.0)),
            ("history.restore", json!({ "name": "moved" })),
        ]),
        "move and restore another state",
    );
    assert!(done.history_id.is_some(), "one step");
    assert_eq!(s.hash(), moved, "the checkpoint's part");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), base, "undo brings back the part before the batch");
    s.ok("mesh.translate", translate(2.0));
}

#[test]
fn dry_runs_check_without_running() {
    let mut s = agent();
    let dry = |command: &str, args: Value| Request::Run(RunParams::new(command, args).dry());
    let response = s.request(dry("primitive.sphere", json!({})));
    assert!(response.seq.is_none(), "a dry run gets no sequence number");
    let done = success(response, "dry run");
    assert_eq!(
        done.result,
        json!({ "dry_run": true }),
        "says it was a dry run"
    );
    assert!(!done.warnings.is_empty(), "and that nothing changed");
    let stats = s.call("mesh.stats", json!({}));
    assert_eq!(stats.seq, Some(1), "the dry run used no sequence number");
    assert_eq!(
        success(stats, "stats").result["parts"],
        json!([]),
        "nothing was added"
    );
    let err = failure(
        s.request(dry(
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 1 }),
        )),
        "dry",
    );
    assert_eq!(code(err.code), "E_NOT_FOUND", "a dry run still checks");
    let failed_dry = s.request(dry("debug.panic", json!({})));
    assert!(
        failed_dry.seq.is_none(),
        "a failed dry run used no sequence number"
    );
    let failed = s.call("debug.panic", json!({}));
    assert_eq!(
        failed.seq,
        Some(2),
        "a failed call reports the number it used"
    );
    s.ok("history.checkpoint", json!({ "name": "c" }));
    let err = failure(
        s.request(dry("history.checkpoint", json!({ "name": "c" }))),
        "dry checkpoint",
    );
    assert_eq!(
        code(err.code),
        "E_PRECONDITION",
        "a dry run runs the command's own check"
    );
    assert!(err.hint.is_some(), "with the fix: {}", err.message);
    s.ok("primitive.cube", json!({ "name": "a" }));
    let named = json!({ "part": "zz", "direction": [1, 0, 0], "distance": 1 });
    let err = failure(s.request(dry("mesh.translate", named)), "dry, named");
    assert_eq!(
        code(err.code),
        "E_NOT_FOUND",
        "a dry run checks that a named part exists"
    );
}
