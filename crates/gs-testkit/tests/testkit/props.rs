//! Each property behind a claim fails on a known-bad step and passes on a good one, so no
//! claim can pass vacuously.

use gs_protocol::{AttrValues, CommandError, Effect, ErrorCode, Outcome, PlainMesh, Success, Vec3};
use gs_testkit::Scene;
use gs_testkit::claims::props::{
    counts_match, effect_matches, local, matches, no_crash, preserves_topology, restored, sound,
    volume_non_decreasing,
};
use gs_testkit::compare::scene_bytes;
use gs_testkit::fixtures::{cube, patch, tetra};
use serde_json::json;

use crate::data::moved;

fn scene(parts: &[(&str, PlainMesh)]) -> Scene {
    parts
        .iter()
        .map(|(n, m)| ((*n).to_string(), m.clone()))
        .collect()
}

fn error(code: ErrorCode) -> Outcome {
    Outcome::Error(CommandError {
        code,
        message: "boom".into(),
        path: None,
        hint: None,
        suggestions: Vec::new(),
        violations: Vec::new(),
        example: None,
        retryable: false,
        retry_after_ms: None,
    })
}

#[test]
fn crashes_are_refused_unless_the_command_is_meant_to_crash() {
    assert!(
        no_crash(&error(ErrorCode::Internal), false).is_err(),
        "a crash"
    );
    assert!(
        no_crash(&error(ErrorCode::Internal), true).is_ok(),
        "debug.panic may crash"
    );
    assert!(
        no_crash(&error(ErrorCode::NotFound), false).is_ok(),
        "an ordinary error"
    );
    let fine = Outcome::Ok(Success {
        result: json!(null),
        effect: None,
        history_id: None,
        warnings: Vec::new(),
    });
    assert!(no_crash(&fine, false).is_ok(), "a success");
}

#[test]
fn undo_probes_need_success_and_the_exact_scene() {
    let a = scene_bytes(&scene(&[("a", tetra())]));
    let b = scene_bytes(&scene(&[("a", moved(&tetra(), [0.0, 1.0, 0.0]))]));
    assert!(restored(true, &a, &a.clone()).is_ok(), "brought back");
    assert!(restored(false, &a, &a.clone()).is_err(), "failed");
    assert!(restored(true, &a, &b).is_err(), "wrong scene");
    let history = json!({ "undoable": 2, "redoable": 1 });
    assert!(counts_match(&history, 2, 1).is_ok(), "agree");
    assert!(counts_match(&history, 1, 1).is_err(), "undoable differs");
    assert!(counts_match(&history, 2, 0).is_err(), "redoable differs");
    assert!(counts_match(&json!({}), 0, 0).is_err(), "missing counts");
}

/// A sound scene passes; every defect fails; faces flattened by an edit are legal.
#[test]
fn soundness_catches_new_defects_only() {
    let good = scene(&[("a", tetra())]);
    assert_eq!(sound(&good, &good.clone()), Ok(()), "sound");
    let mut nan = tetra();
    nan.positions[0][0] = f32::NAN;
    assert!(sound(&good, &scene(&[("a", nan)])).is_err(), "a NaN");
    let mut flat = tetra();
    flat.positions[3] = [1.0, -1.25, 2.0];
    let flattened = scene(&[("a", flat.clone())]);
    assert_eq!(sound(&good, &flattened), Ok(()), "an edit flattened a face");
    let mut unused = tetra();
    unused.attributes.clear();
    unused.positions.push([9.0, 9.0, 9.0]);
    assert!(
        sound(&scene(&[]), &scene(&[("b", unused)])).is_err(),
        "new data still must be sound"
    );
}

fn effect(
    parts: &[&str],
    topology: bool,
    touched: u64,
    bbox: Option<[[f32; 3]; 2]>,
    noop: bool,
) -> Effect {
    Effect {
        parts: parts.iter().map(ToString::to_string).collect(),
        verts_touched: touched,
        bbox: bbox.map(|[lo, hi]| [Vec3(lo), Vec3(hi)]),
        topology_changed: topology,
        changed: std::collections::BTreeMap::new(),
        noop_reason: noop.then(|| "nothing".to_string()),
    }
}

#[test]
fn effects_must_match_the_scene() {
    let before = scene(&[("a", tetra())]);
    let mut nudged = tetra();
    nudged.positions[1] = [2.0, 3.0, 4.0];
    let after = scene(&[("a", nudged)]);
    let right = effect(
        &["a"],
        false,
        1,
        Some([[2.0, 3.0, 4.0], [2.0, 3.0, 4.0]]),
        false,
    );
    assert_eq!(
        effect_matches(Some(&right), &before, &after, true),
        Ok(()),
        "right"
    );
    let wrong = [
        effect(
            &[],
            false,
            1,
            Some([[2.0, 3.0, 4.0], [2.0, 3.0, 4.0]]),
            false,
        ),
        effect(
            &["a"],
            true,
            1,
            Some([[2.0, 3.0, 4.0], [2.0, 3.0, 4.0]]),
            false,
        ),
        effect(
            &["a"],
            false,
            2,
            Some([[2.0, 3.0, 4.0], [2.0, 3.0, 4.0]]),
            false,
        ),
        effect(
            &["a"],
            false,
            1,
            Some([[2.0, 3.0, 4.0], [2.0, 3.0, 5.0]]),
            false,
        ),
        effect(
            &["a"],
            false,
            1,
            Some([[2.0, 3.0, 4.0], [2.0, 3.0, 4.0]]),
            true,
        ),
    ];
    for bad in &wrong {
        assert!(
            effect_matches(Some(bad), &before, &after, true).is_err(),
            "{bad:?}"
        );
    }
    assert!(
        effect_matches(None, &before, &after, true).is_err(),
        "no effect at all"
    );
    let none = effect(&[], false, 0, None, true);
    assert_eq!(
        effect_matches(Some(&none), &before, &before.clone(), false),
        Ok(()),
        "a no-op says why"
    );
    let silent = effect(&[], false, 0, None, false);
    assert!(
        effect_matches(Some(&silent), &before, &before.clone(), false).is_err(),
        "a silent no-op"
    );
}

#[test]
fn a_change_only_the_hash_shows_lists_nothing_and_is_not_a_no_op() {
    // A part's detail levels around its active level change the scene hash, not `part.get`.
    let before = scene(&[("a", tetra())]);
    let silent = effect(&[], false, 0, None, false);
    assert_eq!(
        effect_matches(Some(&silent), &before, &before.clone(), true),
        Ok(()),
        "nothing to list"
    );
    let none = effect(&[], false, 0, None, true);
    assert!(
        effect_matches(Some(&none), &before, &before.clone(), true).is_err(),
        "a change is not a no-op"
    );
}

/// One change to a scene.
type Change<'a> = &'a dyn Fn(&mut Scene);

#[test]
fn topology_means_parts_faces_counts_and_attribute_layout() {
    let before = scene(&[("a", tetra()), ("b", cube(1))]);
    let mut moved_values = before.clone();
    if let Some(a) = moved_values.get_mut("a") {
        a.positions[0][0] = 7.0;
        if let Some(mask) = a.attributes.get_mut("mask") {
            mask.values = AttrValues::F32(vec![1.0; 4]);
        }
    }
    assert_eq!(
        preserves_topology(&before, &moved_values),
        Ok(()),
        "values may change"
    );
    let edits: [(&str, Change); 6] = [
        ("a part added", &|s| drop(s.insert("c".into(), tetra()))),
        ("a part removed", &|s| drop(s.remove("b"))),
        ("faces rewired", &|s| {
            s.entry("a".into()).or_default().faces.swap(0, 1);
        }),
        ("a vertex added", &|s| {
            s.entry("a".into()).or_default().positions.push([0.0; 3]);
        }),
        ("an attribute renamed", &|s| {
            let a = s.entry("a".into()).or_default();
            let mask = a.attributes.remove("mask");
            a.attributes.extend(mask.map(|m| ("mask2".to_string(), m)));
        }),
        ("an attribute retyped", &|s| {
            let a = s.entry("a".into()).or_default();
            if let Some(mask) = a.attributes.get_mut("mask") {
                mask.values = AttrValues::U32(vec![0; 4]);
            }
        }),
    ];
    for (what, change) in edits {
        let mut after = before.clone();
        change(&mut after);
        assert!(preserves_topology(&before, &after).is_err(), "{what}");
    }
    let mut domain = before.clone();
    if let Some(group) = domain
        .get_mut("a")
        .and_then(|a| a.attributes.get_mut("group"))
    {
        group.domain = gs_protocol::AttrDomain::Vertex;
    }
    assert!(
        preserves_topology(&before, &domain).is_err(),
        "an attribute moved domain"
    );
    let mut shorter = before.clone();
    if let Some(mask) = shorter
        .get_mut("a")
        .and_then(|a| a.attributes.get_mut("mask"))
    {
        mask.values = AttrValues::F32(vec![0.0; 3]);
    }
    assert!(
        preserves_topology(&before, &shorter).is_err(),
        "an attribute got shorter"
    );
}

#[test]
fn locality_by_part() {
    let before = scene(&[("a", tetra()), ("b", tetra())]);
    let mut after = before.clone();
    if let Some(a) = after.get_mut("a") {
        a.positions[0][0] = 9.0;
    }
    let args = json!({ "part": "a" });
    assert_eq!(
        local(&before, &after, &args, "/part", None),
        Ok(()),
        "the named part"
    );
    let other = json!({ "part": "b" });
    assert!(
        local(&before, &after, &other, "/part", None).is_err(),
        "a different part changed"
    );
    assert!(
        local(&before, &after, &json!({}), "/part", None).is_err(),
        "no part named, two parts"
    );
    let mut grown = before.clone();
    grown.insert("c".into(), tetra());
    assert!(
        local(&before, &grown, &args, "/part", None).is_err(),
        "a part appeared"
    );
    let one = scene(&[("a", tetra())]);
    let one_after = scene(&[("a", after["a"].clone())]);
    assert_eq!(
        local(&one, &one_after, &json!({}), "/part", None),
        Ok(()),
        "the only part"
    );
}

fn row() -> PlainMesh {
    let mut m = patch(4);
    m.attributes.clear();
    m
}

/// Moves vertex `v` of the patch up by 0.5.
fn lift(v: usize) -> Scene {
    let mut m = row();
    m.positions[v][1] += 0.5;
    scene(&[("p", m)])
}

#[test]
fn locality_by_support_radius() {
    let before = scene(&[("p", row())]);
    // Vertex 0 is at (-1, 0, -1); vertex 1 at (-1, 0, -0.5); vertex 24 at (1, 0, 1).
    let near = json!({ "part": "p", "c": [-1.0, 0.0, -1.0], "r": 0.5 });
    let support = Some(("/c", "/r"));
    assert_eq!(
        local(&before, &lift(0), &near, "/part", support),
        Ok(()),
        "inside"
    );
    assert_eq!(
        local(&before, &lift(1), &near, "/part", support),
        Ok(()),
        "exactly at the radius"
    );
    assert!(
        local(&before, &lift(24), &near, "/part", support).is_err(),
        "far outside"
    );
    assert!(
        local(&before, &lift(2), &near, "/part", support).is_err(),
        "past the radius"
    );
    let several =
        json!({ "part": "p", "s": [{ "c": [5.0, 5.0, 5.0] }, { "c": [1.0, 0.0, 1.0] }], "r": 0.1 });
    assert_eq!(
        local(
            &before,
            &lift(24),
            &several,
            "/part",
            Some(("/s/*/c", "/r"))
        ),
        Ok(()),
        "any center"
    );
    let percent = json!({ "part": "p", "c": [-1.0, 0.0, -1.0], "r": "18%" });
    let diagonal_share = 0.18 * (4.0f64 + 4.0 + 0.0625).sqrt();
    assert!(diagonal_share > 0.5, "18% of the diagonal reaches vertex 1");
    assert_eq!(
        local(&before, &lift(1), &percent, "/part", support),
        Ok(()),
        "a percentage radius"
    );
    assert!(
        local(&before, &lift(2), &percent, "/part", support).is_err(),
        "but not vertex 2"
    );
    let raised = json!({ "part": "p", "c": [-1.0, 0.4, -1.0], "r": 0.5 });
    let height = local(&before, &lift(1), &raised, "/part", support);
    assert!(height.is_err(), "height counts in the distance");
    let no_radius = json!({ "part": "p", "c": [-1.0, 0.0, -1.0] });
    assert_eq!(
        local(&before, &lift(24), &no_radius, "/part", support),
        Ok(()),
        "no radius given"
    );
    let mut fewer = row();
    fewer.positions.pop();
    let shrunk = scene(&[("p", fewer)]);
    assert!(
        local(&before, &shrunk, &near, "/part", support).is_err(),
        "vertex count changed"
    );
    assert!(
        local(&before, &scene(&[]), &near, "/part", support).is_err(),
        "the part vanished"
    );
}

#[test]
fn volume_may_grow_but_not_shrink() {
    let before = scene(&[("a", tetra()), ("open", patch(2))]);
    let grow = |factor: f32| {
        let mut m = tetra();
        m.positions.iter_mut().flatten().for_each(|c| *c *= factor);
        let mut s = before.clone();
        s.insert("a".into(), m);
        s
    };
    let (checked, shrink) = volume_non_decreasing(&before, &grow(1.5), 0.0).unwrap_or((9, 9.0));
    assert_eq!(
        (checked, shrink),
        (1, 0.0),
        "grew: only the closed part counts"
    );
    assert!(
        volume_non_decreasing(&before, &grow(0.99), 0.0).is_err(),
        "shrank"
    );
    let small = volume_non_decreasing(&before, &grow(0.9999), 1e-3);
    assert!(
        small.as_ref().is_ok_and(|(_, s)| *s > 2e-4 && *s < 4e-4),
        "within tolerance: {small:?}"
    );
    let big = scene(&[("c", cube(1))]);
    let mut shrunk = cube(1);
    shrunk
        .positions
        .iter_mut()
        .flatten()
        .for_each(|c| *c *= 0.9999);
    let relative = volume_non_decreasing(&big, &scene(&[("c", shrunk)]), 1e-3);
    let fraction = relative.as_ref().map(|(_, s)| *s).unwrap_or_default();
    assert!(
        fraction > 2e-4 && fraction < 4e-4,
        "relative to a volume of 8: {relative:?}"
    );
    let flat = |dx: f32| {
        let mut m = cube(1);
        (m.positions.iter_mut()).for_each(|p| *p = [p[0] + dx, 0.3, p[2]]);
        scene(&[("c", m)])
    };
    assert_eq!(
        volume_non_decreasing(&flat(0.0), &flat(0.1), 0.0),
        Ok((1, 0.0)),
        "a flat part's volume is zero, and moving it loses none"
    );
    let mut nan = tetra();
    nan.positions[0][0] = f32::NAN;
    let mut broken = before.clone();
    broken.insert("a".into(), nan);
    let skipped = volume_non_decreasing(&before, &broken, 0.0);
    assert_eq!(
        skipped,
        Ok((0, 0.0)),
        "an unsound part is not closed, so it is left to the validator"
    );
    let mut gone = before.clone();
    gone.remove("a");
    assert_eq!(
        volume_non_decreasing(&before, &gone, 0.0),
        Ok((0, 0.0)),
        "nothing left to compare"
    );
}

#[test]
fn mirrored_results_must_match_vertex_by_vertex() {
    let want = scene(&[("a", tetra())]);
    assert_eq!(matches(&want, &want.clone(), 0.0), Ok(0.0), "the same");
    let mut off = tetra();
    off.positions[2][1] += 1e-3;
    let got = scene(&[("a", off)]);
    let seen = matches(&want, &got, 1e-3);
    assert!(
        seen.as_ref().is_ok_and(|&d| d > 1e-4 && d <= 1e-3),
        "small, relative to the size 5: {seen:?}"
    );
    assert!(matches(&want, &got, 1e-5).is_err(), "too far");
    let mut flipped = tetra();
    flipped.faces[0].swap(0, 1);
    assert!(
        matches(&want, &scene(&[("a", flipped)]), 1.0).is_err(),
        "faces differ"
    );
    assert!(
        matches(&want, &scene(&[("b", tetra())]), 1.0).is_err(),
        "parts differ"
    );
    let mut nan = tetra();
    nan.positions[0][0] = f32::NAN;
    assert!(
        matches(&want, &scene(&[("a", nan)]), 1e9).is_err(),
        "NaN is never close"
    );
    let big = scene(&[("a", moved(&tetra(), [1000.0, 0.0, 0.0]))]);
    let mut big_off = big.clone();
    if let Some(a) = big_off.get_mut("a") {
        a.positions[0][0] += 0.5;
    }
    let relative = matches(&big, &big_off, 1e-3);
    assert!(
        relative.as_ref().is_ok_and(|&d| d < 1e-3),
        "tolerance scales with coordinates: {relative:?}"
    );
}
