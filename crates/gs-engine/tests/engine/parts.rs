//! The part list (decision 0015): stable ids that are never reused and that a failed call does
//! not use up, the three ways to name a part, the list order. That a replay hands out the same
//! ids is in `replay`. Remesh visible (`part.remesh`) takes hole closing as the other voxel calls
//! do.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::voxel_mode::{open_edges, open_tube};

/// `part.list` as (id, place, name) rows.
pub(crate) fn rows(s: &mut Session) -> Vec<(u64, u64, String)> {
    let list = s.query("part.list", json!({}));
    let parts = list["parts"].as_array().cloned().unwrap_or_default();
    parts
        .iter()
        .map(|p| {
            let n = |k: &str| p[k].as_u64().unwrap_or(u64::MAX);
            (
                n("id"),
                n("index"),
                p["name"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// Part `name`'s row of `part.list`.
pub(crate) fn row(s: &mut Session, name: &str) -> Value {
    let list = s.query("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    (rows.into_iter())
        .find(|row| row["name"] == json!(name))
        .unwrap_or_else(|| panic!("no part {name} in {list}"))
}

fn made(s: &mut Session, command: &str, args: Value) -> (u64, u64) {
    let done = s.ok(command, args);
    let n = |k: &str| done.result[k].as_u64().unwrap_or(u64::MAX);
    (n("id"), n("index"))
}

#[test]
fn a_part_is_named_by_its_id_its_place_or_its_name() {
    let mut s = Session::new();
    assert_eq!(made(&mut s, "primitive.sphere", json!({})), (1, 0));
    assert_eq!(made(&mut s, "primitive.cube", json!({})), (2, 1));
    let stats = |s: &mut Session, part: &str| s.query("mesh.stats", json!({ "part": part }));
    let want = stats(&mut s, "cube");
    for part in ["#2", "@1"] {
        assert_eq!(stats(&mut s, part), want, "{part} is the cube");
    }
    assert_ne!(stats(&mut s, "@0"), want, "@0 is the sphere");
    for part in ["#3", "@2", "#x", "cubes"] {
        let e = s.err("mesh.stats", json!({ "part": part }));
        assert_eq!(e.code, ErrorCode::NotFound, "{part}: {e:?}");
    }
    // An edit through an id moves the part the id names.
    s.ok(
        "mesh.translate",
        json!({ "part": "#1", "direction": [1, 0, 0], "distance": 2 }),
    );
    let moved = s.query("mesh.bounds", json!({ "part": "sphere" }));
    assert_eq!(moved["center"], json!([2.0, 0.0, 0.0]), "the sphere moved");
    for name in ["#7", "@cube"] {
        let e = s.err("primitive.cube", json!({ "name": name }));
        assert_eq!(
            e.code,
            ErrorCode::Precondition,
            "{name} is refused as a new name: {e:?}"
        );
    }
    let text = s.query("scene.describe", json!({}))["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        text.contains("#1 @0 sphere") && text.contains("#2 @1 cube"),
        "{text}"
    );
}

#[test]
fn ids_are_never_reused_and_a_failed_call_uses_none_up() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({}));
    s.ok("history.checkpoint", json!({ "name": "base" }));
    s.ok("history.undo", json!({}));
    assert_eq!(
        rows(&mut s),
        [(1, 0, "sphere".into())],
        "the cube is undone"
    );
    assert_eq!(
        made(&mut s, "primitive.cube", json!({})),
        (3, 1),
        "a new cube, a new id"
    );
    // A batch that adds a part and then fails rolls back, id and all.
    let failed = s.batch(&[
        ("primitive.cube", json!({ "name": "spare" })),
        (
            "mesh.translate",
            json!({ "part": "nowhere", "direction": [1, 0, 0], "distance": 1 }),
        ),
    ]);
    assert!(
        matches!(failed.outcome, gs_protocol::Outcome::Error(_)),
        "the batch fails"
    );
    assert_eq!(
        made(&mut s, "primitive.cube", json!({ "name": "other" })),
        (4, 2)
    );
    // A checkpoint brings parts back with the ids they had.
    s.ok("history.restore", json!({ "name": "base" }));
    let names: Vec<(u64, String)> = rows(&mut s).into_iter().map(|r| (r.0, r.2)).collect();
    assert_eq!(
        names,
        [(1, "sphere".into()), (2, "cube".into())],
        "as saved"
    );
    assert_eq!(
        made(&mut s, "primitive.cube", json!({ "name": "last" })),
        (5, 2)
    );
}

/// The part names in list order.
fn order(s: &mut Session) -> Vec<String> {
    rows(s).into_iter().map(|r| r.2).collect()
}

/// The parts that show, in list order.
fn showing(s: &mut Session) -> Vec<String> {
    let list = s.query("part.list", json!({}));
    let parts = list["parts"].as_array().cloned().unwrap_or_default();
    (parts.iter())
        .filter(|p| p["shown"] == json!(true))
        .map(|p| p["name"].as_str().unwrap_or("").to_string())
        .collect()
}

/// A sphere, a cube and a sphere named head, in that order.
pub(crate) fn three() -> Session {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({ "center": [0, 0, 3] }));
    s.ok(
        "primitive.sphere",
        json!({ "name": "head", "center": [3, 0, 0] }),
    );
    s
}

/// Every part command (decision 0028) changes the scene, and one undo puts back exactly the
/// scene and list before it, and one redo exactly the one after.
#[test]
fn every_list_command_undoes_and_redoes_exactly_in_one_step() {
    let calls = [
        ("part.duplicate", json!({ "part": "cube" })),
        ("part.add", json!({ "shape": "cube", "after": "sphere" })),
        ("part.delete", json!({ "part": "cube" })),
        ("part.delete", json!({ "part": "head", "scope": "others" })),
        ("part.rename", json!({ "part": "head", "name": "skull" })),
        ("part.reorder", json!({ "part": "head", "to": "top" })),
        (
            "part.reorder",
            json!({ "part": "sphere", "to": { "place": 1 } }),
        ),
        (
            "part.set_visible",
            json!({ "parts": ["cube"], "visible": false }),
        ),
        ("part.mirror_copy", json!({ "part": "head" })),
        ("part.split", json!({ "part": "cube", "by": "groups" })),
        (
            "part.split",
            json!({ "part": "cube", "by": "groups", "keep": true }),
        ),
        ("part.merge_down", json!({ "part": "sphere" })),
        ("part.merge_visible", json!({})),
        ("part.extract", json!({ "part": "head" })),
        ("part.remesh", json!({ "resolution": 16 })),
    ];
    for (command, args) in calls {
        let mut s = three();
        s.ok(
            "group.make",
            json!({ "part": "cube", "maker": { "by": "normals" } }),
        );
        let (hash, list) = (s.hash(), s.query("part.list", json!({})));
        s.ok(command, args.clone());
        let (after, changed) = (s.hash(), s.query("part.list", json!({})));
        assert_ne!(after, hash, "{command} {args} changes the scene");
        s.ok("history.undo", json!({}));
        assert_eq!(s.hash(), hash, "{command} {args}: undone");
        assert_eq!(
            s.query("part.list", json!({})),
            list,
            "{command}: the list too"
        );
        s.ok("history.redo", json!({}));
        assert_eq!(s.hash(), after, "{command} {args}: redone");
        assert_eq!(
            s.query("part.list", json!({})),
            changed,
            "{command}: the list too"
        );
    }
}

#[test]
fn copies_and_moves_go_where_the_list_rules_say() {
    let mut s = three();
    s.ok("part.duplicate", json!({ "part": "sphere" }));
    s.ok("part.add", json!({ "shape": "cube", "after": "cube" }));
    s.ok(
        "part.mirror_copy",
        json!({ "part": "head", "name": "head-left" }),
    );
    let want = ["sphere", "sphere-2", "cube", "cube-2", "head", "head-left"];
    assert_eq!(order(&mut s), want, "each below its source");
    s.ok("part.reorder", json!({ "part": "head", "to": "up" }));
    s.ok("part.reorder", json!({ "part": "sphere", "to": "bottom" }));
    let want = ["sphere-2", "cube", "head", "cube-2", "head-left", "sphere"];
    assert_eq!(order(&mut s), want);
    let nothing = s.ok("part.reorder", json!({ "part": "sphere-2", "to": "top" }));
    assert!(noop(&nothing), "already at the top");
    // The mirror copy is the head mirrored across x = 0, faces turned to face out.
    let (head, left) = (s.mesh("head"), s.mesh("head-left"));
    let mirrored: Vec<[f32; 3]> = head.positions.iter().map(|p| [-p[0], p[1], p[2]]).collect();
    assert_eq!(left.positions, mirrored);
    let [a, b, c, d] = head.faces[0];
    assert_eq!(left.faces[0], [a, d, c, b]);
    let volume = |s: &mut Session, part: &str| {
        let stats = s.query("mesh.stats", json!({ "part": part }));
        stats["parts"][0]["volume"].as_f64().unwrap_or(f64::NAN)
    };
    let (want, got) = (volume(&mut s, "head"), volume(&mut s, "head-left"));
    assert!(
        want > 0.0 && (got - want).abs() < 1e-6 * want,
        "still outward: {got} {want}"
    );
}

/// The catalog check of `parts.list` (decision 0015): a part keeps its id through a reorder, a
/// rename and the deletion of another part, named by its id or its place, and a duplicate gets
/// a new id.
#[test]
fn a_part_keeps_its_id_through_a_reorder_a_rename_and_another_deleted() {
    let mut s = three();
    s.ok("part.reorder", json!({ "part": "head", "to": "top" }));
    s.ok("part.rename", json!({ "part": "#3", "name": "face" }));
    s.ok("part.delete", json!({ "part": "@2" }));
    assert_eq!(
        rows(&mut s),
        [(3, 0, "face".into()), (1, 1, "sphere".into())],
        "the cube deleted, the others with their ids"
    );
    let copy = made(&mut s, "part.duplicate", json!({ "part": "@0" }));
    assert_eq!(copy.0, 4, "a duplicate gets a new id");
    let ids: Vec<u64> = rows(&mut s).into_iter().map(|r| r.0).collect();
    assert_eq!(
        ids,
        [3, 4, 1],
        "the copy below its source, the others as they were"
    );
}

/// Stable ids (decision 0015): a deleted part's id is never handed out again, and undoing the
/// delete brings the part back with its id.
#[test]
fn ids_are_never_reused_after_a_delete() {
    let mut s = three();
    s.ok("part.delete", json!({ "part": "head" }));
    assert_eq!(
        made(&mut s, "part.duplicate", json!({ "part": "cube" })).0,
        4
    );
    s.ok("part.delete", json!({ "scope": "all" }));
    assert_eq!(rows(&mut s), [], "an empty scene");
    assert_eq!(
        made(&mut s, "part.add", json!({ "shape": "sphere" })),
        (5, 0)
    );
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    let ids: Vec<u64> = rows(&mut s).into_iter().map(|r| r.0).collect();
    assert_eq!(ids, [1, 2, 4], "the parts the delete took, with their ids");
}

/// A part that does not show is not hit by a pick that names no part, nor framed; one that
/// names it still reaches it. The active part shows whatever its eye says (decision 0033).
/// Isolate shows the parts it chose, eye on or off, making the first active, and ending it
/// shows exactly what showed before once another part is active.
#[test]
fn a_part_that_does_not_show_is_skipped_and_isolate_restores_the_eyes() {
    let mut s = three();
    let ray = |s: &mut Session, part: Option<&str>| {
        let mut args = json!({ "origin": [0, 0, 10], "direction": [0, 0, -1] });
        if let Some(part) = part {
            args["part"] = json!(part);
        }
        s.query("pick.ray", args)["hit"]["part"].clone()
    };
    assert_eq!(ray(&mut s, None), json!("cube"), "the cube is in front");
    s.ok(
        "part.set_visible",
        json!({ "parts": ["cube"], "visible": false }),
    );
    assert_eq!(
        ray(&mut s, None),
        json!("sphere"),
        "a hidden cube is not picked"
    );
    assert_eq!(
        ray(&mut s, Some("cube")),
        json!("cube"),
        "unless it is named"
    );
    s.ok("part.set_visible", json!({ "visible": false }));
    assert_eq!(showing(&mut s), ["head"], "the active part still shows");
    s.ok(
        "part.set_visible",
        json!({ "parts": ["cube"], "visible": true, "others": true }),
    );
    assert_eq!(showing(&mut s), ["sphere", "head"]);
    s.ok("part.isolate", json!({ "parts": ["cube"] }));
    assert_eq!(showing(&mut s), ["cube"], "isolated, its eye off or not");
    assert_eq!(ray(&mut s, None), json!("cube"));
    let active = s.query("session.state", json!({}))["parts.active"].clone();
    assert_eq!(active, json!("cube"), "the isolated part is active");
    s.ok("part.isolate", json!({ "parts": ["cube"] }));
    assert_eq!(
        showing(&mut s),
        ["sphere", "cube", "head"],
        "the same parts, so it ends; the active cube shows, its eye off"
    );
    s.ok("part.select", json!({ "part": "head" }));
    assert_eq!(showing(&mut s), ["sphere", "head"], "its eye takes effect");
    let off = s.ok("part.isolate", json!({}));
    assert_eq!(off.result["changed"], json!([]), "Isolate is already off");
    s.ok("part.delete", json!({ "scope": "hidden" }));
    assert_eq!(
        order(&mut s),
        ["sphere", "head"],
        "the hidden cube is deleted"
    );
}

/// Decision 0029, section 9: Isolate is session state. An undo leaves it on, and a part an
/// undo takes away keeps its mark when a redo brings it back; a duplicate takes its source's
/// mark; a failed batch leaves Isolate as it was (`part.isolate` is a View command, which a
/// batch refuses, so the batch changes Isolate with a copy); deleting every chosen part ends
/// it. `parts.isolated` follows it throughout.
#[test]
fn isolate_is_not_undone_and_parts_isolated_follows_it() {
    let isolated = |s: &mut Session| s.query("session.state", json!({}))["parts.isolated"].clone();
    let mut s = three();
    assert_eq!(isolated(&mut s), json!(false));
    s.ok("part.isolate", json!({ "parts": ["cube"] }));
    assert_eq!(isolated(&mut s), json!(true));
    s.ok("history.undo", json!({}));
    assert_eq!(
        order(&mut s),
        ["sphere", "cube"],
        "the undo took the head away"
    );
    assert_eq!(showing(&mut s), ["cube"], "and left Isolate on");
    s.ok("history.redo", json!({}));
    assert_eq!(
        showing(&mut s),
        ["cube"],
        "the head is back, still set aside"
    );
    s.ok("part.duplicate", json!({ "part": "cube", "name": "twin" }));
    assert_eq!(showing(&mut s), ["cube", "twin"], "the copy is chosen too");
    let failed = s.batch(&[
        ("part.duplicate", json!({ "part": "cube", "name": "ghost" })),
        ("part.rename", json!({ "part": "nowhere", "name": "x" })),
    ]);
    assert!(matches!(failed.outcome, gs_protocol::Outcome::Error(_)));
    // The failed copy's id goes to the next part, which must not have the copy's mark.
    s.ok("part.add", json!({ "shape": "cube", "name": "new" }));
    let list = s.query("part.list", json!({}));
    let new = (list["parts"].as_array().into_iter().flatten()).find(|p| p["name"] == "new");
    assert_eq!(
        new.map(|p| p["isolate"].clone()),
        Some(json!("off")),
        "{list}"
    );
    s.ok("part.delete", json!({ "part": "cube" }));
    s.ok("part.delete", json!({ "part": "twin" }));
    assert_eq!(isolated(&mut s), json!(false), "no chosen part is left");
    assert_eq!(
        showing(&mut s),
        order(&mut s),
        "every part shows by its eye"
    );
}

/// A log of list commands replays to the same scene and the same list, the active part
/// included.
#[test]
fn a_log_of_list_commands_replays_to_the_same_scene() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("parts-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("parts.jsonl");
    let mut s = crate::determinism::seeded(1, Some(log.clone()));
    for (command, args) in [
        ("primitive.sphere", json!({})),
        ("primitive.cube", json!({})),
        ("part.duplicate", json!({ "part": "@0" })),
        ("part.add", json!({ "shape": "sphere", "after": "#2" })),
        ("part.reorder", json!({ "part": "@3", "to": "top" })),
        ("part.rename", json!({ "part": "#1", "name": "body" })),
        (
            "part.set_visible",
            json!({ "parts": ["@1"], "visible": false }),
        ),
        ("part.isolate", json!({ "parts": ["body"] })),
        ("part.select", json!({ "part": "@0" })),
        ("part.select", json!({ "neighbor": "next" })),
        ("part.mirror_copy", json!({ "part": "body", "axis": "z" })),
        ("part.delete", json!({ "part": "#3" })),
        ("part.isolate", json!({})),
        (
            "group.make",
            json!({ "part": "cube", "maker": { "by": "normals" } }),
        ),
        ("part.split", json!({ "part": "cube", "by": "groups" })),
        ("part.merge_down", json!({ "part": "cube" })),
        ("part.extract", json!({ "part": "body", "thickness": "2%" })),
        ("part.merge_visible", json!({})),
        ("part.remesh", json!({ "resolution": 16, "name": "blob" })),
    ] {
        s.ok(command, args);
    }
    let mut replay = crate::determinism::seeded(2, None);
    crate::support::EngineSession::replay_log(&mut replay, &log);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(replay.hash(), s.hash());
    let list = |s: &mut Session| s.query("part.list", json!({}));
    assert_eq!(list(&mut replay), list(&mut s));
}

/// Order, eyes and Isolate save and open with the scene; a part with detail levels is not
/// mirrored.
#[test]
fn order_and_visibility_save_and_open_and_levels_are_refused() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("parts-scene-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path = dir.join("list.gss").to_string_lossy().into_owned();
    let mut s = three();
    s.ok("part.reorder", json!({ "part": "head", "to": "top" }));
    s.ok(
        "part.set_visible",
        json!({ "parts": ["cube"], "visible": false }),
    );
    s.ok("part.isolate", json!({ "parts": ["sphere", "cube"] }));
    s.ok("part.rename", json!({ "part": "head", "name": "tête 頭" }));
    s.ok("file.save", json!({ "path": path }));
    let mut opened = Session::new();
    opened.ok("file.open", json!({ "path": path }));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(opened.hash(), s.hash());
    let list = |s: &mut Session| s.query("part.list", json!({}));
    assert_eq!(list(&mut opened), list(&mut s));
    assert_eq!(order(&mut opened)[0], "tête 頭", "a Unicode name survives");
    s.ok("level.subdivide", json!({ "part": "tête 頭" }));
    let e = s.err("part.mirror_copy", json!({ "part": "tête 頭" }));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
}

/// The catalog checks of `parts.reorder` and `parts.rename`: in a list of eight parts, Move to
/// Top on the fifth makes it first and keeps the others' order, Move to Bottom puts it last; a
/// rename changes only that name (that it survives a save is in the test above), and an empty
/// name is refused.
#[test]
fn eight_parts_move_to_the_top_and_bottom_and_a_rename_changes_only_the_name() {
    let mut s = Session::new();
    let names: Vec<String> = (1..=8).map(|k| format!("p{k}")).collect();
    for name in &names {
        s.ok("primitive.cube", json!({ "name": name }));
    }
    s.ok("part.reorder", json!({ "part": "@4", "to": "top" }));
    let mut want = names.clone();
    let fifth = want.remove(4);
    want.insert(0, fifth.clone());
    assert_eq!(order(&mut s), want);
    s.ok("part.reorder", json!({ "part": "p5", "to": "bottom" }));
    want.remove(0);
    want.push(fifth);
    assert_eq!(order(&mut s), want);

    let before = rows(&mut s);
    s.ok("part.rename", json!({ "part": "p2", "name": "Kopf ✋" }));
    let after = rows(&mut s);
    let changed: Vec<_> = before.iter().zip(&after).filter(|(a, b)| a != b).collect();
    assert_eq!(changed.len(), 1, "one row changed: {changed:?}");
    let (was, now) = changed[0];
    assert_eq!((now.0, now.1, now.2.as_str()), (was.0, was.1, "Kopf ✋"));
    let e = s.err("part.rename", json!({ "part": "p3", "name": "" }));
    assert_eq!(e.code, ErrorCode::ArgsInvalid, "{e:?}");
}

/// The catalog checks of `parts.split-shells`, `parts.split-by-mask` and `parts.extract`.
#[test]
fn shells_mask_and_extract_meet_their_catalog_checks() {
    // Three shells become three parts, and one undo gives back the single part.
    let mut s = three();
    s.ok("part.merge_visible", json!({ "name": "all" }));
    let hash = s.hash();
    let done = s.ok("part.split", json!({ "part": "all", "by": "shells" }));
    assert_eq!(done.result["parts"].as_array().map(Vec::len), Some(3));
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), hash, "one undo joins them again");

    // A strip of three quads, masked from x = 2: the middle quad's corner mean is exactly 0.5,
    // which counts as masked. Paint (a vertex color) and groups carry to both parts.
    let mut s = Session::new();
    let positions = (0..8)
        .map(|v| [(v % 4) as f32, (v / 4) as f32, 0.0])
        .collect();
    let mask = (0..8).map(|v| f32::from(u8::from(v % 4 >= 2))).collect();
    let columns = [
        ("mask", AttrDomain::Vertex, AttrValues::F32(mask)),
        (
            "color",
            AttrDomain::Vertex,
            AttrValues::U32((0..8).map(|v| v * 0x0101_0101).collect()),
        ),
        ("group", AttrDomain::Face, AttrValues::U32(vec![1, 2, 3])),
    ];
    let strip = PlainMesh {
        positions,
        faces: (0..3).map(|k| [k, k + 1, k + 5, k + 4]).collect(),
        attributes: (columns.into_iter())
            .map(|(name, domain, values)| (name.to_string(), Attribute { domain, values }))
            .collect::<BTreeMap<_, _>>(),
    };
    s.add_part("strip", &strip);
    let whole = face_records(&mut s, "strip");
    s.ok("part.split", json!({ "part": "strip", "by": "mask" }));
    assert_eq!(order(&mut s), ["strip", "strip-masked"]);
    let (unmasked, masked) = (
        face_records(&mut s, "strip"),
        face_records(&mut s, "strip-masked"),
    );
    assert_eq!((unmasked.len(), masked.len()), (1, 2));
    let mut both = [unmasked, masked].concat();
    both.sort();
    assert_eq!(both, whole, "every face, color and group carried");

    // Extract a masked patch with Double on: three face groups. Showing only the outer one and
    // extracting again with Double off gives a shell outside the first one only.
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({}));
    let pen = json!({ "brush": "Mask Pen", "size": "25%", "samples": [{ "position": [0, 0, 1] }] });
    s.ok("brush.stroke", pen);
    s.ok("part.extract", json!({ "double": true, "name": "armor" }));
    let groups = s.query("group.list", json!({ "part": "armor" }));
    let ids: Vec<u64> = (groups["groups"].as_array().into_iter().flatten())
        .filter_map(|g| g["id"].as_u64())
        .collect();
    assert_eq!(ids, [1, 2, 3], "front, back and rim");
    let outer = json!({ "part": "armor", "select": { "by": "groups", "ids": [1] } });
    s.ok("visibility.only", outer);
    s.ok(
        "part.extract",
        json!({ "part": "armor", "name": "outside" }),
    );
    let radii = |s: &mut Session, part: &str| {
        let mesh = s.mesh(part);
        let r = (mesh.positions.iter()).map(|p| p.iter().map(|c| c * c).sum::<f32>().sqrt());
        r.fold((f32::MAX, f32::MIN), |(lo, hi), r| (lo.min(r), hi.max(r)))
    };
    let (sphere, armor, outside) = (
        radii(&mut s, "sphere"),
        radii(&mut s, "armor"),
        radii(&mut s, "outside"),
    );
    assert!(
        armor.0 < sphere.0,
        "Double reaches inside: {armor:?} {sphere:?}"
    );
    assert!(
        outside.0 > sphere.1,
        "the second shell is outside only: {outside:?} {sphere:?}"
    );
}

/// Everything each face of a part says, sorted: its corners' positions, then every column's
/// values at its corners or at the face.
fn face_records(s: &mut Session, part: &str) -> Vec<String> {
    let mesh = s.mesh(part);
    let mut records: Vec<String> = (0..mesh.faces.len())
        .map(|f| {
            let corners: Vec<usize> = (mesh.faces[f].iter())
                .filter(|&&v| v != NO_VERTEX)
                .map(|&v| v as usize)
                .collect();
            let points: Vec<[f32; 3]> = corners.iter().map(|&v| mesh.positions[v]).collect();
            let mut record = vec![format!("{points:?}")];
            for (name, column) in &mesh.attributes {
                let at = match column.domain {
                    AttrDomain::Vertex => corners.clone(),
                    AttrDomain::Face => vec![f],
                };
                let values = match &column.values {
                    AttrValues::F32(v) => {
                        format!("{:?}", at.iter().map(|&i| v[i]).collect::<Vec<_>>())
                    }
                    AttrValues::U32(v) => {
                        format!("{:?}", at.iter().map(|&i| v[i]).collect::<Vec<_>>())
                    }
                };
                record.push(format!("{name}={values}"));
            }
            record.join(" ")
        })
        .collect();
    records.sort();
    records
}

/// Splitting a cube by its six groups gives six parts whose faces add up to the cube's, each
/// below the last; merging them back down gives every face and group again. A hidden part is
/// left out of merge visible and remesh; a part with levels is not split.
#[test]
fn split_by_groups_and_merge_down_give_back_every_face() {
    let mut s = three();
    s.ok(
        "group.make",
        json!({ "part": "cube", "maker": { "by": "normals" } }),
    );
    let whole = face_records(&mut s, "cube");
    let done = s.ok("part.split", json!({ "part": "cube", "by": "groups" }));
    let parts = done.result["parts"].as_array().cloned().unwrap_or_default();
    let names: Vec<String> = (parts.iter())
        .map(|p| p["part"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert_eq!(names[0], "cube", "the first piece keeps the name");
    assert_eq!(
        order(&mut s)[1..7],
        names[..],
        "the pieces sit together, in order"
    );
    let mut pieces: Vec<String> = names.iter().flat_map(|n| face_records(&mut s, n)).collect();
    pieces.sort();
    assert_eq!(
        pieces, whole,
        "the pieces' faces add up to the cube's exactly"
    );
    for _ in 1..6 {
        s.ok("part.merge_down", json!({ "part": "cube" }));
    }
    assert_eq!(order(&mut s), ["sphere", "cube", "head"]);
    assert_eq!(
        face_records(&mut s, "cube"),
        whole,
        "every face and group is back"
    );
    assert_eq!(rows(&mut s)[1].0, 2, "with the cube's id");

    s.ok(
        "part.set_visible",
        json!({ "parts": ["cube"], "visible": false }),
    );
    let remeshed = s.ok("part.remesh", json!({ "resolution": 24 }));
    let from = &remeshed.result["from"];
    assert_eq!(
        from,
        &json!(["sphere", "head"]),
        "the hidden cube is left out"
    );
    let groups = s.query("group.list", json!({ "part": "remeshed" }));
    let ids: Vec<u64> = (groups["groups"].as_array().into_iter().flatten())
        .filter_map(|g| g["id"].as_u64())
        .collect();
    assert_eq!(ids, [1, 2], "one group per part remeshed");
    let merged = s.ok("part.merge_visible", json!({}));
    let from = &merged.result["from"];
    assert_eq!(
        from,
        &json!(["sphere", "head", "remeshed"]),
        "the hidden cube is left out"
    );
    assert_eq!(merged.result["part"]["part"], json!("merged-sphere"));
    s.ok("level.subdivide", json!({ "part": "head" }));
    let e = s.err("part.split", json!({ "part": "head", "by": "shells" }));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
}

/// Remesh visible takes hole closing as the other voxel calls do (decision 0052 as amended by
/// step Prefs-c): left out, the session's, written into the logged call.
#[test]
fn remesh_visible_with_hole_closing_0_leaves_an_open_part_open_logs_0_and_replays_the_same_at_1() {
    let dir = crate::socket::scratch("remesh-visible-closing");
    let log = dir.join("remesh.jsonl");
    let mut s = crate::determinism::seeded(0, Some(log.clone()));
    s.ok("mesh.voxel_settings", json!({ "close_holes": 0 }));
    s.ok(
        "part.create",
        json!({ "name": "tube", "mesh": open_tube(32) }),
    );
    s.ok("part.remesh", json!({ "resolution": 24 }));
    assert!(
        !open_edges(&s.mesh("remeshed")).is_empty(),
        "the ends stay open"
    );
    let (_, records) = crate::support::log_records(&log);
    let logged: Vec<Value> = (records.iter())
        .filter(|r| r.command == "part.remesh")
        .map(|r| r.args["close_holes"].clone())
        .collect();
    assert_eq!(logged, [json!(0)], "the logged call says 0");
    // Replayed where the hole closing is 1, without the call that set it to 0.
    let mut replay = crate::determinism::seeded(0, None);
    for r in (records.iter()).filter(|r| r.command != "mesh.voxel_settings") {
        let request = r.request().unwrap_or_else(|e| panic!("{e}"));
        gs_testkit::success(replay.request(request), &format!("{r:?}"));
    }
    assert_eq!(replay.hash(), s.hash(), "the same scene");
    assert!(
        !open_edges(&replay.mesh("remeshed")).is_empty(),
        "still open"
    );
    // Given 1, the same part comes out closed.
    let args = json!({ "parts": ["tube"], "resolution": 24, "close_holes": 1, "name": "closed" });
    s.ok("part.remesh", args);
    assert!(open_edges(&s.mesh("closed")).is_empty(), "1 closes them");
    std::fs::remove_dir_all(dir).ok();
}

/// Whether a call changed nothing and said why.
fn noop(done: &gs_protocol::Success) -> bool {
    (done.effect.as_ref()).is_some_and(|e| e.noop_reason.is_some())
}
