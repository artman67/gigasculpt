//! The Reference View (decision 0020), through commands: with rotation linked it turns exactly
//! as main turns, in every orbit mode and through any alignment; unlinked zoom and slide stay
//! its own, and unlinking freezes it; lining up the fronts makes a chosen reference turn match
//! main and keeps matching it; it never touches the document, its undo or its hash; agents can
//! set it up and read it; and its cameras replay bit for bit from the log. Metamorphic
//! relations and registry properties (ARCHITECTURE 10.1).

use std::path::PathBuf;

use gs_core::Quat;
use gs_engine::EngineConfig;
use gs_protocol::{ErrorCode, Role};
use gs_testkit::rng::signed_unit;
use gs_testkit::{Session, SplitMix64};
use serde_json::{Value, json};

use crate::support::{EngineSession, camera, document, exact, human, log_records, windowed};

/// How far apart two turns put the axes, at most.
fn apart(a: Quat, b: Quat) -> f32 {
    let (a, b) = (a.axes(), b.axes());
    let pairs = [(a.0, b.0), (a.1, b.1), (a.2, b.2)];
    (pairs.iter())
        .flat_map(|(u, v)| (*u - *v).to_array())
        .fold(0.0, |m, d| m.max(d.abs()))
}

/// A sphere at the origin and a cube to its right, the cube shown frozen as `ref:1`.
fn scene_with_reference(s: &mut Session, open: Value) {
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    s.ok("reference.open", open);
}

/// A random move of main, as the human at the window would make it: every orbit mode, snaps,
/// rolls, standard views and exact cameras.
fn move_main(s: &mut Session, rng: &mut SplitMix64) {
    let pick = rng.below(7);
    let mode = ["free", "x", "y", "z"][rng.below(4)];
    let mut u = || signed_unit(rng.next_u64());
    let (command, args) = match pick {
        0..=2 => (
            "view.orbit",
            json!({ "right_deg": 120.0 * u(), "up_deg": 90.0 * u(), "mode": mode }),
        ),
        3 => (
            "view.orbit",
            json!({ "right_deg": 50.0 * u(), "mode": mode, "snap": true }),
        ),
        4 => ("view.roll", json!({ "deg": 90.0 * u() })),
        5 => ("view.align", json!({ "to": "left", "toggle": true })),
        _ => {
            let q = Quat::from_array([u(), u(), u(), u()]).normalized();
            let camera = json!({ "orientation": q.to_array(), "target": [0, 0, 0], "distance": 4 });
            ("view.set", json!({ "camera": camera }))
        }
    };
    s.ok(command, args);
}

#[test]
fn with_rotation_linked_the_reference_turns_exactly_as_main_turns() {
    let mut s = human();
    scene_with_reference(&mut s, json!({ "parts": "cube" }));
    let mut rng = SplitMix64::new(41);
    // Not aligned: the very same turn, number for number.
    for _ in 0..60 {
        move_main(&mut s, &mut rng);
        let (main, reference) = (camera(&mut s, "main"), camera(&mut s, "ref:1"));
        assert_eq!(reference.orientation, main.orientation, "the same turn");
    }
    // Aligned by a turn: main's turns reach it through the alignment, each by the same amount.
    let turn = Quat::from_array([0.3, -0.5, 0.2, 0.787_4]).normalized();
    let to = json!({ "turn": turn.to_array(), "scale": 0.5, "offset": [1, 2, 3] });
    s.ok("reference.align", json!({ "to": to }));
    let mut before = (camera(&mut s, "main"), camera(&mut s, "ref:1"));
    for _ in 0..60 {
        move_main(&mut s, &mut rng);
        let (main, reference) = (camera(&mut s, "main"), camera(&mut s, "ref:1"));
        let through = turn.inverse() * main.orientation;
        assert!(apart(reference.orientation, through) < 2e-6, "aligned turn");
        // The turn from the last step: main's, carried through the alignment.
        let main_step = main.orientation * before.0.orientation.inverse();
        let ref_step = reference.orientation * before.1.orientation.inverse();
        let carried = turn.inverse() * main_step * turn;
        assert!(
            apart(ref_step, carried) < 4e-6,
            "the same rotation each step"
        );
        before = (main, reference);
    }
}

/// How far toward the edge of a `size` picture through `c` the farthest of `points` lands:
/// 1 on the edge, 0.9 the border framing keeps.
fn edge(c: &gs_core::camera::Camera, points: &[[f32; 3]], size: [f32; 2]) -> f32 {
    let reach = |p: &[f32; 3]| match c.project(*p, size) {
        Some([x, y]) => (2.0 * x / size[0] - 1.0)
            .abs()
            .max((1.0 - 2.0 * y / size[1]).abs()),
        None => f32::INFINITY,
    };
    points.iter().map(reach).fold(0.0, f32::max)
}

#[test]
fn a_reference_is_framed_by_the_sphere_around_it_so_linked_turns_keep_it_in_the_picture() {
    // A round scene still fills its picture: the sphere reaches the farthest point, not the
    // corners of the boxes around the points.
    let mut round = human();
    round.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let square = json!({ "scene": true, "viewport": [500, 500] });
    round.ok("reference.open", square);
    let c = camera(&mut round, "ref:1");
    let filled = edge(&c, &round.mesh("sphere").positions, [500.0, 500.0]);
    assert!(filled > 0.85, "a sphere fills {filled} of its picture");
    let mut s = human();
    // Long along the axis it opens looking down: a box fitted from the front is too close once
    // main turns it side on.
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("primitive.cube", json!({ "center": [0, 0, -4] }));
    let size = [800.0, 500.0];
    s.ok(
        "reference.open",
        json!({ "scene": true, "viewport": [800, 500] }),
    );
    let opened = camera(&mut s, "ref:1");
    let mut points = s.mesh("sphere").positions;
    points.extend(s.mesh("cube").positions);
    let mut rng = SplitMix64::new(7);
    for step in 0..40 {
        move_main(&mut s, &mut rng);
        let most = edge(&camera(&mut s, "ref:1"), &points, size);
        assert!(most <= 0.9 + 1e-3, "step {step}: a point at {most}");
        // Framing it again at any turn frames it the same: target and distance.
        s.ok("view.frame", json!({ "view": "ref:1" }));
        let framed = camera(&mut s, "ref:1");
        let at = |c: &gs_core::camera::Camera| (c.target, c.distance);
        assert_eq!(at(&framed), at(&opened), "step {step}");
    }
}

#[test]
fn unlinked_zoom_and_pan_stay_the_references_own_and_unlinking_freezes_it() {
    let mut s = human();
    scene_with_reference(&mut s, json!({ "parts": "cube" }));
    let start = camera(&mut s, "ref:1");
    for _ in 0..4 {
        s.ok("view.dolly", json!({ "factor": 0.5 }));
    }
    s.ok("view.pan", json!({ "dx": 140, "dy": -60 }));
    s.ok("view.orbit", json!({ "right_deg": 35, "up_deg": 10 }));
    let after = camera(&mut s, "ref:1");
    let main = camera(&mut s, "main");
    assert_eq!(
        (after.distance.to_bits(), after.target.map(f32::to_bits)),
        (start.distance.to_bits(), start.target.map(f32::to_bits)),
        "main came 16 times closer and slid; the reference's framing did not change"
    );
    assert_eq!(
        after.orientation, main.orientation,
        "while its turn followed"
    );
    // Its own zoom and slide move it, not main.
    let main = exact(&mut s, "main");
    s.ok("view.dolly", json!({ "view": "ref:1", "factor": 2 }));
    s.ok("view.pan", json!({ "view": "ref:1", "dx": 30 }));
    assert_eq!(exact(&mut s, "main"), main, "main stayed");
    let moved = camera(&mut s, "ref:1").distance.to_bits();
    assert_ne!(moved, after.distance.to_bits(), "the reference moved");
    // Its linked turn is main's: turning it alone is refused, with the way out.
    let e = s.err("view.orbit", json!({ "view": "ref:1", "right_deg": 20 }));
    assert_eq!(json!(e.code), json!(ErrorCode::Precondition));
    assert!(
        e.hint.is_some_and(|h| h.contains("rotation=false")),
        "says how"
    );
    // Unlinked, it keeps the picture it had and stays put while main turns.
    let linked = exact(&mut s, "ref:1");
    s.ok("reference.link", json!({ "rotation": false }));
    assert_eq!(exact(&mut s, "ref:1"), linked, "unlinking does not jump");
    for deg in [30, -70, 115] {
        s.ok(
            "view.orbit",
            json!({ "right_deg": deg, "up_deg": 20, "mode": "y" }),
        );
        assert_eq!(exact(&mut s, "ref:1"), linked, "frozen bit for bit");
    }
    // Linked again, it takes main's turn at once; zoom linked, main's distance through the scale.
    s.ok("reference.link", json!({ "rotation": true, "zoom": true }));
    let (main, reference) = (camera(&mut s, "main"), camera(&mut s, "ref:1"));
    assert_eq!(reference.orientation, main.orientation, "relinked");
    assert_eq!(
        reference.distance.to_bits(),
        main.distance.to_bits(),
        "scale 1"
    );
    let e = s.err("view.dolly", json!({ "view": "ref:1", "factor": 0.5 }));
    assert!(e.message.contains("zoom is linked"), "{}", e.message);
}

#[test]
fn lining_up_the_fronts_matches_a_chosen_turn_and_keeps_matching_it() {
    let mut s = human();
    scene_with_reference(
        &mut s,
        json!({ "parts": "cube", "link": { "rotation": false } }),
    );
    // The reference turned to some side, main to another.
    let chosen = Quat::from_array([-0.2, 0.6, 0.1, 0.77]).normalized();
    let set = json!({ "view": "ref:1", "camera": {
        "orientation": chosen.to_array(), "target": [3, 0, 0], "distance": 5 } });
    s.ok("view.set", set);
    s.ok("view.orbit", json!({ "right_deg": -40, "up_deg": 25 }));
    let main = camera(&mut s, "main").orientation;
    let shown = s.ok("reference.align", json!({})).result;
    assert_eq!(
        shown["reference"]["link"]["rotation"],
        json!(true),
        "aligned, it turns with main"
    );
    let aligned = camera(&mut s, "ref:1").orientation;
    assert!(
        apart(aligned, chosen) < 2e-6,
        "it still shows the chosen side"
    );
    // From now on the reference relative to main is fixed: every turn of main turns it alike.
    let offset = chosen * main.inverse();
    let mut rng = SplitMix64::new(42);
    for _ in 0..40 {
        move_main(&mut s, &mut rng);
        let (main, reference) = (camera(&mut s, "main"), camera(&mut s, "ref:1"));
        let kept = reference.orientation * main.orientation.inverse();
        assert!(apart(kept, offset) < 4e-6, "the fronts stay lined up");
    }
    // Its box was put on the scene's, so linking zoom and pan shows both at one size and place.
    let align = s.query("reference.list", json!({}))[0]["reference"]["align"].clone();
    let scale = align["scale"].as_f64().unwrap_or_default();
    assert!(scale > 1.0, "the cube is smaller than the scene: {align}");
    s.ok("reference.align", json!({ "reset": true }));
    let list = s.query("reference.list", json!({}));
    assert_eq!(
        list[0]["reference"]["align"],
        json!({ "turn": [0.0, 0.0, 0.0, 1.0], "scale": 1.0, "offset": [0.0, 0.0, 0.0] })
    );
}

/// A small OBJ file: a unit box. Its path.
fn box_file(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("reference-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("box.obj");
    let corner = |k: usize| {
        let c = |bit: usize| if k & (1 << bit) == 0 { -0.5 } else { 0.5 };
        format!("v {} {} {}\n", c(0), c(1), c(2))
    };
    let faces = [
        "1 3 4 2", "5 6 8 7", "1 2 6 5", "3 7 8 4", "1 5 7 3", "2 4 8 6",
    ];
    let text: String = (0..8)
        .map(corner)
        .chain(faces.iter().map(|f| format!("f {f}\n")))
        .collect();
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
    path
}

#[test]
fn the_reference_never_enters_the_document_its_undo_or_its_hash() {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    let before = document(&mut s);
    let file = box_file("undo");
    let opened = s.ok(
        "reference.open",
        json!({ "file": file.display().to_string() }),
    );
    let source = &opened.result["reference"]["source"]["file"];
    assert_eq!(source["hash"].as_str().map(str::len), Some(64), "{source}");
    assert_eq!(opened.result["reference"]["parts"], json!(["box"]));
    s.ok(
        "reference.open",
        json!({ "view": "ref:2", "parts": ["cube", "sphere"] }),
    );
    s.ok("reference.open", json!({ "view": "ref:3", "scene": true }));
    let link = json!({ "view": "ref:1", "rotation": false, "zoom": true });
    s.ok("reference.link", link);
    s.ok("view.orbit", json!({ "view": "ref:1", "right_deg": 90 }));
    s.ok("reference.align", json!({ "view": "ref:1" }));
    s.ok(
        "view.look",
        json!({ "view": "ref:2", "matcap": "red-wax", "wireframe": true }),
    );
    s.ok("view.frame", json!({ "view": "ref:2" }));
    s.ok("view.undo", json!({ "view": "ref:2" }));
    s.ok("view.orbit", json!({ "right_deg": 15 }));
    s.ok("reference.close", json!({ "view": "ref:3" }));
    assert_eq!(
        document(&mut s),
        before,
        "same hash, revision and undo steps"
    );
    let list = s.query("reference.list", json!({}));
    assert_eq!(list.as_array().map(Vec::len), Some(2), "{list}");
    // The document's undo takes back the cube; the frozen copy stays in ref:2.
    s.ok("history.undo", json!({}));
    let parts = &s.query("view.get", json!({ "view": "ref:2" }))["reference"]["parts"];
    assert_eq!(parts, &json!(["cube", "sphere"]), "frozen");
    s.ok("reference.close", json!({}));
    s.ok("reference.close", json!({}));
    let e = s.err("reference.close", json!({}));
    assert_eq!(json!(e.code), json!(ErrorCode::Precondition), "none open");
    std::fs::remove_dir_all(file.parent().unwrap_or(&file)).ok();
}

#[test]
fn agents_set_up_and_read_the_reference_view() {
    let mut s = windowed();
    let state = s.query("session.state", json!({}));
    assert_eq!(state["reference"], Value::Null, "none open");
    let e = s.err("reference.align", json!({}));
    let says_how = e.hint.is_some_and(|h| h.contains("reference.open"));
    assert!(
        says_how && e.message.contains("no Reference View"),
        "{}",
        e.message
    );
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    s.ok(
        "reference.open",
        json!({ "parts": "cube", "look": { "matcap": "red-wax" } }),
    );
    s.ok("reference.link", json!({ "flip": "pan" }));
    s.ok("view.look", json!({ "view": "ref:1", "wireframe": true }));
    s.ok("reference.align", json!({}));
    let state = s.query("session.state", json!({}));
    let values = [
        "reference",
        "reference.link.rotation",
        "reference.link.zoom",
    ];
    let more = [
        "reference.link.pan",
        "reference.look",
        "reference.wireframe",
    ];
    let got: Vec<&Value> = values.iter().chain(&more).map(|k| &state[*k]).collect();
    let want = [
        json!("ref:1"),
        json!(true),
        json!(false),
        json!(true),
        json!("red-wax"),
        json!(true),
    ];
    assert_eq!(got, want.iter().collect::<Vec<_>>(), "{state}");
    let view = s.query("view.get", json!({ "view": "ref:1" }));
    assert_eq!(view["reference"]["parts"], json!(["cube"]), "{view}");
    assert_eq!(
        view["reference"]["source"],
        json!({ "parts": { "parts": ["cube"] } })
    );
    // The window's reference is the human's to move, as main is.
    let e = s.err("view.dolly", json!({ "view": "ref:1", "factor": 0.5 }));
    assert_eq!(json!(e.code), json!(ErrorCode::Precondition));
    assert!(
        e.hint.is_some_and(|h| h.contains("agents_move_view")),
        "says how"
    );
    s.as_human("session.control", json!({ "agents_move_view": true }));
    s.ok("view.dolly", json!({ "view": "ref:1", "factor": 0.5 }));
    // Looking through it shows its own scene: a contact sheet cannot mix it with another.
    let e = s.err("view.contact_sheet", json!({ "views": ["ref:1", "front"] }));
    assert_eq!(
        json!(e.code),
        json!(ErrorCode::ArgsInvalid),
        "{}",
        e.message
    );
    let e = s.err("reference.open", json!({ "scene": true, "parts": "cube" }));
    assert_eq!(json!(e.code), json!(ErrorCode::ArgsInvalid), "one source");
    let e = s.err("view.get", json!({ "view": "ref:7" }));
    assert!(e.message.contains("not open"), "{}", e.message);
}

/// Decision 0024: the reference opened last is active, `reference.next` moves on by number and
/// wraps, a command without `view` acts on the active one, and closing it falls back to the
/// lowest-numbered open one.
#[test]
fn the_active_reference_is_the_one_opened_last_or_moved_to_and_view_less_commands_act_on_it() {
    let mut s = windowed();
    let active = |s: &mut Session| s.query("session.state", json!({}))["reference"].clone();
    let pan = |s: &mut Session, view: &str| {
        s.query("view.get", json!({ "view": view }))["reference"]["link"]["pan"].clone()
    };
    scene_with_reference(&mut s, json!({ "scene": true }));
    s.ok("reference.open", json!({ "parts": "cube" }));
    s.ok("reference.open", json!({ "parts": "sphere" }));
    assert_eq!(active(&mut s), json!("ref:3"), "the one opened last");
    s.ok("reference.link", json!({ "flip": "pan" }));
    let pans = ["ref:1", "ref:2", "ref:3"].map(|v| pan(&mut s, v));
    assert_eq!(
        pans,
        [false, false, true].map(Value::from),
        "on the active one"
    );
    let moved = ["ref:1", "ref:2", "ref:3", "ref:1"].map(|_| s.ok("reference.next", json!({})));
    let got = moved.map(|m| m.result["view"].clone());
    assert_eq!(
        got,
        ["ref:1", "ref:2", "ref:3", "ref:1"].map(Value::from),
        "wraps"
    );
    let from = s.ok("reference.next", json!({ "view": "ref:2" }));
    let to = (from.result["view"].clone(), active(&mut s));
    assert_eq!(to, (json!("ref:3"), json!("ref:3")));
    s.ok("reference.close", json!({}));
    assert_eq!(active(&mut s), json!("ref:1"), "the lowest open one");
    s.ok("reference.close", json!({ "view": "ref:1" }));
    assert_eq!(active(&mut s), json!("ref:2"));
    s.ok("reference.close", json!({}));
    assert_eq!(active(&mut s), Value::Null);
    let e = s.err("reference.next", json!({}));
    assert_eq!(json!(e.code), json!(ErrorCode::Precondition), "none open");
}

#[test]
fn a_snapshot_through_the_reference_shows_its_own_scene_in_its_own_look() {
    let renderer = gs_ui::Offscreen::host(None).unwrap_or_else(|e| panic!("{e}"));
    let mut s = human();
    s.engine.attach_renderer(renderer);
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok(
        "primitive.cube",
        json!({ "center": [3, 0, 0], "name": "cube" }),
    );
    s.ok(
        "reference.open",
        json!({ "parts": "cube", "look": { "matcap": "red-wax" } }),
    );
    let look = |s: &mut Session, view: Value| {
        let shot = s.query(
            "view.snapshot",
            json!({ "view": view, "size": 64, "antialias": false }),
        );
        shot["png_base64"].as_str().unwrap_or_default().to_string()
    };
    let through = look(&mut s, json!("ref:1"));
    let same_camera = serde_json::from_str::<Value>(&exact(&mut s, "ref:1")).unwrap_or_default();
    let scene = look(&mut s, same_camera.clone());
    assert_ne!(through, scene, "the reference's own look, not main's");
    // Take the cube out of the scene: the frozen reference still shows it.
    s.ok("history.undo", json!({}));
    assert_eq!(look(&mut s, json!("ref:1")), through, "its own scene");
    assert_ne!(look(&mut s, same_camera), scene, "the scene lost the cube");
}

#[test]
fn reference_cameras_replay_bit_for_bit_from_the_log() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("reference-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let log = dir.join("log.jsonl");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    scene_with_reference(&mut s, json!({ "parts": "cube" }));
    s.ok("reference.open", json!({ "view": "ref:2", "scene": true, "link": { "rotation": true, "zoom": true, "pan": true } }));
    s.ok(
        "reference.link",
        json!({ "view": "ref:1", "rotation": false }),
    );
    s.ok(
        "view.orbit",
        json!({ "view": "ref:1", "right_deg": 77, "up_deg": -12, "mode": "y" }),
    );
    s.ok("reference.align", json!({ "view": "ref:1" }));
    for k in 0..30 {
        let args = json!({ "right_deg": 1.3, "up_deg": 0.4, "gesture": 5, "end": k == 29 });
        s.ok("view.orbit", args);
    }
    for k in 0..10 {
        let args = json!({ "view": "ref:1", "factor": 0.97, "gesture": 6, "end": k == 9 });
        s.ok("view.dolly", args);
    }
    s.ok("view.pan", json!({ "dx": 12, "dy": 30 }));
    s.ok(
        "reference.align",
        json!({ "view": "ref:2", "to": {
        "turn": [0, 0.38268343, 0, 0.9238795], "scale": 2, "offset": [0, 1, 0] } }),
    );
    s.ok("view.lens", json!({ "focal_mm": 85 }));
    let want: Vec<String> = ["main", "ref:1", "ref:2"].map(|v| exact(&mut s, v)).into();
    let list = s.query("reference.list", json!({}));
    let (_, records) = log_records(&log);
    assert!(
        !records
            .iter()
            .any(|r| r.command == "view.orbit" && r.args["view"] == "main"),
        "the gesture on main is one view.set line"
    );
    let mut replay = human();
    replay.replay_log(&log);
    std::fs::remove_dir_all(&dir).ok();
    let got: Vec<String> = ["main", "ref:1", "ref:2"]
        .map(|v| exact(&mut replay, v))
        .into();
    assert_eq!(got, want, "every camera, bit for bit");
    assert_eq!(
        replay.query("reference.list", json!({})),
        list,
        "every setup"
    );
}
