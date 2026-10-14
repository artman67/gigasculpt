//! Pure point maps on a part with detail levels (decision 0041, section 6, amended 2026-10-12
//! for DF8): a move, turn, scale, flatten or deformer made with a lower level shown runs on the
//! top, so every level is the map of its own points and the next switch carries nothing; the
//! shown level is what the live drag showed, to the bit; fully masked and hidden points hold on
//! every level; inflate and the surface falloff keep today's result and the carry; one undo
//! restores every level; the same bits on 1 and 24 threads; a frozen part keeps the carry; and
//! a log from before replays to this build's result, `--verify` naming the line that differs.
//! Amended 2026-10-13 for DF8-notes: `mesh.translate` is such a map and runs on the top,
//! `mesh.jitter` is not and keeps the carry, and a mask painted below the top weighs each top
//! point when a move runs there.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gs_protocol::{AttrValues, LogRecord, PlainMesh};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::support::{EngineSession, agent, bits, human, levels, log_records, stroke};

/// `op` on the sphere about a frame off its vertices, as `transform.apply` takes it.
fn on_sphere(op: Value) -> Value {
    let frame = json!({ "origin": [0.05, 0.03, 0.02], "axes": [0.0, 0.0, 0.0, 1.0] });
    let mut call = json!({ "part": "sphere", "frame": frame });
    call["op"] = op;
    call
}

/// A bend of `degrees` toward x along y.
fn bend(degrees: f32) -> Value {
    on_sphere(json!({ "bend": { "axis": "y", "toward": "x", "degrees": degrees } }))
}

/// A soft turn of 45 degrees about z, fading out 1.5 units from the frame's origin, the
/// distance measured `distance`.
fn soft_turn(distance: &str) -> Value {
    let mut call = on_sphere(json!({ "rotate": { "axis": "z", "degrees": 45.0 } }));
    call["falloff"] = json!(0.0);
    call["reach"] = json!(1.5);
    call["distance"] = json!(distance);
    call
}

/// A sphere with three levels and detail drawn on the top, in `s`.
fn three_levels(mut s: Session) -> Session {
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    s
}

/// [`three_levels`] shown at level 1.
fn detailed(s: Session) -> Session {
    let mut s = three_levels(s);
    s.ok("level.set", json!({ "level": 1 }));
    s
}

/// Switches to `level` and gives how many top points the switch carried up.
fn switch(s: &mut Session, level: u64) -> u64 {
    let state = s.query("level.set", json!({ "level": level }));
    state["carried"].as_u64().unwrap_or(u64::MAX)
}

/// Every level's mesh from the base up, the sphere left shown at `back`.
fn every_level(s: &mut Session, back: u64) -> Vec<PlainMesh> {
    let meshes = (1..=3)
        .map(|k| {
            switch(s, k);
            s.mesh("sphere")
        })
        .collect();
    switch(s, back);
    meshes
}

/// Whether two meshes are the same to the bit.
fn same(a: &PlainMesh, b: &PlainMesh) -> bool {
    bits(&a.positions) == bits(&b.positions) && json!(a) == json!(b)
}

/// `command` with `args` run on a part made of `mesh` alone: what these points get by themselves.
fn alone(mesh: &PlainMesh, command: &str, mut args: Value) -> Vec<[u32; 3]> {
    let mut s = agent();
    s.add_part("copy", mesh);
    args["part"] = json!("copy");
    s.ok(command, args);
    bits(&s.mesh("copy").positions)
}

/// `call` run on a part made of `mesh` alone, with the frame and box `pinned` names: the map of
/// these points by themselves.
fn mapped(mesh: &PlainMesh, call: &Value, pinned: &Value) -> Vec<[u32; 3]> {
    let mut call = call.clone();
    call["frame"] = pinned["frame"].clone();
    if let Some(bounds) = pinned.get("box") {
        call["box"] = bounds.clone();
    }
    alone(mesh, "transform.apply", call)
}

/// A move of 0.3 units along (1, 2, 0.5).
fn translate() -> Value {
    json!({ "part": "sphere", "direction": [1.0, 2.0, 0.5], "distance": 0.3 })
}

fn pending(s: &mut Session) -> Value {
    levels(s)["pending"].clone()
}

#[test]
fn a_pure_map_on_a_lower_level_maps_every_level_and_leaves_nothing_to_carry() {
    for call in [bend(60.0), soft_turn("straight")] {
        let mut s = detailed(agent());
        let start = every_level(&mut s, 1);
        let pinned = s.query("transform.apply", call.clone());
        assert_eq!(
            switch(&mut s, 3),
            0,
            "{call}: the next switch carries nothing"
        );
        let after = every_level(&mut s, 1);
        for (k, (before, now)) in start.iter().zip(&after).enumerate() {
            assert!(
                bits(&now.positions) == mapped(before, &call, &pinned),
                "{call}: level {} is the map of its own points",
                k + 1
            );
        }
    }
}

/// Decision 0041, amended 2026-10-13: `mesh.translate` adds one offset to every point, a pure
/// map, so below the top it moves the top; a move of zero stays no step.
#[test]
fn a_translate_on_a_lower_level_moves_the_top_and_one_undo_restores_every_level() {
    let mut s = detailed(agent());
    let start = every_level(&mut s, 1);
    let (before, shown) = (s.hash(), s.mesh("sphere"));
    let still = json!({ "part": "sphere", "direction": [1.0, 0.0, 0.0], "distance": 0.0 });
    let still = s.ok("mesh.translate", still);
    assert!(still.history_id.is_none(), "a move of zero is no step");
    s.ok("mesh.translate", translate());
    assert_eq!(pending(&mut s), json!(false), "nothing waits to be carried");
    let after = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo restores the top");
    assert!(same(&s.mesh("sphere"), &shown), "and the shown level");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo");
    assert_eq!(switch(&mut s, 3), 0, "the next switch carries nothing");
    let now = every_level(&mut s, 1);
    for (k, (was, is)) in start.iter().zip(&now).enumerate() {
        assert!(
            bits(&is.positions) == alone(was, "mesh.translate", translate()),
            "level {} is the move of its own points",
            k + 1
        );
    }
}

/// Decision 0041, section 5: Accept and a gizmo drag's end run the call the live drag showed,
/// and the top path keeps what it showed to the bit.
#[test]
fn the_shown_level_is_the_live_drag_to_the_bit_and_nothing_is_left_pending() {
    let mut s = detailed(human());
    let deformer = |s: &mut Session, phase: &str, transforms: Value| {
        let args = json!({ "phase": phase, "transforms": transforms });
        success(s.as_human("gizmo.deformer", args), phase);
    };
    deformer(&mut s, "open", json!([bend(0.0)]));
    deformer(&mut s, "update", json!([bend(60.0)]));
    let live = s.mesh("sphere");
    success(
        s.as_human("gizmo.deformer", json!({ "phase": "accept" })),
        "accept",
    );
    assert!(
        same(&s.mesh("sphere"), &live),
        "Accept keeps what the drag showed"
    );
    assert_eq!(pending(&mut s), json!(false), "and ran on the top");

    let stretch = json!({ "phase": "update", "transform": { "op": { "scale": [1.4, 1.0, 1.0] } } });
    s.ok("gizmo.drag", stretch);
    let live = s.mesh("sphere");
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    assert!(
        same(&s.mesh("sphere"), &live),
        "a drag's end keeps what it showed"
    );
    assert_eq!(pending(&mut s), json!(false), "and ran on the top");
}

/// The points of `m` every transform holds: masked fully, or every face around them hidden.
fn held(m: &PlainMesh) -> BTreeSet<[u32; 3]> {
    let values = |name: &str| m.attributes.get(name).map(|a| a.values.clone());
    let mask = match values("mask") {
        Some(AttrValues::F32(v)) => v,
        _ => vec![0.0; m.positions.len()],
    };
    let hide = match values("hide") {
        Some(AttrValues::U32(v)) => v,
        _ => vec![0; m.faces.len()],
    };
    let (mut faced, mut shown) = (vec![false; mask.len()], vec![false; mask.len()]);
    for (face, corners) in m.faces.iter().enumerate() {
        for &v in corners.iter().filter(|&&v| v != u32::MAX) {
            faced[v as usize] = true;
            shown[v as usize] |= hide[face] == 0;
        }
    }
    (0..mask.len())
        .filter(|&i| mask[i] >= 1.0 || (faced[i] && !shown[i]))
        .map(|i| m.positions[i].map(f32::to_bits))
        .collect()
}

/// Review 0004, M1: the top decides what moves. A point's bits are the same on every level it
/// is on, so the top's held points are found on each level by their bits.
#[test]
fn fully_masked_and_hidden_points_hold_on_every_level() {
    let mut s = three_levels(agent());
    let front = json!({ "select": { "by": "facing" }, "camera": "front" });
    s.ok("visibility.hide", front);
    s.ok("mask.set", json!({ "value": 1 }));
    s.ok("visibility.show_all", json!({}));
    // A cap whose edge cuts through level 1's faces: level 1 shows less of it hidden.
    let cap = json!({ "select": { "by": "facing", "angle": 50 }, "camera": "right" });
    s.ok("visibility.hide", cap);
    let held = held(&s.mesh("sphere"));
    s.ok("level.set", json!({ "level": 1 }));
    let start = every_level(&mut s, 1);
    s.ok("transform.apply", bend(60.0));
    let mut now = vec![s.mesh("sphere")];
    now.extend(every_level(&mut s, 1).into_iter().skip(1));
    for (k, (before, after)) in start.iter().zip(&now).enumerate() {
        let (b, a) = (bits(&before.positions), bits(&after.positions));
        let kept = (b.iter().zip(&a)).filter(|(p, _)| held.contains(*p));
        let (count, moved) = (kept.clone().count(), kept.filter(|(p, q)| p != q).count());
        let level = k + 1;
        assert!(count > 0, "level {level} has held points");
        assert_eq!(moved, 0, "level {level}: every held point keeps its bits");
        assert!(b != a, "level {level}: the others bend");
    }
}

/// Today's path, so today's bits: inflate reads normals, the surface falloff the shown level's
/// edges, and `mesh.jitter` draws each point's offset from its number, not its position
/// (decision 0041, amended 2026-10-13), so each edits the shown level and is carried. The old
/// logs below run them at level 1 and match main's scene after each. `mesh.smooth` is its own
/// command and never asks for the top. A bend and a translate beside them run on the top.
#[test]
fn inflate_the_surface_falloff_and_jitter_keep_todays_result_and_the_carry() {
    let jitter = json!({ "part": "sphere", "amount": "1%", "seed": 5 });
    let carried = [
        ("transform.apply", on_sphere(json!({ "inflate": 0.05 }))),
        ("transform.apply", soft_turn("surface")),
        ("mesh.jitter", jitter),
    ];
    for (command, args) in carried {
        let mut s = detailed(agent());
        s.ok(command, args.clone());
        assert_eq!(pending(&mut s), json!(true), "{args} waits to be carried");
        assert!(switch(&mut s, 3) > 0, "{args}: the next switch carries it");
    }
    for (command, args) in [
        ("transform.apply", bend(60.0)),
        ("mesh.translate", translate()),
    ] {
        let mut s = detailed(agent());
        s.ok(command, args.clone());
        assert_eq!(pending(&mut s), json!(false), "{args} runs on the top");
    }
}

/// [`detailed`] with a mask painted at level 1 and carried to the top, the top shown.
fn masked_below(s: Session) -> Session {
    let mut s = detailed(s);
    let samples = [[-0.2, 0.0, 1.0], [0.2, 0.0, 1.0]].map(|p| json!({ "position": p }));
    let pen = json!({ "part": "sphere", "brush": "Mask Pen", "size": 0.8, "samples": samples });
    // Each dab takes the mask most of the way to 1; three strokes reach it under the middle.
    for _ in 0..3 {
        s.ok("brush.stroke", pen.clone());
    }
    switch(&mut s, 3);
    s
}

/// Item 204, point 3 (decision 0041, amended 2026-10-13, section 5): a mask painted below the
/// top reaches the top's points at the next switch, and a move made below the top afterwards
/// weighs each top point by its own mask, as the same move made on the top does, to the bit;
/// the points the mask holds fully keep their bits.
#[test]
fn a_mask_painted_below_the_top_weighs_each_top_point_and_does_not_spread() {
    let call = on_sphere(json!({ "move": [0.3, 0.0, 0.0] }));
    let mut on_top = masked_below(agent());
    let top = on_top.mesh("sphere");
    on_top.ok("transform.apply", call.clone());
    let mut s = masked_below(agent());
    switch(&mut s, 1);
    s.ok("transform.apply", call);
    let carried = switch(&mut s, 3);
    let now = bits(&s.mesh("sphere").positions);
    let (held, was) = (held(&top), bits(&top.positions));
    let kept = (was.iter().zip(&now)).filter(|(p, _)| held.contains(*p));
    let (count, moved) = (kept.clone().count(), kept.filter(|(p, q)| p != q).count());
    assert!(count > 0, "the mask holds some top points fully");
    assert_eq!(moved, 0, "every one keeps its bits");
    let made_on_top = bits(&on_top.mesh("sphere").positions);
    assert!(
        now == made_on_top,
        "the top is the same move made on the top"
    );
    assert_eq!(carried, 0, "and nothing was carried");
}

#[test]
fn one_undo_restores_every_level_and_redo_maps_them_again() {
    let mut s = detailed(agent());
    let start = every_level(&mut s, 1);
    let before = s.hash();
    s.ok("transform.apply", bend(60.0));
    let after = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo");
    assert_eq!(
        pending(&mut s),
        json!(false),
        "redo leaves nothing to carry"
    );
    s.ok("history.undo", json!({}));
    let back = every_level(&mut s, 1);
    for (k, (a, b)) in start.iter().zip(&back).enumerate() {
        assert!(same(a, b), "level {} as it was, to the bit", k + 1);
    }
}

#[test]
fn the_same_bits_on_one_and_many_threads() {
    let script = || {
        let mut s = detailed(agent());
        let mut hashes = Vec::new();
        for call in [soft_turn("straight"), bend(60.0)] {
            s.ok("transform.apply", call);
            hashes.push(s.hash());
        }
        let carried = switch(&mut s, 3);
        hashes.push(s.hash());
        s.ok("history.undo", json!({}));
        s.ok("history.undo", json!({}));
        hashes.push(s.hash());
        (hashes, carried)
    };
    let one = gs_par::with_threads(1, script);
    let many = gs_par::with_threads(24, script);
    assert_eq!(one, many, "the same bits on 1 and 24 threads");
    assert_eq!(one.1, 0, "both ran on the top");
}

/// Decision 0030: a frozen part has one level, so a bend is an edit of the base, and unfreezing
/// carries it up as before. The same bend on a twin that is not frozen maps its top.
#[test]
fn a_frozen_part_keeps_the_carry() {
    let mut thawed = detailed(agent());
    let top = every_level(&mut thawed, 1).remove(2);
    let pinned = thawed.query("transform.apply", bend(60.0));
    switch(&mut thawed, 3);
    let mapped_top = bits(&thawed.mesh("sphere").positions);
    assert!(
        mapped_top == mapped(&top, &bend(60.0), &pinned),
        "the twin's top is the map"
    );

    let mut frozen = detailed(agent());
    frozen.ok("level.freeze", json!({}));
    let base = frozen.query("transform.apply", bend(60.0));
    assert_eq!(base, pinned, "the same call on the same base");
    let back = frozen.ok("level.unfreeze", json!({}));
    assert_eq!(back.result["how"], json!("carried"), "unfreezing carries");
    switch(&mut frozen, 3);
    assert!(
        bits(&frozen.mesh("sphere").positions) != mapped_top,
        "the frozen part's top is the carry, not the map"
    );
}

/// The old-log rule (decision 0041, section 6, amended for DF8): a line means what this build
/// does. `golden/bend_on_level_1.jsonl` is a verify-mode log written by main at 5559ec1a, before
/// the top path: a cube with two levels added, noise on the top, level 1 shown, an inflate and a
/// surface turn (today's path, carried to the top and back), a bend, the top shown again. It
/// replays without an error to this build's result; the first scene that differs is after the
/// bend, whose shown level is what it was, the map of level 1's points.
/// To write it again, turn the rule back in its one place (`transform::route::on_top`) and
/// record the same calls.
#[test]
fn a_log_from_before_the_top_path_replays_to_this_builds_result_and_names_the_bend() {
    let bends = |r: &LogRecord| r.args["op"].get("bend").is_some();
    let (first, bend, records) = replay("bend_on_level_1.jsonl", bends);
    let inflates = records
        .iter()
        .any(|r| r.args["op"].get("inflate").is_some());
    assert!(bend.is_some() && inflates, "the log inflates and bends");
    assert_eq!(first, bend, "the first line that differs is the bend");
}

/// The same rule for the two commands DF8 left (decision 0041, amended 2026-10-13, section 4).
/// `golden/translate_on_level_1.jsonl` is a verify-mode log written by main at 1b0f7849: a
/// sphere with two levels added, a stroke on the top, level 1 shown, a jitter carried to the
/// top and back, a translate, the top shown again. The jitter's lines replay to the bit; the
/// first scene that differs is after the translate, whose shown level is what it was. To write
/// it again, turn `on_top` back and record the same calls.
#[test]
fn a_log_from_before_replays_its_jitter_to_the_bit_and_names_the_translate() {
    let moves = |r: &LogRecord| r.command == "mesh.translate";
    let (first, moved, records) = replay("translate_on_level_1.jsonl", moves);
    let jitters = records.iter().any(|r| r.command == "mesh.jitter");
    assert!(moved.is_some() && jitters, "the log jitters and moves");
    assert_eq!(first, moved, "the first line that differs is the translate");
}

/// Replays `golden/<name>`, a verify-mode log, and gives the number of its first line whose
/// scene this build makes differently, the number of the last line `picked` chooses, and the
/// records. Each chosen line's shown level must be what it was: the call on its points alone.
fn replay(
    name: &str,
    picked: impl Fn(&LogRecord) -> bool,
) -> (Option<u32>, Option<u32>, Vec<LogRecord>) {
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/engine/golden");
    let (header, records) = log_records(&log.join(name));
    assert!(header.verify, "every line carries the scene after it");
    let mut s = seeded(header.seed, None);
    let (mut first, mut chosen) = (None, None);
    for (number, record) in (2..).zip(&records) {
        let part = record.args["part"].as_str().unwrap_or_default().to_owned();
        let before = picked(record).then(|| s.mesh(&part));
        let request = record.request().unwrap_or_else(|e| panic!("{e}"));
        success(s.request(request), &format!("log line {number}"));
        if let Some(before) = before {
            chosen = Some(number);
            let shown = bits(&s.mesh(&part).positions);
            let again = alone(&before, &record.command, record.args.clone());
            assert!(shown == again, "line {number}'s shown level is what it was");
        }
        if first.is_none() && record.hash.as_ref() != Some(&s.hash()) {
            first = Some(number);
        }
    }
    (first, chosen, records)
}
