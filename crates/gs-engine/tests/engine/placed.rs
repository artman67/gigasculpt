//! A part's transform through commands (decision 0029). Differential checks: a whole-part move,
//! turn and even scale kept in the part's transform give the same points as the same calls kept
//! in the points, once `part.apply_transform` writes the transform in (which changes no bit
//! `part.get` shows); picks, strokes and queries on a moved part agree with the same part
//! baked. And properties: a routed move costs bytes of undo at any size, the mirror map
//! does not change when its part moves and a moved part mirrors across its own plane, the locked Mesh to Axis and Reset Orientation move the
//! part with its gizmo, a pose survives a swap of its part and a failed call, and a part
//! scaled to nothing or far down can still be framed and picked.

use gs_testkit::{Session, code, failure};
use serde_json::{Value, json};

use crate::support::{agent, human, worst};

/// The calls both routes run: a turn about a slanted axis, a move and an even scale, about a
/// frame on the plane x = 0, so that turning on a mirror across x (which leaves out a plane the
/// frame sits on, decision 0027, section 6) changes nothing but where the result is kept.
fn calls(mirror: bool) -> Vec<Value> {
    let frame = json!({ "origin": [0.0, 0.2, -0.1], "axes": [0.0, 0.0, 0.0, 1.0] });
    let ops = [
        json!({ "rotate": { "axis": [1.0, 2.0, 3.0], "degrees": 35 } }),
        json!({ "move": [0.3, -0.2, 0.7] }),
        json!({ "scale": [1.5, 1.5, 1.5] }),
    ];
    let mut out = Vec::new();
    for op in ops {
        let mut args = json!({ "frame": frame, "op": op });
        if mirror {
            args["symmetry"] = json!({ "mirror": { "x": true } });
        }
        out.push(args);
    }
    out
}

fn positions(s: &mut Session, part: &str) -> Vec<[f32; 3]> {
    s.mesh(part).positions
}

/// The bytes the newest undo step holds.
fn newest_step_bytes(s: &mut Session) -> u64 {
    s.history()["steps"][0]["bytes"]
        .as_u64()
        .unwrap_or(u64::MAX)
}

/// The largest difference between the numbers of two JSON values of the same shape; anything
/// else must be equal.
fn gap(a: &Value, b: &Value) -> f64 {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap_or(f64::NAN) - y.as_f64().unwrap_or(f64::NAN)).abs()
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => (x.iter().zip(y))
            .map(|(x, y)| gap(x, y))
            .fold(0.0, f64::max),
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => (x.iter())
            .map(|(k, v)| y.get(k).map_or(f64::INFINITY, |w| gap(v, w)))
            .fold(0.0, f64::max),
        _ if a == b => 0.0,
        _ => f64::INFINITY,
    }
}

#[test]
fn both_routes_give_the_same_points_after_apply_transform() {
    let run = |mirror: bool| {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
        let mut bytes = Vec::new();
        for args in calls(mirror) {
            s.ok("transform.apply", args);
            bytes.push(newest_step_bytes(&mut s));
        }
        (s, bytes)
    };
    let (mut placed, small) = run(false);
    let (mut points, large) = run(true);
    // 3458 points: a step that keeps the old positions holds at least 12 bytes a point.
    assert!(
        small.iter().all(|&b| b < 1024),
        "transform steps: {small:?}"
    );
    assert!(
        large.iter().all(|&b| b >= 3458 * 12),
        "point steps: {large:?}"
    );

    let shown = positions(&mut placed, "sphere");
    let before = placed.hash();
    placed.ok("part.apply_transform", json!({}));
    assert_eq!(
        positions(&mut placed, "sphere"),
        shown,
        "the same bits, written in"
    );
    assert_ne!(
        placed.hash(),
        before,
        "the points and the transform changed"
    );
    let again = placed.ok("part.apply_transform", json!({}));
    assert!(
        !again.warnings.is_empty(),
        "a part at the identity is left alone"
    );

    // Each route rounds differently: about 1e-7 per call on a sphere 3 units across.
    let gap = worst(&shown, &positions(&mut points, "sphere"));
    assert!(gap < 2e-6, "the two routes are {gap} apart");

    placed.ok("history.undo", json!({}));
    assert_eq!(placed.hash(), before, "undo brings the transform back");
    assert_eq!(positions(&mut placed, "sphere"), shown);
}

#[test]
fn a_routed_move_holds_bytes_of_undo_at_any_size() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 256 }));
    s.ok("level.subdivide", json!({}));
    let stats = s.query("mesh.stats", json!({}));
    let points = stats["parts"][0]["vertices"].as_u64().unwrap_or(0);
    assert!(points > 1_500_000, "{points} points");
    let shift = |mirror: bool| {
        let mut args = json!({ "op": { "move": [0.5, 0.0, 0.0] } });
        if mirror {
            args["symmetry"] = json!({ "mirror": { "x": true } });
        }
        args
    };
    s.ok("transform.apply", shift(false));
    let routed = newest_step_bytes(&mut s);
    s.ok("transform.apply", shift(true));
    let edited = newest_step_bytes(&mut s);
    assert!(routed < 1024, "a routed move holds {routed} bytes");
    assert!(
        edited >= points * 12,
        "a move of the points holds {edited} bytes"
    );
}

#[test]
fn a_moved_part_answers_as_the_same_part_baked() {
    let setup = |bake: bool| {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
        for args in calls(false) {
            s.ok("transform.apply", args);
        }
        if bake {
            s.ok("part.apply_transform", json!({}));
        }
        s
    };
    let (mut moved, mut baked) = (setup(false), setup(true));
    // Framing is left out: it boxes a part's node boxes mapped by their corners, so a turned
    // part frames a little wider than the same part baked (decision 0029, section 4).
    let ray = |x: f32| json!({ "origin": [x, 6.0, 0.5], "direction": [0.0, -1.0, 0.0] });
    let ask: [(&str, Value); 4] = [
        ("mesh.bounds", json!({})),
        ("mesh.stats", json!({})),
        ("pick.ray", ray(0.4)),
        ("pick.ray", ray(0.6)),
    ];
    let mut hits = Vec::new();
    for (command, args) in ask {
        let (a, b) = (
            moved.ok(command, args.clone()).result,
            baked.ok(command, args).result,
        );
        let gap = gap(&a, &b);
        assert!(gap < 1e-5, "{command} answers {gap} apart:\n{a}\n{b}");
        if command == "pick.ray" {
            assert!(a["hit"].is_object(), "the ray meets the part: {a}");
            hits.push(json!({ "position": a["hit"]["position"] }));
        }
    }

    // Strokes along the two hits, in the part's space on one and in the scene on the other.
    // Round tips: a square tip's first dab is turned by a part axis (decision 0029, section 4).
    for brush in ["Draw", "Inflate", "Smooth"] {
        let stroke = json!({ "brush": brush, "size": "20%", "samples": hits });
        let before = positions(&mut moved, "sphere");
        moved.ok("brush.stroke", stroke.clone());
        let changed = worst(&before, &positions(&mut moved, "sphere"));
        assert!(changed > 1e-4, "{brush} moved points by {changed}");
        baked.ok("brush.stroke", stroke);
        let apart = worst(
            &positions(&mut moved, "sphere"),
            &positions(&mut baked, "sphere"),
        );
        assert!(apart < 1e-5, "{brush} lands {apart} apart");
    }

    // Without a turn the mapped node boxes are the baked ones, so framing agrees too.
    let framed = |bake: bool| {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
        for args in &calls(false)[1..] {
            s.ok("transform.apply", args.clone());
        }
        if bake {
            s.ok("part.apply_transform", json!({}));
        }
        s.ok("view.frame", json!({}));
        s.query("view.get", json!({}))["camera"].clone()
    };
    let (a, b) = (framed(false), framed(true));
    assert!(
        gap(&a, &b) < 1e-5,
        "framing puts the camera apart:\n{a}\n{b}"
    );
}

#[test]
fn the_mirror_map_stays_when_its_part_moves() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let mapped = s.ok("symmetry.map", json!({ "axis": "x" })).result;
    let map = |s: &mut Session| serde_json::to_value(s.mesh("sphere").attributes).ok();
    let before = map(&mut s);
    for args in calls(false) {
        s.ok("transform.apply", args);
    }
    assert_eq!(map(&mut s), before, "the map is untouched");
    let again = s.ok("symmetry.map", json!({ "axis": "x" })).result;
    assert_eq!(again, mapped, "the moved part still pairs the same way");
}

/// Decision 0029, section 7, and the Check of the catalog's `canvas.local-symmetry`: a sphere
/// moved 2 along x by its transform mirrors a stroke across its own plane, x = 2; the same
/// sphere made there, at the identity, mirrors across the scene's, x = 0, where the mirrored
/// dab finds no surface.
#[test]
fn a_moved_part_mirrors_across_its_own_plane() {
    let touched = |routed: bool| {
        let mut s = agent();
        if routed {
            s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
            s.ok(
                "transform.apply",
                json!({ "op": { "move": [2.0, 0.0, 0.0] } }),
            );
        } else {
            s.ok(
                "primitive.sphere",
                json!({ "subdivisions": 16, "center": [2, 0, 0] }),
            );
        }
        let before = positions(&mut s, "sphere");
        let stroke = json!({
            "brush": "Draw",
            "size": 0.3,
            "samples": [{ "position": [2.6, 0.0, 0.8] }],
            "symmetry": { "mirror": { "x": true } },
        });
        s.ok("brush.stroke", stroke);
        let after = positions(&mut s, "sphere");
        let xs = (before.iter().zip(&after))
            .filter(|(p, q)| p.map(f32::to_bits) != q.map(f32::to_bits))
            .map(|(p, _)| p[0]);
        xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
    };
    let (low, high) = touched(true);
    assert!(
        low < 1.7 && high > 2.3,
        "both sides of x = 2 moved: x from {low} to {high}"
    );
    let (low, high) = touched(false);
    assert!(
        low > 2.0,
        "only the dab's side moved: x from {low} to {high}"
    );
}

#[test]
fn mesh_to_axis_and_reset_orientation_move_the_part_with_its_gizmo() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let start = positions(&mut s, "cube");
    // A gizmo on the cube's corner, turned 30 degrees about z, turns the cube with it.
    let corner = [0.5f32, 0.5, 0.5];
    // Half of 30 degrees: sin and cos of 15 degrees.
    let axes = [0.0, 0.0, 0.258_819_04, 0.965_925_8];
    s.ok("gizmo.place", json!({ "origin": corner, "axes": axes }));
    let turn = json!({ "op": { "rotate": { "axis": "z", "degrees": 30 } } });
    s.ok("transform.apply", turn);
    let turned = positions(&mut s, "cube");
    assert!(worst(&turned, &start) > 0.1, "the cube turned");

    let square = s.ok("gizmo.reset_orientation", json!({}));
    assert!(newest_step_bytes(&mut s) < 1024, "one transform step");
    assert_eq!(square.result["pose"]["axes"], json!([0.0, 0.0, 0.0, 1.0]));
    let gap = worst(&positions(&mut s, "cube"), &start);
    assert!(gap < 1e-6, "the cube is square to the scene again: {gap}");

    let moved = s.ok("gizmo.mesh_to_axis", json!({}));
    assert!(newest_step_bytes(&mut s) < 1024, "one transform step");
    assert_eq!(moved.result["pose"]["origin"], json!([0.0, 0.0, 0.0]));
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|p| std::array::from_fn(|k| p[k] - corner[k]))
        .collect();
    let gap = worst(&positions(&mut s, "cube"), &want);
    assert!(
        gap < 1e-6,
        "the cube keeps its offset from the gizmo: {gap}"
    );

    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    assert_eq!(positions(&mut s, "cube"), turned, "two undo steps, exactly");
    let pose = s.query("gizmo.describe", json!({}))["pose"].clone();
    let home = json!({ "origin": [0.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] });
    assert_eq!(
        [&pose["origin"], &pose["axes"]],
        [&home["origin"], &home["axes"]],
        "the pose is session state: undo leaves it"
    );
}

#[test]
fn a_pose_survives_a_swap_of_its_part_and_a_failed_call() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let place = json!({ "origin": [0, 1, 0], "normal": [0, 1, 0] });
    let placed = s.ok("gizmo.place", place).result["pose"].clone();
    let pose = |s: &mut Session| s.query("gizmo.describe", json!({}))["pose"].clone();
    s.ok(
        "transform.apply",
        json!({ "op": { "move": [0.2, 0.0, 0.0] } }),
    );
    s.ok("part.apply_transform", json!({}));
    assert_eq!(pose(&mut s), placed, "a whole-part swap keeps the pose");

    // The second step fails as it runs, after the first moved the pose and the part.
    let before = s.hash();
    let too_loose = json!({ "axis": "x", "tolerance": "50%" });
    let failed = s.batch(&[
        ("gizmo.mesh_to_axis", json!({})),
        ("symmetry.map", too_loose),
    ]);
    let error = failure(failed, "the batch");
    assert_eq!(code(error.code), "E_ARGS_INVALID", "{}", error.message);
    assert_eq!(s.hash(), before, "the part is where it was");
    assert_eq!(pose(&mut s), placed, "and so is its pose");
}

/// Review 0008, m3: a part scaled to 0 is a point, and framing it, in main or in a Reference
/// View, puts the camera some way off rather than on the point, which refused the move. (Only
/// at the origin: anywhere else a part of no size is too far from the origin for its size.)
#[test]
fn a_part_of_no_size_is_framed_from_some_way_off() {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let at = json!({ "origin": [0.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] });
    s.ok(
        "transform.apply",
        json!({ "frame": at, "op": { "scale": [0.0, 0.0, 0.0] } }),
    );
    s.ok("reference.open", json!({ "scene": true }));
    for view in ["main", "ref:1"] {
        let framed = s.ok("view.frame", json!({ "view": view }));
        let camera = &framed.result["camera"];
        let distance = camera["distance"].as_f64().unwrap_or(f64::NAN);
        assert!(distance > 0.01 && distance < 10.0, "{view}: {camera}");
        assert!(
            gap(&camera["target"], &json!([0, 0, 0])) < 1e-6,
            "{view}: {camera}"
        );
    }
}

/// Review 0008, m3: a part scaled far down is still found by `pick.nearest`. The point maps
/// into its space 1e24 times farther out, past where a squared distance fits an f32.
#[test]
fn a_part_scaled_far_down_is_still_the_nearest() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let frame = json!({ "origin": [0, 0, 0], "axes": [0, 0, 0, 1] });
    for _ in 0..2 {
        let tiny = json!({ "frame": frame, "op": { "scale": [1e-12, 1e-12, 1e-12] } });
        s.ok("transform.apply", tiny);
    }
    let near = s.ok("pick.nearest", json!({ "position": [0.0, 5.0, 0.0] }));
    let distance = near.result["found"]["distance"]
        .as_f64()
        .unwrap_or(f64::NAN);
    assert!((distance - 5.0).abs() < 1e-5, "{}", near.result);
}
