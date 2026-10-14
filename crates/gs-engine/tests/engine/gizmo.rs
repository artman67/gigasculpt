//! The gizmo and `transform.apply` through commands (decision 0027): a drag out and back leaves
//! every bit, a weight scales the transform and not its result, a mirrored transform keeps a
//! symmetric part symmetric with the points on the plane kept on it, falloff -100 is rigid, a
//! drag commits as one undo step whose log line replays it, a rigid turn at a low level turns
//! the top level's detail with it, `gizmo.describe` keeps the gizmo's size on screen and fades an
//! axis that points at the viewer, the same calls give the same bits on 1 and 24 threads, and
//! opening a scene forgets the old scene's gizmo.

use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_protocol::Role;
use gs_testkit::Session;
use gs_testkit::measure::{dot, norm, sub};
use serde_json::{Value, json};

use crate::support::{EngineSession, agent, human, log_records, tagged_triangle, worst};

/// The world frame at `origin`.
fn at(origin: [f32; 3]) -> Value {
    json!({ "origin": origin, "axes": [0.0, 0.0, 0.0, 1.0] })
}

fn positions(s: &mut Session, part: &str) -> Vec<[f32; 3]> {
    s.mesh(part).positions
}

/// `p` turned 120 degrees about the axis [1, 1, 1] through `o`: X goes to Y, Y to Z and Z to
/// X, so the reference needs no trigonometry.
pub(crate) fn cycled(p: [f32; 3], o: [f32; 3]) -> [f32; 3] {
    let v: [f64; 3] = std::array::from_fn(|i| f64::from(p[i]) - f64::from(o[i]));
    std::array::from_fn(|i| (f64::from(o[i]) + v[(i + 2) % 3]) as f32)
}

#[test]
fn a_drag_out_and_back_leaves_every_bit() {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let (before, points) = (s.hash(), positions(&mut s, "sphere"));
    let update = |op: Value| json!({ "phase": "update", "transform": { "op": op } });
    s.ok(
        "gizmo.drag",
        update(json!({ "rotate": { "axis": "y", "degrees": 40 } })),
    );
    assert_ne!(s.hash(), before, "the drag turns the sphere live");
    s.ok("gizmo.drag", update(json!({ "move": [0.3, 0.0, 0.0] })));
    s.ok("gizmo.drag", update(json!({ "move": [0.0, 0.0, 0.0] })));
    assert_eq!(s.hash(), before, "back where it began, to the bit");
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    assert_eq!(positions(&mut s, "sphere"), points, "and kept so");

    s.ok("gizmo.drag", update(json!({ "scale": [1.5, 1.0, 1.0] })));
    s.ok("gizmo.drag", json!({ "phase": "cancel" }));
    assert_eq!(s.hash(), before, "a cancel puts every bit back");
}

#[test]
fn half_weight_gives_half_the_distance_and_half_the_angle() {
    // Every point of the tagged triangle has a mask of 0.5, so a weight of 0.5.
    let mut s = agent();
    s.add_part("t", &tagged_triangle());
    let start = positions(&mut s, "t");
    let turn = json!({ "frame": at([0.0; 3]), "op": { "rotate": { "axis": "z", "degrees": 60 } } });
    s.ok("transform.apply", turn);
    // 30 degrees about Z: cos 30 = sqrt(3) / 2, sin 30 = 1 / 2.
    let (cos, sin) = (3f64.sqrt() / 2.0, 0.5);
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|p| {
            let (x, y) = (f64::from(p[0]), f64::from(p[1]));
            [(x * cos - y * sin) as f32, (x * sin + y * cos) as f32, p[2]]
        })
        .collect();
    let got = positions(&mut s, "t");
    assert!(
        worst(&got, &want) < 1e-6,
        "half the angle about the same axis: {got:?}"
    );

    s.ok("history.undo", json!({}));
    s.ok(
        "transform.apply",
        json!({ "op": { "move": [0.2, 0.4, 0.0] } }),
    );
    let want: Vec<[f32; 3]> = start
        .iter()
        .map(|p| [p[0] + 0.1, p[1] + 0.2, p[2]])
        .collect();
    let got = positions(&mut s, "t");
    assert!(worst(&got, &want) < 1e-6, "half the distance: {got:?}");
}

#[test]
fn a_mirrored_transform_keeps_a_symmetric_part_symmetric_and_the_plane_on_the_plane() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let start = positions(&mut s, "sphere");
    let mirror = |p: [f32; 3]| [-p[0], p[1], p[2]];
    let close = |p: [f32; 3], q: [f32; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 1e-6);
    let pairs: Vec<(usize, usize)> = (0..start.len())
        .filter_map(|i| {
            let j = (0..start.len()).find(|&j| close(start[i], mirror(start[j])))?;
            Some((i, j))
        })
        .collect();
    assert_eq!(
        pairs.len(),
        start.len(),
        "the sphere is mirror symmetric in x"
    );
    let args = json!({
        "frame": at([0.5, 0.2, 0.1]),
        "op": { "rotate": { "axis": "z", "degrees": 25 } },
        "falloff": 0,
        "reach": "150%",
        "symmetry": { "mirror": { "x": true } },
    });
    s.ok("transform.apply", args);
    let got = positions(&mut s, "sphere");
    for (i, j) in pairs {
        assert!(
            close(got[i], mirror(got[j])),
            "points {i} and {j} stay mirrored"
        );
    }
    let on_plane: Vec<usize> = (0..start.len()).filter(|&i| start[i][0] == 0.0).collect();
    assert!(!on_plane.is_empty(), "the sphere has points on the plane");
    for i in on_plane {
        assert_eq!(got[i][0], 0.0, "point {i} stays on the plane");
        assert_ne!(got[i], start[i], "point {i} still moves along the plane");
    }

    // With the frame on the plane the gizmo and its mirror coincide: one piece moves.
    s.ok("history.undo", json!({}));
    let across = json!({
        "frame": at([0.0; 3]),
        "op": { "move": [0.3, 0.0, 0.0] },
        "symmetry": { "mirror": { "x": true } },
    });
    s.ok("transform.apply", across);
    let want: Vec<[f32; 3]> = start.iter().map(|p| [p[0] + 0.3, p[1], p[2]]).collect();
    assert_eq!(
        positions(&mut s, "sphere"),
        want,
        "the whole sphere moves together"
    );
}

#[test]
fn falloff_minus_100_is_rigid() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let start = positions(&mut s, "sphere");
    let o = [0.2, 0.1, -0.3];
    let args = json!({
        "frame": at(o),
        "op": { "rotate": { "axis": [1.0, 1.0, 1.0], "degrees": 120 } },
        "falloff": -100,
        "reach": 0.1,
    });
    s.ok("transform.apply", args);
    let want: Vec<[f32; 3]> = start.iter().map(|&p| cycled(p, o)).collect();
    let got = positions(&mut s, "sphere");
    assert!(
        worst(&got, &want) < 2e-6,
        "every point turns fully, near the reach or not"
    );
}

#[test]
fn a_drag_commits_one_undo_step_that_undoes_exactly_and_replays_from_its_log_line() {
    let log = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gizmo-commit-{}.jsonl", std::process::id()));
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok(
        "gizmo.place",
        json!({ "origin": [0, 1, 0], "normal": [0, 1, 0] }),
    );
    let (before, undoable) = (s.hash(), s.history()["undoable"].clone());
    for degrees in [10.0, 21.0, 33.0] {
        let turn = json!({ "op": { "rotate": { "axis": "x", "degrees": degrees } } });
        s.ok(
            "gizmo.drag",
            json!({ "phase": "update", "transform": turn, "snap": {} }),
        );
    }
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    let after = s.hash();
    assert_ne!(after, before);
    let undoable = undoable.as_u64().map(|n| n + 1);
    assert_eq!(s.history()["undoable"].as_u64(), undoable, "one undo step");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo puts every bit back");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo brings the drag back");

    let (_, records) = log_records(&log);
    let line = (records.iter())
        .find(|r| r.command == "transform.apply")
        .unwrap_or_else(|| panic!("no transform.apply line: {records:?}"));
    assert_eq!(
        line.args["frame"]["origin"],
        json!([0.0, 1.0, 0.0]),
        "the gizmo's numbers"
    );
    assert_eq!(
        line.args["op"]["rotate"]["degrees"],
        json!(35.0),
        "snapped to 5 degrees"
    );
    assert!(
        records.iter().all(|r| r.command != "gizmo.drag"),
        "the drag itself is not logged"
    );
    let mut replay = agent();
    replay.replay_log(&log);
    assert_eq!(
        replay.hash(),
        after,
        "the log line replays the drag to the bit"
    );
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_rigid_turn_at_a_low_level_turns_the_top_detail_rigidly() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    let samples: Vec<Value> = (0..4)
        .map(|i| json!({ "position": [0.05 * f64::from(i), 0.2, 1.0] }))
        .collect();
    let detail = json!({ "brush": "Draw", "samples": samples, "size": 0.4 });
    let smooth = positions(&mut s, "sphere");
    s.ok("brush.stroke", detail);
    let top = positions(&mut s, "sphere");
    assert!(worst(&top, &smooth) > 0.01, "the top level has detail");
    s.ok("level.set", json!({ "level": 1 }));
    let turn = json!({ "frame": at([0.0; 3]), "op": { "rotate": { "axis": "z", "degrees": 90 } } });
    s.ok("transform.apply", turn);
    s.ok("level.set", json!({ "level": 3 }));
    let want: Vec<[f32; 3]> = top.iter().map(|p| [-p[1], p[0], p[2]]).collect();
    let got = positions(&mut s, "sphere");
    assert!(
        worst(&got, &want) < 1e-5,
        "the top level turned with its detail, off by {}",
        worst(&got, &want)
    );
}

/// `gizmo.describe` of the sphere's gizmo from a camera orbiting it.
fn describe(s: &mut Session, elevation: f64, distance: f32, at: Option<[f32; 2]>) -> Value {
    let camera = json!({
        "azimuth_deg": 0,
        "elevation_deg": elevation,
        "target": [0, 0, 0],
        "distance": distance,
    });
    let mut args = json!({ "camera": camera });
    if let Some(at) = at {
        args["at"] = json!(at);
    }
    s.query("gizmo.describe", args)
}

fn handle(d: &Value, name: &str) -> Value {
    let handles = d["handles"].as_array().cloned().unwrap_or_default();
    (handles.into_iter())
        .find(|h| h["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {d}"))
}

fn number(v: &Value) -> f32 {
    v.as_f64().unwrap_or(f64::NAN) as f32
}

#[test]
fn the_gizmo_keeps_its_size_on_screen_and_an_axis_at_the_viewer_cannot_be_grabbed() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let near = describe(&mut s, 0.0, 4.0, None);
    let far = describe(&mut s, 0.0, 8.0, None);
    assert_eq!(
        (near["unit_px"].clone(), far["unit_px"].clone()),
        (json!(75.0), json!(75.0))
    );
    let ratio = number(&far["unit"]) / number(&near["unit"]);
    assert!(
        (ratio - 2.0).abs() < 1e-4,
        "twice as far, twice the scene size: {ratio}"
    );
    let arrow = handle(&near, "move_x")["outline"].clone();
    let [a, b] = [&arrow[0], &arrow[1]].map(|p| [number(&p[0]), number(&p[1])]);
    let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
    let length = (dx * dx + dy * dy).sqrt();
    assert!(
        (length - 0.55 * 75.0).abs() < 0.01,
        "an arrow across the view: {length} px"
    );

    let z = handle(&near, "move_z");
    assert_eq!(
        (z["fade"].clone(), z["pickable"].clone()),
        (json!(0.0), json!(false))
    );
    let x = handle(&near, "move_x");
    assert_eq!(
        (x["fade"].clone(), x["pickable"].clone()),
        (json!(1.0), json!(true))
    );
    let centre = handle(&near, "move_view")["outline"].clone();
    let mid = |k: usize| (0..4).map(|i| number(&centre[i][k])).sum::<f32>() / 4.0;
    let pressed = describe(&mut s, 0.0, 4.0, Some([mid(0), mid(1)]));
    assert_eq!(
        pressed["at"],
        json!("move_view"),
        "the Z arrow lies under the press, ahead of the centre, but cannot be grabbed"
    );

    // Tilted 19.9484 degrees, whose cosine is 0.94, so 1 - |cos| = 0.06: halfway through the
    // fade from 0.02 to 0.1.
    let tilted = describe(&mut s, 19.948_44, 4.0, None);
    let fade = number(&handle(&tilted, "move_z")["fade"]);
    assert!((fade - 0.5).abs() < 0.01, "half faded: {fade}");
}

/// Transforms of every kind and a drag, returning the scene hash after each call.
fn script() -> Vec<String> {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let calls = [
        json!({
            "op": { "rotate": { "axis": "y", "degrees": 30 } },
            "falloff": 20,
            "reach": "60%",
            "distance": "surface",
            "symmetry": { "mirror": { "x": true } },
        }),
        json!({ "frame": at([0.0, 1.0, 0.0]), "op": { "inflate": 0.05 }, "falloff": 0, "reach": 0.8 }),
        json!({ "op": { "flatten": { "axis": "y", "amount": 0.3 } }, "falloff": -50, "reach": 1.5 }),
    ];
    let mut hashes = vec![s.hash()];
    for args in calls {
        s.ok("transform.apply", args);
        hashes.push(s.hash());
    }
    s.ok(
        "gizmo.place",
        json!({ "origin": [0.3, 0.3, 0.8], "normal": [0.3, 0.3, 0.9] }),
    );
    for f in [1.1, 1.3] {
        let grow = json!({ "op": { "scale": [f, 1.0, f] }, "falloff": 0, "reach": "50%" });
        s.ok(
            "gizmo.drag",
            json!({ "phase": "update", "transform": grow }),
        );
        hashes.push(s.hash());
    }
    s.ok("gizmo.drag", json!({ "phase": "end" }));
    hashes.push(s.hash());
    hashes
}

#[test]
fn transforms_and_drags_give_the_same_bits_on_1_and_24_threads() {
    let one = gs_par::with_threads(1, script);
    let many = gs_par::with_threads(24, script);
    assert_eq!(one, many, "the same bits on 1 and 24 threads");
    let changed = one.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(
        changed,
        one.len() - 2,
        "every call but the end changed the scene"
    );
}

#[test]
fn opening_a_scene_forgets_the_old_scenes_gizmo() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gizmo-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path = dir.join("cube.gss").to_string_lossy().into_owned();
    let mut saver = agent();
    saver.ok("primitive.cube", json!({}));
    saver.ok("file.save", json!({ "path": path }));
    let mut fresh = agent();
    fresh.ok("file.open", json!({ "path": path }));
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok(
        "gizmo.place",
        json!({ "origin": [5.0, 5.0, 5.0], "normal": [1, 0, 0] }),
    );
    s.ok(
        "file.open",
        json!({ "path": path, "discard_changes": true }),
    );
    let (got, want) = (
        s.query("gizmo.describe", json!({})),
        fresh.query("gizmo.describe", json!({})),
    );
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        got, want,
        "the opened cube's gizmo is a fresh session's, not the sphere's"
    );
}

/// Masks the back half of the sphere fully: hides the faces toward the front camera, masks
/// what shows and shows everything again.
fn mask_the_back(s: &mut Session) {
    let front = json!({ "select": { "by": "facing" }, "camera": "front" });
    s.ok("visibility.hide", front);
    s.ok("mask.set", json!({ "value": 1 }));
    s.ok("visibility.show_all", json!({}));
}

/// Masks one more ring of the sphere fully, as extending the mask with a brush would: hides the
/// unmasked faces, shows a ring more, masks what shows and shows everything again. Four undo
/// steps. (A grow only spreads a soft edge, which leaves every point that was below 1 below 1.)
fn extend_the_mask(s: &mut Session) {
    s.ok("visibility.hide", json!({ "select": { "by": "unmasked" } }));
    s.ok("visibility.grow", json!({}));
    s.ok("mask.set", json!({ "value": 1 }));
    s.ok("visibility.show_all", json!({}));
}

/// The centre of the bounding box of the sphere's points whose mask is below 1, every point
/// showing, worked out from `part.get`.
fn unmasked_centre(s: &mut Session) -> [f32; 3] {
    let mesh = s.query("part.get", json!({ "part": "sphere" }));
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let mask = list(&mesh["attributes"]["mask"]["values"]["f32"]);
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for (i, p) in list(&mesh["positions"]).iter().enumerate() {
        if mask.get(i).map_or(0.0, number) >= 1.0 {
            continue;
        }
        for a in 0..3 {
            lo[a] = lo[a].min(number(&p[a]));
            hi[a] = hi[a].max(number(&p[a]));
        }
    }
    std::array::from_fn(|a| f32::midpoint(lo[a], hi[a]))
}

/// The gizmo's origin, as `gizmo.place` or `gizmo.describe` answer.
fn origin(answer: &Value) -> [f32; 3] {
    std::array::from_fn(|a| number(&answer["pose"]["origin"][a]))
}

/// `gizmo.unmasked-center`: with the back half of a sphere fully masked, the rule puts the
/// gizmo at the centre of the unmasked front half's bounding box and keeps its axes; with
/// every point masked it is refused, naming the fix.
#[test]
fn unmasked_center_puts_the_gizmo_in_the_middle_of_the_unmasked_half() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    mask_the_back(&mut s);
    let turned = json!([0.0, 0.0, 0.258_819, 0.965_926]);
    let before = s.query("gizmo.place", json!({ "axes": turned }));
    let placed = s.query("gizmo.place", json!({ "rule": "unmasked_center" }));
    let want = unmasked_centre(&mut s);
    assert!(want[2] > 0.4, "the front half's middle: {want:?}");
    assert_eq!(origin(&placed), want, "the unmasked half's middle");
    assert_eq!(
        placed["pose"]["axes"], before["pose"]["axes"],
        "the axes kept"
    );

    s.ok("mask.set", json!({ "value": 1 }));
    let refused = s.err("gizmo.place", json!({ "rule": "unmasked_center" }));
    assert!(
        refused.message.contains("masked") && refused.hint.is_some(),
        "{refused:?}"
    );
}

/// `gizmo.unmasked-center`'s follow (proposed, off by default): a gizmo that follows its mask
/// goes to the centre of the unmasked points when follow is turned on, and again, with no
/// gizmo command, after every change that moves that centre, an undo included. One that does
/// not follow stays, and a placement by hand holds until the centre moves.
#[test]
fn a_gizmo_that_follows_the_mask_goes_to_the_new_unmasked_centre_by_itself() {
    let mut s = agent();
    // Fine enough that two rings more still leave points unmasked.
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    mask_the_back(&mut s);
    let half = s.query("gizmo.place", json!({ "rule": "unmasked_center" }));
    assert_eq!(half["pose"]["follow"], json!(false), "off by default");
    let at = |s: &mut Session| origin(&s.query("gizmo.describe", json!({ "part": "sphere" })));
    extend_the_mask(&mut s);
    assert_eq!(at(&mut s), origin(&half), "not following, it stays");

    let on = s.query("gizmo.place", json!({ "follow": true }));
    let extended = unmasked_centre(&mut s);
    assert_ne!(extended, origin(&half), "the mask's ring moved the centre");
    assert_eq!(
        (origin(&on), on["pose"]["follow"].clone()),
        (extended, json!(true)),
        "turned on, it goes to the centre"
    );
    extend_the_mask(&mut s);
    let again = unmasked_centre(&mut s);
    assert_ne!(again, extended, "a ring more moved the centre again");
    assert_eq!(at(&mut s), again, "it followed with no gizmo command");
    for _ in 0..4 {
        s.ok("history.undo", json!({}));
    }
    assert_eq!(at(&mut s), extended, "and back with the undos");

    let by_hand = [0.5, 0.25, 0.0];
    s.ok("gizmo.place", json!({ "origin": by_hand }));
    s.ok("primitive.cube", json!({}));
    assert_eq!(
        at(&mut s),
        by_hand,
        "placed by hand, it holds through an edit that leaves the centre"
    );
    let shrink = json!({ "part": "sphere", "op": "shrink" });
    s.ok("mask.filter", shrink);
    let shrunk = unmasked_centre(&mut s);
    assert_ne!(shrunk, extended, "a shrink frees the mask's edge");
    assert_eq!(at(&mut s), shrunk, "until the centre moves");
}

/// `gizmo.gizmo` with a hidden half: a drag turns about where the gizmo was when it began. A
/// gizmo not placed sits at the middle of the points that show, and a turn of those points
/// moves that middle; the pivot must not move with it, so a turn reaching 40 degrees by way of
/// 20 gives the same bits as one straight to 40.
#[test]
fn a_drag_turns_about_where_the_gizmo_was_when_it_began() {
    let turned = |degrees: &[f32]| {
        let mut s = human();
        s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        let front = json!({ "select": { "by": "facing" }, "camera": "front" });
        s.ok("visibility.hide", front);
        for d in degrees {
            let op = json!({ "rotate": { "axis": "x", "degrees": d } });
            s.ok(
                "gizmo.drag",
                json!({ "phase": "update", "transform": { "op": op } }),
            );
        }
        s.ok("gizmo.drag", json!({ "phase": "end" }));
        s.hash()
    };
    assert_eq!(
        turned(&[20.0, 40.0]),
        turned(&[40.0]),
        "the pivot stays where the drag began"
    );
}

/// `gizmo.reset-orientation` and `gizmo.mesh-to-axis` with Alt held: the gizmo-only forms the
/// window's buttons send. With a part turned 30 degrees and its gizmo turned with it,
/// `gizmo.reset` squares only the gizmo to the scene axes, keeping its centre, and
/// `gizmo.place` rule `world_origin` moves only the gizmo to the origin, keeping its axes;
/// every vertex stays to the bit.
#[test]
fn with_alt_mesh_to_axis_and_reset_orientation_move_only_the_gizmo() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let turned = json!([0.0, 0.0, 0.258_819, 0.965_926]);
    s.ok(
        "gizmo.place",
        json!({ "origin": [0.3, 0.2, 0.1], "axes": turned }),
    );
    let turn = json!({ "op": { "rotate": { "axis": "z", "degrees": 30 } } });
    s.ok("transform.apply", turn);
    let (hash, points) = (s.hash(), positions(&mut s, "sphere"));
    let before = s.query("gizmo.describe", json!({}));
    let square = json!([0.0, 0.0, 0.0, 1.0]);
    assert_ne!(before["pose"]["axes"], square, "turned with the part");

    let reset = s.query("gizmo.reset", json!({}));
    assert_eq!(reset["pose"]["axes"], square, "square to the scene axes");
    assert_eq!(origin(&reset), origin(&before), "its centre kept");
    let placed = s.query("gizmo.place", json!({ "rule": "world_origin" }));
    assert_eq!(origin(&placed), [0.0; 3], "on the scene origin");
    assert_eq!(placed["pose"]["axes"], square, "its axes kept");
    assert_eq!(
        (s.hash(), positions(&mut s, "sphere")),
        (hash, points),
        "every vertex stays to the bit"
    );
}

/// `gizmo.gizmo-scale`'s figures, through the `transform.apply` ops its gestures send: an
/// even scale by k takes each point p to o + k(p - o); a scale along Z changes only Z; an
/// inflate of a tenth of a sphere's radius moves every point a tenth along its normal; a full
/// flatten across Z puts every point of a cube on the plane through the origin. The window's
/// Ctrl inflate, Ctrl flatten and Alt plane scale wait for the maintainer's call.
#[test]
fn scale_inflate_and_flatten_give_the_checks_figures() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let start = positions(&mut s, "sphere");
    let (o, k) = ([0.2f32, -0.1, 0.3], 1.5f32);
    let even = json!({ "frame": at(o), "op": { "scale": [k, k, k] } });
    s.ok("transform.apply", even);
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|p| std::array::from_fn(|i| o[i] + k * (p[i] - o[i])))
        .collect();
    let got = positions(&mut s, "sphere");
    assert!(worst(&got, &want) < 1e-5, "o + k(p - o)");

    s.ok("history.undo", json!({}));
    let along_z = json!({ "frame": at(o), "op": { "scale": [1.0, 1.0, k] } });
    s.ok("transform.apply", along_z);
    for (p, q) in start.iter().zip(positions(&mut s, "sphere")) {
        assert_eq!([q[0], q[1]], [p[0], p[1]], "Z only");
        assert!(
            (q[2] - (o[2] + k * (p[2] - o[2]))).abs() < 1e-5,
            "{p:?} to {q:?}"
        );
    }

    s.ok("history.undo", json!({}));
    let ids: Vec<u32> = (0..start.len() as u32).collect();
    let read = s.query("mesh.sample", json!({ "ids": ids, "attributes": [] }));
    let normals = read["vertices"].as_array().cloned().unwrap_or_default();
    assert_eq!(normals.len(), start.len());
    s.ok("transform.apply", json!({ "op": { "inflate": 0.1 } }));
    let got = positions(&mut s, "sphere");
    for ((p, q), v) in start.iter().zip(&got).zip(&normals) {
        let n = std::array::from_fn(|i| v["normal"][i].as_f64().unwrap_or(0.0));
        let by = sub(q.map(f64::from), p.map(f64::from));
        assert!(
            (norm(by) - 0.1).abs() < 1e-5,
            "a tenth of the radius: {by:?}"
        );
        let along = dot(by, n) / norm(by);
        assert!(along > 1.0 - 1e-4, "along the normal: {by:?} {n:?}");
    }

    s.ok("primitive.cube", json!({ "subdivisions": 4 }));
    let cube = positions(&mut s, "cube");
    let flat = json!({
        "part": "cube",
        "frame": at(o),
        "op": { "flatten": { "axis": "z", "amount": 1.0 } },
    });
    s.ok("transform.apply", flat);
    for (p, q) in cube.iter().zip(positions(&mut s, "cube")) {
        assert!(
            (q[2] - o[2]).abs() < 1e-6,
            "on the plane z = {}: {q:?}",
            o[2]
        );
        assert_eq!([q[0], q[1]], [p[0], p[1]], "straight across it");
    }
}

/// `gizmo.soft-falloff`'s Check: with falloff 0 a move takes the point at the frame's origin
/// the whole way and leaves every point beyond the reach to the bit, and a point at half weight
/// turned 90 degrees turns 45 about the same axis, so it keeps its distance to the axis.
/// Falloff -100 moving every point rigidly is `falloff_minus_100_is_rigid`.
#[test]
fn soft_falloff_moves_the_origin_fully_and_nothing_beyond_the_reach() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let start = positions(&mut s, "sphere");
    let (o, reach) = (start[0], 0.8f32);
    let soft = json!({
        "frame": at(o),
        "op": { "move": [0.1, 0.0, 0.0] },
        "falloff": 0,
        "reach": reach,
    });
    s.ok("transform.apply", soft);
    let got = positions(&mut s, "sphere");
    assert_eq!(
        got[0],
        [o[0] + 0.1, o[1], o[2]],
        "the point at the origin, fully"
    );
    let (mut beyond, mut within) = (0, 0);
    for (p, q) in start.iter().zip(&got) {
        let d = norm(sub(p.map(f64::from), o.map(f64::from)));
        if d > f64::from(reach) * 1.001 {
            assert_eq!(q, p, "beyond the reach, to the bit");
            beyond += 1;
        } else if d < f64::from(reach) * 0.999 && q.map(f32::to_bits) != p.map(f32::to_bits) {
            within += 1;
        }
    }
    assert!(beyond > 0 && within > 1, "{beyond} beyond, {within} within");

    // Every point of the tagged triangle has a mask of 0.5, so a weight of 0.5.
    s.add_part("t", &tagged_triangle());
    let tri = positions(&mut s, "t");
    let quarter = json!({
        "part": "t",
        "frame": at([0.0; 3]),
        "op": { "rotate": { "axis": "z", "degrees": 90 } },
    });
    s.ok("transform.apply", quarter);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let want: Vec<[f32; 3]> = (tri.iter())
        .map(|p| {
            let (x, y) = (f64::from(p[0]), f64::from(p[1]));
            [((x - y) * half) as f32, ((x + y) * half) as f32, p[2]]
        })
        .collect();
    let got = positions(&mut s, "t");
    assert!(worst(&got, &want) < 1e-6, "45 degrees about Z: {got:?}");
    for (p, q) in tri.iter().zip(&got) {
        let off_axis = |v: &[f32; 3]| norm([f64::from(v[0]), f64::from(v[1]), 0.0]);
        let gap = off_axis(p) - off_axis(q);
        assert!(
            gap.abs() < 1e-6,
            "its distance to the axis kept: {p:?} {q:?}"
        );
    }
}
