//! Each part's boolean operation and group start (decision 0057, section 5): `part.set_operation`
//! sets them on one part as one undo step, `part.list` and `part.get` report them, the scene
//! hash and the effect see them, and copies follow the note's copy rule. Nothing is cut yet
//! (LB5 builds `boolean.apply`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use gs_protocol::ErrorCode;
use gs_testkit::Session;
use gs_testkit::claims::run::Harness;
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::support::{EngineSession, agent};

/// A sphere and then a cube, which is active (the last part made, decision 0033).
fn two(mut s: Session) -> Session {
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({}));
    s
}

/// Part `name`'s operation and start, as `part.list` reports them.
fn marks(s: &mut Session, name: &str) -> (Value, Value) {
    let list = s.query("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    let row = (rows.into_iter())
        .find(|row| row["name"] == json!(name))
        .unwrap_or_else(|| panic!("no part {name} in {list}"));
    (row["operation"].clone(), row["start"].clone())
}

/// `operation` and `start` as `marks` gives them.
fn marked(operation: &str, start: bool) -> (Value, Value) {
    (json!(operation), json!(start))
}

/// `name` changed in part `part` and nothing else anywhere.
fn only(part: &str, name: &str) -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(part.to_string(), vec![name.to_string()])])
}

/// Either field is one undo step on the part named, or on the active part when none is; undo
/// and redo give back each state's hash, and the logged lines replay to the same scene.
#[test]
fn set_operation_marks_one_part_in_one_undo_step_and_its_log_replays() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("operation-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("operation.jsonl");
    let mut s = two(seeded(1, Some(log.clone())));
    let before = s.hash();
    let cut = s.ok(
        "part.set_operation",
        json!({ "part": "sphere", "operation": "difference" }),
    );
    assert_eq!(
        cut.result,
        json!({ "part": "sphere", "operation": "difference", "start": false }),
        "the part as it is now"
    );
    let once = s.hash();
    let started = s.ok("part.set_operation", json!({ "start": true }));
    assert_eq!(
        started.result["part"],
        json!("cube"),
        "left out: the active part"
    );
    let twice = s.hash();
    assert_eq!(
        s.history()["undoable"],
        json!(4),
        "one step each, after the two parts"
    );
    let mut replay = seeded(2, None);
    replay.replay_log(&log);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(replay.hash(), twice, "the replay");
    for (step, hash) in [
        ("undo", &once),
        ("undo", &before),
        ("redo", &once),
        ("redo", &twice),
    ] {
        s.ok(&format!("history.{step}"), json!({}));
        assert_eq!(&s.hash(), hash, "{step}");
    }
    assert_eq!(marks(&mut s, "sphere"), marked("difference", false));
    assert_eq!(marks(&mut s, "cube"), marked("union", true));
}

/// An operation the command does not know is refused by its argument, a missing part by its
/// name, and either leaves the scene as it was; a call that sets what the part already has, or
/// nothing, says so and makes no step, as `part.set_visible` does.
#[test]
fn a_bad_operation_is_refused_and_a_call_that_changes_nothing_says_so() {
    let mut s = two(agent());
    let e = s.err("part.set_operation", json!({ "operation": "subtract" }));
    assert_eq!(
        (e.code, e.path.as_deref()),
        (ErrorCode::ArgsInvalid, Some("/operation")),
        "{e:?}"
    );
    let e = s.err(
        "part.set_operation",
        json!({ "part": "nothing", "operation": "union" }),
    );
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    s.ok("part.set_operation", json!({ "operation": "intersect" }));
    let steps = s.history()["undoable"].clone();
    for args in [
        json!({ "operation": "intersect" }),
        json!({ "start": false }),
        json!({ "operation": "intersect", "start": false }),
        json!({}),
    ] {
        let done = s.ok("part.set_operation", args.clone());
        let reason = (done.effect.as_ref()).and_then(|e| e.noop_reason.clone());
        let reason = reason.unwrap_or_default();
        assert!(reason.contains("cube"), "{args}: names the part: {reason}");
        assert!(done.history_id.is_none(), "{args}: no step");
    }
    assert_eq!(s.history()["undoable"], steps, "nothing to undo");
}

/// `part.list` gives every row both fields; `part.get` gives them beside the mesh data, left out
/// at their defaults, so what it gives a part that was never marked is still exactly what
/// `part.create` takes.
#[test]
fn part_list_and_part_get_report_operation_and_start() {
    let mut s = two(agent());
    assert_eq!(marks(&mut s, "sphere"), marked("union", false));
    assert_eq!(marks(&mut s, "cube"), marked("union", false));
    s.ok(
        "part.set_operation",
        json!({ "part": "sphere", "start": true }),
    );
    s.ok(
        "part.set_operation",
        json!({ "part": "cube", "operation": "intersect", "start": true }),
    );
    assert_eq!(marks(&mut s, "sphere"), marked("union", true));
    assert_eq!(marks(&mut s, "cube"), marked("intersect", true));
    let got = s.query("part.get", json!({ "part": "cube" }));
    assert_eq!(
        (&got["operation"], &got["start"]),
        (&json!("intersect"), &json!(true))
    );
    assert!(got["positions"].is_array(), "beside the mesh data: {got}");
    let plain = s.query("part.get", json!({ "part": "sphere" }));
    assert_eq!(plain["start"], json!(true));
    assert!(
        plain.get("operation").is_none(),
        "a union is left out: {plain}"
    );
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    let unmarked = s.query("part.get", json!({ "part": "sphere" }));
    s.ok("part.create", json!({ "name": "again", "mesh": unmarked }));
}

/// The scene hash changes with each field and comes back once both are at their defaults (a
/// scene that never used them hashes as before them), and the effect names the field that
/// changed and touches no vertex.
#[test]
fn the_hash_and_the_effect_see_each_field() {
    let mut s = two(agent());
    let plain = s.hash();
    let mut seen = vec![plain.clone()];
    for (args, name) in [
        (json!({ "operation": "difference" }), "operation"),
        (json!({ "operation": "intersect" }), "operation"),
        (json!({ "start": true }), "start"),
        (json!({ "operation": "union" }), "operation"),
    ] {
        let done = s.ok("part.set_operation", args.clone());
        let effect = done.effect.unwrap_or_else(|| panic!("{args}: an effect"));
        assert_eq!(effect.changed, only("cube", name), "{args}");
        assert_eq!(
            (
                effect.verts_touched,
                effect.topology_changed,
                effect.parts.len()
            ),
            (0, false, 0),
            "{args}: no vertex"
        );
        let hash = s.hash();
        assert!(!seen.contains(&hash), "{args}: a hash of its own");
        seen.push(hash);
    }
    s.ok("part.set_operation", json!({ "start": false }));
    assert_eq!(s.hash(), plain, "both at their defaults");
}

/// Turning Voxel mode off changes only the part's switch, and the effect names it, as it names
/// the other properties.
#[test]
fn turning_voxel_mode_off_names_voxel_mode() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    s.ok("mesh.voxel_mode", json!({ "on": true }));
    let off = s.ok("mesh.voxel_mode", json!({ "on": false })).effect;
    let named = off.map(|e| e.changed).unwrap_or_default();
    assert_eq!(named, only("sphere", "voxel_mode"));
}

/// The copy rule: a duplicate (placed or not) and a mirror copy are unions that start no group,
/// whatever the part was; split pieces keep the part's operation and start no group, while the
/// part itself keeps both.
#[test]
fn copies_start_no_group_and_duplicates_and_mirror_copies_are_unions() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    s.ok("group.make", json!({ "maker": { "by": "normals" } }));
    s.ok(
        "part.set_operation",
        json!({ "operation": "difference", "start": true }),
    );
    s.ok("part.duplicate", json!({ "part": "cube", "name": "copy" }));
    let moved = json!({ "op": { "move": [3.0, 0.0, 0.0] } });
    s.ok(
        "part.duplicate",
        json!({ "part": "cube", "name": "moved", "transform": moved }),
    );
    s.ok(
        "part.mirror_copy",
        json!({ "part": "cube", "name": "mirrored" }),
    );
    let pieces = s.ok("part.split", json!({ "part": "cube", "by": "groups" }));
    for name in ["copy", "moved", "mirrored"] {
        assert_eq!(marks(&mut s, name), marked("union", false), "{name}");
    }
    let parts = pieces.result["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(parts.len(), 6, "a piece per side");
    assert_eq!(
        marks(&mut s, "cube"),
        marked("difference", true),
        "the part"
    );
    for piece in &parts[1..] {
        let name = piece["part"].as_str().unwrap_or_default();
        assert_eq!(marks(&mut s, name), marked("difference", false), "{name}");
    }
}

/// `part.set_operation` keeps the claims decision 0057 (section 7) gives it, `preserves_topology`
/// and `local` to the part named, and the three every edit keeps, each checked on random plans
/// of it alone over the standard fixtures.
#[test]
fn set_operation_keeps_its_claims() {
    let catalog = Session::new().engine.catalog().clone();
    let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
    let only = harness.only(&["part.set_operation"]);
    let tally = only.run_until_checked().unwrap_or_else(|e| panic!("{e}"));
    for claim in [
        "preserves_topology",
        "local",
        "deterministic",
        "undo_exact",
        "atomic",
    ] {
        assert!(
            tally.count("part.set_operation", claim) >= 1,
            "{claim} was checked"
        );
    }
}
