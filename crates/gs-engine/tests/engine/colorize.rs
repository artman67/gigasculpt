//! Colorize, each part's paint switch (decision 0039, section 10, amended for P6):
//! `part.set_paint_visible` hides or shows the paint of a part, or of every part, as one undo
//! step and never changes it; a look draws a part whose paint is hidden as its material alone;
//! `part.list`, `part.get`, the scene hash and the effect report the switch, and copies keep it.

use std::collections::BTreeMap;

use gs_io::mesh::column;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::Session;
use gs_testkit::claims::run::Harness;
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::parts::row;
use crate::support::{EngineSession, agent};
use crate::views::log_path;

/// Red, as the color column holds it (red in the low byte).
const RED: u32 = 0xff00_00ff;

/// A unit sphere of `subdivisions`, as plain data, red above its equator when `painted`.
fn sphere(subdivisions: u32, painted: bool) -> PlainMesh {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": subdivisions }));
    let mut mesh = s.mesh("sphere");
    if painted {
        let colors = (mesh.positions.iter())
            .map(|p| if p[1] > 0.0 { RED } else { u32::MAX })
            .collect();
        let column = Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(colors),
        };
        mesh.attributes.insert("color".into(), column);
    }
    mesh
}

/// Whether part `name` shows its paint, as `part.list` reports it.
fn shows_paint(s: &mut Session, name: &str) -> Value {
    row(s, name)["paint_visible"].clone()
}

/// The PNG of a small front view, drawn the same way every time.
fn picture(s: &mut Session) -> String {
    let args = json!({ "view": "front", "size": 64, "antialias": false });
    let shot = s.query("view.snapshot", args);
    shot["png_base64"].as_str().unwrap_or_default().to_string()
}

/// A session that draws through the offscreen renderer (lavapipe), holding `mesh` as "ball".
fn drawing(mesh: &PlainMesh) -> Session {
    let mut s = agent();
    let renderer = gs_ui::Offscreen::host(None).unwrap_or_else(|e| panic!("{e}"));
    s.engine.attach_renderer(renderer);
    s.add_part("ball", mesh);
    s
}

/// Hiding a part's paint draws it exactly as the same part unpainted, and showing it again
/// gives back every byte of its color column and the picture it drew.
#[test]
fn hidden_paint_draws_the_material_alone_and_shows_again_unchanged() {
    let mut s = drawing(&sphere(8, true));
    let colors =
        |s: &mut Session| column(&s.mesh("ball"), "color", AttrDomain::Vertex).map(<[u32]>::to_vec);
    let (before, painted) = (colors(&mut s), picture(&mut s));
    let unpainted = picture(&mut drawing(&sphere(8, false)));
    assert_ne!(painted, unpainted, "the paint shows");
    s.ok("part.set_paint_visible", json!({ "visible": false }));
    assert_eq!(picture(&mut s), unpainted, "hidden: the material alone");
    assert_eq!(colors(&mut s), before, "hiding keeps every byte");
    s.ok("part.set_paint_visible", json!({ "visible": true }));
    assert_eq!(picture(&mut s), painted, "shown again: the same picture");
    assert_eq!(colors(&mut s), before, "and the same bytes");
}

/// A part never painted stores no color column whether its paint shows or not, and the scene
/// hashes as before once it shows again.
#[test]
fn an_unpainted_part_stores_no_color_column() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let plain = s.hash();
    s.ok("part.set_paint_visible", json!({ "visible": false }));
    assert_ne!(s.hash(), plain, "the hash sees the switch");
    s.ok("part.set_paint_visible", json!({ "visible": true }));
    assert_eq!(s.hash(), plain, "shown again: the scene as before");
    let got = s.query("part.get", json!({}));
    assert!(
        got["attributes"].get("color").is_none(),
        "no column: {}",
        got["attributes"]
    );
}

/// `every` sets every part, those whose eye is off too, as one undo step; it takes no part,
/// and a call that changes nothing says so and makes no step, as `part.set_visible` does.
#[test]
fn every_part_at_once_is_one_undo_step() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    s.ok("primitive.cylinder", json!({ "center": [-3, 0, 0] }));
    s.ok(
        "part.set_visible",
        json!({ "parts": ["cylinder"], "visible": false }),
    );
    let steps = s.history()["undoable"].as_u64().unwrap_or_default();
    let done = s.ok(
        "part.set_paint_visible",
        json!({ "visible": false, "every": true }),
    );
    let effect = done.effect.unwrap_or_else(|| panic!("an effect"));
    let named: BTreeMap<String, Vec<String>> = ["cube", "cylinder", "sphere"]
        .map(|p| (p.to_string(), vec!["paint_visible".to_string()]))
        .into();
    assert_eq!(effect.changed, named, "every part, its switch alone");
    assert_eq!((effect.verts_touched, effect.topology_changed), (0, false));
    for name in ["sphere", "cube", "cylinder"] {
        assert_eq!(shows_paint(&mut s, name), json!(false), "{name}");
    }
    assert_eq!(s.history()["undoable"], json!(steps + 1), "one step");
    for args in [
        json!({ "visible": false, "every": true }),
        json!({ "part": "cube", "visible": false }),
    ] {
        let again = s.ok("part.set_paint_visible", args.clone());
        let reason = (again.effect.as_ref()).and_then(|e| e.noop_reason.clone());
        assert!(reason.is_some(), "{args}: says it changed nothing");
        assert!(again.history_id.is_none(), "{args}: no step");
    }
    let e = s.err(
        "part.set_paint_visible",
        json!({ "part": "cube", "visible": true, "every": true }),
    );
    assert_eq!(
        (e.code, e.path.as_deref()),
        (ErrorCode::ArgsInvalid, Some("/part")),
        "{e:?}"
    );
    s.ok("history.undo", json!({}));
    for name in ["sphere", "cube", "cylinder"] {
        assert_eq!(
            shows_paint(&mut s, name),
            json!(true),
            "{name} after the undo"
        );
    }
}

/// One part at a time, the active one when none is named: undo and redo give back each state's
/// hash, the logged lines replay to the same scene, and `part.get` gives the switch beside the
/// mesh data only while the paint is hidden.
#[test]
fn undo_redo_and_the_log_keep_the_switch() {
    let log = log_path("colorize");
    let mut s = seeded(1, Some(log.clone()));
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({}));
    let before = s.hash();
    let done = s.ok("part.set_paint_visible", json!({ "visible": false }));
    assert_eq!(done.result["changed"], json!(["cube"]), "the active part");
    let once = s.hash();
    s.ok(
        "part.set_paint_visible",
        json!({ "part": "sphere", "visible": false }),
    );
    let twice = s.hash();
    assert_eq!(
        s.query("part.get", json!({ "part": "cube" }))["paint_visible"],
        json!(false)
    );
    let mut replay = seeded(2, None);
    replay.replay_log(&log);
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
    s.ok(
        "part.set_paint_visible",
        json!({ "part": "cube", "visible": true }),
    );
    assert_eq!(shows_paint(&mut s, "cube"), json!(true));
    assert_eq!(shows_paint(&mut s, "sphere"), json!(false));
    let shown = s.query("part.get", json!({ "part": "cube" }));
    assert!(
        shown.get("paint_visible").is_none(),
        "left out while shown: {}",
        shown["paint_visible"]
    );
}

/// The copy rule: a duplicate (placed or not, as the gizmo's Ctrl drag copy ends), a mirror
/// copy and every split piece hide their paint as the part does; a part made anew shows it.
#[test]
fn copies_keep_the_switch_and_new_parts_show_their_paint() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    s.ok("group.make", json!({ "maker": { "by": "normals" } }));
    s.ok("part.set_paint_visible", json!({ "visible": false }));
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
    s.ok("part.merge_visible", json!({ "name": "merged" }));
    let pieces = s.ok("part.split", json!({ "part": "cube", "by": "groups" }));
    let parts = pieces.result["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(parts.len(), 6, "a piece per side");
    let mut kept: Vec<String> = ["copy", "moved", "mirrored"].map(String::from).into();
    kept.extend(
        parts
            .iter()
            .map(|p| p["part"].as_str().unwrap_or_default().to_string()),
    );
    for name in &kept {
        assert_eq!(shows_paint(&mut s, name), json!(false), "{name}");
    }
    assert_eq!(shows_paint(&mut s, "merged"), json!(true), "made anew");
}

/// Both commands keep the claims decision 0039 gives them, and the three every edit keeps,
/// each checked on random plans of the two alone over the standard fixtures.
#[test]
fn colorize_and_fill_keep_their_claims() {
    let catalog = Session::new().engine.catalog().clone();
    let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
    let only = harness.only(&["part.set_paint_visible", "paint.fill"]);
    let tally = only.run_until_checked().unwrap_or_else(|e| panic!("{e}"));
    let claims = [
        ("part.set_paint_visible", "preserves_topology"),
        ("part.set_paint_visible", "positions_only"),
        ("paint.fill", "mask_holds"),
        ("paint.fill", "hidden_holds"),
    ];
    for (command, claim) in claims {
        assert!(
            tally.count(command, claim) >= 1,
            "{command}: {claim} was checked"
        );
    }
    for command in ["part.set_paint_visible", "paint.fill"] {
        for claim in ["deterministic", "undo_exact", "atomic"] {
            assert!(
                tally.count(command, claim) >= 1,
                "{command}: {claim} was checked"
            );
        }
    }
}
