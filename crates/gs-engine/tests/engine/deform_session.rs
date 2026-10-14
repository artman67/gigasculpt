//! The open deformer's session (decision 0041, sections 4 and 5, amended 2026-10-12 for DF4):
//! `gizmo.deformer` keeps one undo step open across handle drags; Accept commits it as one
//! fully resolved `transform.apply` line named after the deformer, Cancel leaves every bit, Reset
//! goes back to identity and stays open, every update applies the whole deformer again from the
//! start, an edit, a history call or another tool accepts it first, undo while it is open is
//! Accept then undo, and a slider pull is one step whose value springs back. Beside it, the
//! held gizmo drag that an edit or undo cuts (the note's third open question).

use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_protocol::Role;
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::support::{EngineSession, agent, human, log_records, windowed};

/// The deformers' frame: unturned, off the sphere's vertices.
const ORIGIN: [f32; 3] = [0.05, 0.03, 0.02];

/// `op` on the sphere about the deformers' frame, as `transform.apply` takes it.
fn on_sphere(op: Value) -> Value {
    let frame = json!({ "origin": ORIGIN, "axes": [0.0, 0.0, 0.0, 1.0] });
    let mut call = json!({ "part": "sphere", "frame": frame });
    call["op"] = op;
    call
}

/// A bend of `degrees` toward x along y.
fn bend(degrees: f32) -> Value {
    on_sphere(json!({ "bend": { "axis": "y", "toward": "x", "degrees": degrees } }))
}

/// A twist of `degrees` about `axis`.
fn twist(axis: &str, degrees: f32) -> Value {
    on_sphere(json!({ "twist": { "axis": axis, "degrees": degrees } }))
}

/// A `gizmo.deformer` call with transforms, run as the human.
fn deformer(s: &mut Session, phase: &str, transforms: &[Value]) {
    let args = json!({ "phase": phase, "transforms": transforms });
    success(s.as_human("gizmo.deformer", args), phase);
}

/// A `gizmo.deformer` call without transforms (lift, accept, cancel, reset), as the human.
fn phase(s: &mut Session, phase: &str) {
    success(
        s.as_human("gizmo.deformer", json!({ "phase": phase })),
        phase,
    );
}

/// Opens a bend, drags a handle to `degrees` and lets go: the deformer is open, nothing held.
fn bent(s: &mut Session, degrees: f32) {
    deformer(s, "open", &[bend(0.0)]);
    deformer(s, "update", &[bend(degrees)]);
    phase(s, "lift");
}

fn state(s: &mut Session) -> Value {
    s.query("session.state", json!({}))
}

fn undoable(s: &mut Session) -> u64 {
    s.history()["undoable"].as_u64().unwrap_or_default()
}

/// A human's session with a sphere.
fn sphere() -> Session {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 5 }));
    s
}

#[test]
fn accept_is_one_undo_step_and_one_resolved_line_named_after_the_deformer() {
    let log = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("deform-accept-{}.jsonl", std::process::id()));
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 5 }));
    let (before, steps) = (s.hash(), undoable(&mut s));
    deformer(&mut s, "open", &[bend(0.0)]);
    for degrees in [20.0, 45.0, 70.0] {
        deformer(&mut s, "update", &[bend(degrees)]);
    }
    phase(&mut s, "lift");
    assert_eq!(
        undoable(&mut s),
        steps,
        "nothing is committed while it is open"
    );
    assert_eq!(state(&mut s)["deformer"], json!("bend"));
    phase(&mut s, "accept");
    let after = s.hash();
    assert_ne!(after, before);
    assert_eq!(undoable(&mut s), steps + 1, "one undo step");
    let now = state(&mut s);
    assert_eq!(
        now["history.undo"],
        json!("Bend"),
        "named after the deformer"
    );
    assert_eq!(now["deformer"], Value::Null, "closed");

    let (_, records) = log_records(&log);
    let lines: Vec<_> = (records.iter())
        .filter(|r| r.command != "primitive.sphere")
        .collect();
    assert!(
        lines.len() == 1 && lines[0].command == "transform.apply",
        "one transform.apply line: {lines:?}"
    );
    let args = &lines[0].args;
    assert_eq!(args["frame"]["origin"], json!(ORIGIN), "the frame");
    assert_eq!(
        args["op"]["bend"]["degrees"],
        json!(70.0),
        "the last update"
    );
    assert!(
        args["box"]["lo"].is_array() && args["box"]["hi"].is_array(),
        "the box is written in: {args}"
    );
    let mut replay = agent();
    replay.replay_log(&log);
    assert_eq!(replay.hash(), after, "the line replays it to the bit");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo puts every bit back");
    std::fs::remove_file(&log).ok();
}

#[test]
fn cancel_leaves_the_part_bit_for_bit() {
    let mut s = sphere();
    let (before, points, steps) = (s.hash(), s.mesh("sphere").positions, undoable(&mut s));
    bent(&mut s, 60.0);
    deformer(&mut s, "update", &[bend(-30.0)]);
    assert_ne!(s.hash(), before, "the deformer bends live");
    phase(&mut s, "cancel");
    assert_eq!(s.hash(), before, "every bit back");
    assert_eq!(s.mesh("sphere").positions, points);
    assert_eq!(undoable(&mut s), steps, "and nothing to undo");
    assert_eq!(state(&mut s)["deformer"], Value::Null, "closed");

    // A turn of the whole part keeps its result in the part's transform; Cancel puts that back.
    let turn = on_sphere(json!({ "rotate": { "axis": "z", "degrees": 0.0 } }));
    deformer(&mut s, "open", &[turn]);
    let turn = on_sphere(json!({ "rotate": { "axis": "z", "degrees": 40.0 } }));
    deformer(&mut s, "update", &[turn]);
    assert_ne!(s.hash(), before, "the part turns live");
    phase(&mut s, "cancel");
    assert_eq!(s.hash(), before, "every bit back");
}

#[test]
fn reset_puts_the_arguments_to_identity_and_keeps_the_session_open() {
    let mut s = sphere();
    let before = s.hash();
    bent(&mut s, 50.0);
    let opened = state(&mut s);
    phase(&mut s, "reset");
    assert_eq!(s.hash(), before, "the part as it was");
    let now = state(&mut s);
    assert_eq!(now["deformer"], json!("bend"), "still open");
    let applied = &now["deformer.transforms"][0];
    assert_eq!(applied["op"]["bend"]["degrees"], json!(0.0), "at identity");
    assert_eq!(
        applied["box"], opened["deformer.transforms"][0]["box"],
        "the box kept"
    );
    deformer(&mut s, "update", &[bend(30.0)]);
    assert_ne!(s.hash(), before, "a handle drag bends again");
    phase(&mut s, "accept");
    assert_eq!(state(&mut s)["history.undo"], json!("Bend"));
}

/// The framework's last Check clause through the session (claim `mask_holds`): with the back
/// half of the sphere fully masked, a bend leaves every masked point where it was, live and once
/// accepted, and moves the others.
#[test]
fn a_deformer_leaves_fully_masked_points_untouched_live_and_accepted() {
    let mut s = sphere();
    let front = json!({ "select": { "by": "facing" }, "camera": "front" });
    s.ok("visibility.hide", front);
    s.ok("mask.set", json!({ "value": 1 }));
    s.ok("visibility.show_all", json!({}));
    let before = s.mesh("sphere").positions;
    let mesh = s.query("part.get", json!({ "part": "sphere" }));
    let masked: Vec<bool> = (mesh["attributes"]["mask"]["values"]["f32"].as_array())
        .map(|m| m.iter().map(|v| v.as_f64() >= Some(1.0)).collect())
        .unwrap_or_default();
    assert!(
        masked.contains(&true) && masked.contains(&false),
        "half masked"
    );
    let check = |now: &[[f32; 3]], when: &str| {
        let moved = |i: usize| now[i].map(f32::to_bits) != before[i].map(f32::to_bits);
        let held = (0..before.len()).filter(|&i| masked[i]).all(|i| !moved(i));
        assert!(held, "{when}: every fully masked point is where it was");
        assert!((0..before.len()).any(moved), "{when}: the others bend");
    };
    bent(&mut s, 60.0);
    check(&s.mesh("sphere").positions, "live");
    phase(&mut s, "accept");
    check(&s.mesh("sphere").positions, "accepted");
}

#[test]
fn every_update_applies_the_whole_deformer_again_from_the_start() {
    let mut a = sphere();
    deformer(&mut a, "open", &[bend(0.0)]);
    for degrees in [25.0, 40.0] {
        deformer(&mut a, "update", &[bend(degrees)]);
    }
    phase(&mut a, "lift");
    for degrees in [55.0, 60.0] {
        deformer(&mut a, "update", &[bend(degrees)]);
    }
    phase(&mut a, "lift");
    let mut b = sphere();
    deformer(&mut b, "open", &[bend(0.0)]);
    deformer(&mut b, "update", &[bend(60.0)]);
    assert_eq!(
        a.mesh("sphere").positions,
        b.mesh("sphere").positions,
        "two handle drags give the bits of one drag to the same arguments"
    );
    phase(&mut a, "accept");
    phase(&mut b, "accept");
    let mut c = sphere();
    c.ok("transform.apply", bend(60.0));
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.hash(), c.hash(), "and of the call itself");
}

#[test]
fn an_edit_a_history_call_or_another_tool_accepts_an_open_deformer_first() {
    for (command, args) in [
        ("primitive.cube", json!({ "name": "c" })),
        ("history.checkpoint", json!({ "name": "c" })),
        ("brush.set", json!({ "brush": "Clay" })),
    ] {
        let mut s = sphere();
        let steps = undoable(&mut s);
        bent(&mut s, 50.0);
        let live = s.mesh("sphere").positions;
        s.ok(command, args);
        assert_eq!(state(&mut s)["deformer"], Value::Null, "{command} accepts");
        assert_eq!(
            s.mesh("sphere").positions,
            live,
            "{command}: kept as it was"
        );
        let history = s.history();
        let bends = (history["steps"].as_array().into_iter().flatten())
            .filter(|step| step["command"] == "transform.apply")
            .count();
        assert_eq!(bends, 1, "{command}: one step for the deformer: {history}");
        assert!(undoable(&mut s) > steps, "{command}");
    }

    // Turning the gizmo tool off, in a session with a window.
    let mut s = windowed();
    s.ok("primitive.sphere", json!({ "subdivisions": 5 }));
    s.ok("ui.gizmo", json!({ "tool": true }));
    bent(&mut s, 50.0);
    s.ok("ui.gizmo", json!({ "tool": false }));
    assert_eq!(state(&mut s)["deformer"], Value::Null, "the tool accepts");
    assert_eq!(state(&mut s)["history.undo"], json!("Bend"));
}

#[test]
fn undo_while_a_deformer_is_open_lands_where_cancel_would_and_keeps_it_to_redo() {
    let mut s = sphere();
    let before = s.hash();
    bent(&mut s, 50.0);
    let live = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "where Cancel would land");
    let now = state(&mut s);
    assert_eq!(now["deformer"], Value::Null, "closed");
    assert_eq!(now["history.redo"], json!("Bend"), "the deformer to redo");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), live, "redo brings it back");
}

#[test]
fn one_slider_pull_is_one_step_and_its_value_springs_back() {
    let log = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("deform-slider-{}.jsonl", std::process::id()));
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 5 }));
    let steps = undoable(&mut s);
    // Twist with X and Z lit: one transform per axis, in one session.
    let pull = |degrees| [twist("x", degrees), twist("z", degrees)];
    for pulled in 1..=2 {
        deformer(&mut s, "open", &pull(0.0));
        for degrees in [10.0, 25.0, 40.0] {
            deformer(&mut s, "update", &pull(degrees));
        }
        phase(&mut s, "accept");
        let now = state(&mut s);
        assert_eq!(now["deformer"], Value::Null, "the value springs back");
        assert_eq!(undoable(&mut s), steps + pulled, "one step a pull");
        assert_eq!(now["history.undo"], json!("Twist and 1 more"));
    }
    let after = s.hash();
    let (_, records) = log_records(&log);
    let pulls: Vec<_> = (records.iter())
        .filter(|r| r.command != "primitive.sphere")
        .collect();
    assert_eq!(pulls.len(), 2, "a line a pull: {pulls:?}");
    for line in pulls {
        let steps = line.args["commands"].as_array().map(Vec::as_slice);
        let Some([x, z]) = steps else {
            panic!("a batch of two: {line:?}");
        };
        assert_eq!(x["args"]["op"]["twist"]["degrees"], json!(40.0));
        assert_eq!(z["args"]["op"]["twist"]["axis"], json!("z"));
    }
    let mut replay = agent();
    replay.replay_log(&log);
    assert_eq!(
        replay.hash(),
        after,
        "the pulls add up, and replay to the bit"
    );
    std::fs::remove_file(&log).ok();
}

/// The note's third open question: a human edit or undo in the middle of a held gizmo drag
/// commits it, and the window's later updates of that drag, which carry the whole transform
/// since the press, are dropped until it lets go, as a cut stroke's samples are. A held handle
/// of an open deformer is the same: the edit accepts it, and its later updates change nothing.
#[test]
fn an_edit_or_undo_during_a_held_drag_cuts_it_and_drops_the_rest_of_it() {
    let stretch = |x: f32| {
        let transform = json!({ "part": "sphere", "op": { "scale": [x, 1.0, 1.0] } });
        json!({ "phase": "update", "transform": transform })
    };
    let width =
        |s: &mut Session| (s.mesh("sphere").positions.iter()).fold(0.0f32, |w, p| w.max(p[0]));
    let mut s = sphere();
    s.ok("primitive.cube", json!({ "name": "c" }));
    let (wide, steps) = (width(&mut s), undoable(&mut s));
    s.ok("gizmo.drag", stretch(1.5));
    let edit = json!({ "part": "c", "direction": [0, 0, 1], "distance": 0.5 });
    s.ok("mesh.translate", edit);
    let cut = s.hash();
    s.ok("gizmo.drag", stretch(2.0));
    let w = width(&mut s);
    assert_eq!(
        s.hash(),
        cut,
        "the rest of the drag is dropped: {w} wide from {wide}"
    );
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    let w = width(&mut s);
    assert!(
        (w - 1.5 * wide).abs() < 1e-5,
        "stretched once: {w} from {wide}"
    );
    assert_eq!(
        undoable(&mut s),
        steps + 2,
        "the drag up to the edit, then the edit"
    );
    s.ok("gizmo.drag", stretch(2.0));
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    let w = width(&mut s);
    assert!(
        (w - 3.0 * wide).abs() < 1e-4,
        "the next drag is a new one: {w}"
    );

    let mut s = sphere();
    let before = s.hash();
    s.ok("gizmo.drag", stretch(1.5));
    let held = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo takes the drag back");
    s.ok("gizmo.drag", stretch(2.0));
    assert_eq!(s.hash(), before, "the rest of the drag is dropped");
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    assert_eq!(s.hash(), before);
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), held, "the drag up to the undo, to redo");

    let mut s = sphere();
    deformer(&mut s, "open", &[bend(0.0)]);
    deformer(&mut s, "update", &[bend(40.0)]);
    s.ok("primitive.cube", json!({ "name": "c" }));
    assert_eq!(
        state(&mut s)["deformer"],
        Value::Null,
        "the edit accepts it"
    );
    let accepted = s.hash();
    deformer(&mut s, "update", &[bend(80.0)]);
    phase(&mut s, "lift");
    assert_eq!(s.hash(), accepted, "its later updates change nothing");
}
