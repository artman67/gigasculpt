//! The effect every edit reports: one per call, vertices touched by position or attribute
//! values, topology changes, and a reason whenever nothing changed. (The claims harness checks
//! every edit's effect against the test kit's slow reference; these pin down the cases a random
//! plan rarely builds.)

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, Outcome, PlainMesh, Success};
use serde_json::json;

use gs_testkit::success;

use crate::support::{agent, tagged_triangle};

/// What an edit's effect says changed, part by part.
fn changed(done: &Success) -> BTreeMap<String, Vec<String>> {
    (done.effect.as_ref()).map_or_else(BTreeMap::new, |e| e.changed.clone())
}

/// `names` changed in part `part` and nothing else anywhere.
fn only(part: &str, names: &[&str]) -> BTreeMap<String, Vec<String>> {
    let names = names.iter().map(ToString::to_string).collect();
    BTreeMap::from([(part.to_string(), names)])
}

/// An edit names what it changed in each part: a whole-part move its transform and not its
/// points, a squash its points, a mask fill its mask, an eye its `visible`. A refused call
/// reports no effect and changes nothing, even after an earlier step of its batch wrote.
#[test]
fn an_edit_names_what_it_changed_and_a_refused_one_nothing() {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "name": "ball", "subdivisions": 3 }),
    );
    let calls = [
        (
            "transform.apply",
            json!({ "op": { "move": [0, 0.1, 0] } }),
            "transform",
        ),
        (
            "transform.apply",
            json!({ "op": { "scale": [1, 0.5, 1] } }),
            "positions",
        ),
        ("mask.set", json!({ "value": 1 }), "mask"),
        ("part.set_visible", json!({ "visible": false }), "visible"),
        ("part.set_visible", json!({ "visible": true }), "visible"),
    ];
    for (command, args, name) in calls {
        let done = s.ok(command, args.clone());
        assert_eq!(changed(&done), only("ball", &[name]), "{command} {args}");
    }
    let finer = changed(&s.ok("level.subdivide", json!({})));
    let all = ["faces", "levels", "mask", "positions"].map(str::to_string);
    assert_eq!(
        finer,
        BTreeMap::from([("ball".into(), all.into())]),
        "new faces: all of it"
    );
    let before = s.hash();
    let refused = s.batch(&[
        ("mask.set", json!({ "value": 0 })),
        ("mask.set", json!({ "part": "nothing", "value": 0 })),
    ]);
    assert!(
        matches!(refused.outcome, Outcome::Error(_)),
        "a missing part is refused: {refused:?}"
    );
    assert_eq!(s.hash(), before, "the refused batch changed nothing");
}

#[test]
fn a_batch_is_one_step_with_one_effect() {
    let mut s = agent();
    let done = success(
        s.batch(&[
            ("primitive.cube", json!({ "name": "a" })),
            (
                "mesh.translate",
                json!({ "direction": [0, 0, 1], "distance": 1 }),
            ),
            ("mesh.stats", json!({})),
        ]),
        "batch",
    );
    let outputs = done.result.as_array().cloned().unwrap_or_default();
    assert_eq!(outputs.len(), 3, "one output per step");
    let next = s.call("session.info", json!({}));
    assert_eq!(
        next.seq,
        Some(4),
        "the batch's three steps used sequence numbers 1 to 3"
    );
    assert_eq!(
        outputs[2]["parts"][0]["vertices"],
        json!(8),
        "later steps see earlier ones"
    );
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("a batch reports its effect"));
    assert_eq!(
        (effect.verts_touched, effect.topology_changed),
        (8, true),
        "one effect for all"
    );
    let history = s.history();
    assert_eq!(history["undoable"], json!(1), "one step");
    assert_eq!(
        history["steps"][0]["command"],
        json!("cmd.batch"),
        "named as a batch"
    );
    assert_eq!(
        history["steps"][0]["args"]["commands"][1]["args"]["part"],
        json!("a"),
        "pinned args"
    );
    s.ok("history.undo", json!({}));
    assert_eq!(
        s.query("mesh.stats", json!({}))["parts"],
        json!([]),
        "one undo removes it all"
    );
    let noop = success(s.batch(&[("mesh.stats", json!({}))]), "a batch of queries");
    assert!(
        noop.history_id.is_none() && !noop.warnings.is_empty(),
        "no step, and says why"
    );
}

#[test]
fn restoring_rewired_faces_reports_a_topology_change() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "name": "a" }));
    let mut mesh = s.mesh("a");
    s.ok("history.checkpoint", json!({ "name": "quads" }));
    s.ok("history.undo", json!({}));
    let split = |f: &[u32; 4]| [[f[0], f[1], f[2], u32::MAX], [f[0], f[2], f[3], u32::MAX]];
    mesh.faces = mesh.faces.iter().flat_map(split).collect();
    s.ok("part.create", json!({ "name": "a", "mesh": mesh }));
    let done = s.ok("history.restore", json!({ "name": "quads" }));
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("restore reports an effect"));
    assert!(
        effect.topology_changed,
        "same vertex count, different faces"
    );
    assert_eq!(
        effect.verts_touched, 8,
        "every vertex of a rewired part counts"
    );
}

#[test]
fn edits_that_change_nothing_say_why_and_make_no_step() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let done = s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 0 }),
    );
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("edits report an effect"));
    assert_eq!(
        (effect.verts_touched, effect.topology_changed),
        (0, false),
        "nothing moved"
    );
    assert!(
        effect.parts.is_empty() && effect.bbox.is_none(),
        "nothing touched"
    );
    let reason = effect.noop_reason.unwrap_or_default();
    assert!(
        reason.contains("distance 0") && done.warnings.contains(&reason),
        "the command says why, and the reason is also a warning: {reason}"
    );
    assert!(done.history_id.is_none(), "no step");
    assert_eq!(s.history()["undoable"], json!(1), "only the cube");
    let jitter = s.ok("mesh.jitter", json!({ "amount": 0 }));
    let reason = jitter
        .effect
        .and_then(|e| e.noop_reason)
        .unwrap_or_default();
    assert!(
        reason.contains("up to 0"),
        "a jitter of 0 says so: {reason}"
    );
    let batch = success(
        s.batch(&[
            ("mesh.stats", json!({})),
            (
                "mesh.translate",
                json!({ "direction": [0, 1, 0], "distance": 0 }),
            ),
        ]),
        "a batch that changes nothing",
    );
    let reason = batch.effect.and_then(|e| e.noop_reason).unwrap_or_default();
    assert!(
        reason.contains("step 1: moving by"),
        "a batch names the step: {reason}"
    );
    assert!(
        s.ok("mesh.stats", json!({})).effect.is_none(),
        "queries report no effect"
    );
    s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1 }),
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.history()["redoable"], json!(1), "edits after no-ops undo");
}

fn set_values(mesh: &mut PlainMesh, column: &str, index: usize, value: f32) {
    match mesh.attributes.get_mut(column).map(|a| &mut a.values) {
        Some(AttrValues::F32(values)) => values[index] = value,
        Some(AttrValues::U32(values)) => values[index] = value as u32,
        None => panic!("no column {column}"),
    }
}

/// A change to attribute values alone is a real change: the touched vertices are counted, and
/// there is no no-op reason.
#[test]
fn restoring_attribute_values_reports_the_vertices_touched() {
    for (column, index, value, touched) in [("mask", 1, 0.9, 1), ("group", 0, 8.0, 3)] {
        let mut s = agent();
        let mut mesh = tagged_triangle();
        s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
        let original = s.hash();
        s.ok("history.checkpoint", json!({ "name": "base" }));
        s.ok("history.undo", json!({}));
        set_values(&mut mesh, column, index, value);
        s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
        let done = s.ok("history.restore", json!({ "name": "base" }));
        let effect = done
            .effect
            .unwrap_or_else(|| panic!("restore reports an effect"));
        assert_eq!(
            (
                effect.verts_touched,
                effect.topology_changed,
                effect.noop_reason
            ),
            (touched, false, None),
            "{column}: the vertices whose values changed back"
        );
        assert!(done.history_id.is_some(), "{column}: one step");
        assert_eq!(
            s.hash(),
            original,
            "{column}: the values really changed back"
        );
    }
}

/// A column that changes type but keeps its bit patterns (0.0 as f32, 0 as u32) is still a
/// change to every vertex.
#[test]
fn a_column_that_changes_type_touches_every_vertex() {
    let mut s = agent();
    let mut mesh = tagged_triangle();
    let column = |values| Attribute {
        domain: AttrDomain::Vertex,
        values,
    };
    mesh.attributes
        .insert("weight".into(), column(AttrValues::F32(vec![0.0; 3])));
    s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
    s.ok("history.checkpoint", json!({ "name": "floats" }));
    s.ok("history.undo", json!({}));
    mesh.attributes
        .insert("weight".into(), column(AttrValues::U32(vec![0; 3])));
    s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
    let done = s.ok("history.restore", json!({ "name": "floats" }));
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("restore reports an effect"));
    assert_eq!(
        (effect.verts_touched, effect.noop_reason),
        (3, None),
        "the whole part"
    );
}
