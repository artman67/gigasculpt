//! The history model accepts a correct history and catches each kind of history bug,
//! including research 06's "a new edit forgets to clear redo".

use gs_testkit::claims::model::{Call, Model};
use serde_json::json;

fn state(n: u8) -> Vec<u8> {
    vec![n]
}

/// A model at state 2 after edits 0 -> 1 -> 2.
fn two_edits() -> Model {
    let mut m = Model::new(state(0));
    for n in 1..=2 {
        assert_eq!(
            m.step(Call::Edit, true, true, state(n)),
            Ok(()),
            "edit to {n}"
        );
    }
    m
}

#[test]
fn a_correct_history_passes() {
    let mut m = two_edits();
    assert_eq!((m.undoable(), m.redoable()), (2, 0), "two steps");
    assert_eq!(m.step(Call::Undo, true, false, state(1)), Ok(()), "undo");
    assert_eq!(
        m.step(Call::Undo, true, false, state(0)),
        Ok(()),
        "undo again"
    );
    assert_eq!(
        m.step(Call::Undo, false, false, state(0)),
        Ok(()),
        "nothing more to undo"
    );
    assert_eq!((m.undoable(), m.redoable()), (0, 2), "all undone");
    assert_eq!(m.step(Call::Redo, true, false, state(1)), Ok(()), "redo");
    assert_eq!(
        m.step(Call::Checkpoint("base"), true, false, state(1)),
        Ok(()),
        "save"
    );
    assert_eq!(
        m.step(Call::Edit, true, true, state(5)),
        Ok(()),
        "a new edit"
    );
    assert_eq!(
        (m.undoable(), m.redoable()),
        (2, 0),
        "the new edit cleared redo"
    );
    assert_eq!(
        m.step(Call::Redo, false, false, state(5)),
        Ok(()),
        "nothing to redo"
    );
    assert_eq!(
        m.step(Call::Restore("base"), true, true, state(1)),
        Ok(()),
        "restore"
    );
    assert_eq!(
        m.step(Call::Restore("base"), true, false, state(1)),
        Ok(()),
        "restore again: no-op"
    );
    assert_eq!(
        m.step(Call::Restore("nope"), false, false, state(1)),
        Ok(()),
        "unknown checkpoint"
    );
    assert_eq!(
        m.step(Call::Edit, false, false, state(1)),
        Ok(()),
        "a failed edit"
    );
    assert_eq!(
        m.step(Call::Edit, true, false, state(1)),
        Ok(()),
        "a no-op edit"
    );
    assert_eq!((m.undoable(), m.redoable()), (3, 0), "edit, edit, restore");
}

/// Each wrong outcome, from a model at state 2 with nothing to redo.
#[test]
fn each_kind_of_history_bug_is_caught() {
    let bugs: [(&str, Call, bool, bool, Vec<u8>); 10] = [
        (
            "a failed edit changed the scene",
            Call::Edit,
            false,
            false,
            state(9),
        ),
        (
            "an edit changed the scene with no step",
            Call::Edit,
            true,
            false,
            state(9),
        ),
        (
            "an edit made a step but changed nothing",
            Call::Edit,
            true,
            true,
            state(2),
        ),
        (
            "undo brought back the wrong state",
            Call::Undo,
            true,
            false,
            state(0),
        ),
        (
            "undo failed with something to undo",
            Call::Undo,
            false,
            false,
            state(2),
        ),
        (
            "redo succeeded with nothing to redo",
            Call::Redo,
            true,
            false,
            state(2),
        ),
        (
            "a checkpoint changed the scene",
            Call::Checkpoint("x"),
            true,
            false,
            state(9),
        ),
        (
            "a checkpoint failed",
            Call::Checkpoint("x"),
            false,
            false,
            state(2),
        ),
        (
            "restoring an unknown checkpoint worked",
            Call::Restore("x"),
            true,
            true,
            state(0),
        ),
        (
            "a failed restore changed the scene",
            Call::Restore("x"),
            false,
            false,
            state(0),
        ),
    ];
    for (what, call, ok, made_step, after) in bugs {
        assert!(
            two_edits().step(call, ok, made_step, after).is_err(),
            "{what}"
        );
    }
}

#[test]
fn a_new_edit_that_forgets_to_clear_redo_is_caught() {
    let mut m = two_edits();
    assert_eq!(m.step(Call::Undo, true, false, state(1)), Ok(()), "undo");
    assert_eq!(
        m.step(Call::Edit, true, true, state(7)),
        Ok(()),
        "a new edit"
    );
    let buggy = m.clone().step(Call::Redo, true, false, state(2));
    assert!(buggy.is_err(), "redo brought back the old future");
    assert_eq!(
        m.step(Call::Redo, false, false, state(7)),
        Ok(()),
        "redo must fail"
    );
}

#[test]
fn restores_must_bring_back_the_saved_state() {
    let mut m = two_edits();
    assert_eq!(
        m.step(Call::Checkpoint("base"), true, false, state(2)),
        Ok(()),
        "save 2"
    );
    assert_eq!(m.step(Call::Undo, true, false, state(1)), Ok(()), "undo");
    let wrong = m.clone().step(Call::Restore("base"), true, true, state(1));
    assert!(wrong.is_err(), "a restore that did nothing");
    let elsewhere = m.clone().step(Call::Restore("base"), true, true, state(8));
    assert!(elsewhere.is_err(), "a restore to a different state");
    assert_eq!(
        m.step(Call::Restore("base"), true, true, state(2)),
        Ok(()),
        "a right restore"
    );
    assert_eq!(
        m.step(Call::Undo, true, false, state(1)),
        Ok(()),
        "undo the restore"
    );
    let forward = m.clone().step(Call::Redo, true, false, state(1));
    assert!(forward.is_err(), "redo must bring the restored state back");
    assert_eq!(m.step(Call::Redo, true, false, state(2)), Ok(()), "it does");
}

#[test]
fn commands_map_to_the_calls_the_model_knows() {
    let named = json!({ "name": "base" });
    let cases = [
        ("history.undo", Call::Undo),
        ("history.redo", Call::Redo),
        ("history.checkpoint", Call::Checkpoint("base")),
        ("history.restore", Call::Restore("base")),
        ("mesh.translate", Call::Edit),
        ("history.restored", Call::Edit),
    ];
    for (command, want) in cases {
        assert_eq!(Call::of(command, &named), want, "{command}");
    }
    assert_eq!(
        Call::of("history.restore", &json!({})),
        Call::Restore(""),
        "no name"
    );
}
