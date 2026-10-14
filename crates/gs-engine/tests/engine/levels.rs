//! Detail levels through commands (decision 0014): a part subdivides keeping its numbering,
//! every level change is one undo step that undo and redo cross exactly (on one thread and on
//! many), deleting levels keeps what is left, level numbers count from 1 and stop at the ends,
//! topology tools refuse a part with levels, naming the remedies, whole-part mask and
//! visibility commands leave the same top from any level, replacing a part with levels
//! reports what `part.get` shows, and smooth UVs smooth the top's layout while each lower level
//! keeps its own.

use gs_core::subdivide_faces;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::claims::props::effect_matches;
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

use crate::support::{agent, levels, stroke};

/// A world-space Draw stroke over the sphere near `at`.
#[test]
fn a_cube_subdivides_to_26_points_keeping_its_numbering() {
    let mut s = agent();
    let cube = fixtures::cube(1);
    s.add_part("c", &cube);
    let made = s.ok("level.subdivide", json!({ "smooth": false }));
    assert_eq!(
        (made.result["count"].clone(), made.result["active"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(
        (
            made.result["vertices"].clone(),
            made.result["faces"].clone()
        ),
        (json!(26), json!(24))
    );
    assert!(made.effect.is_some_and(|e| e.topology_changed));
    let got: PlainMesh = s.mesh("c");
    let want = subdivide_faces(&cube.faces, 8).map(|x| x.0);
    assert_eq!(Some(got.faces), want, "decision 0006's numbering");
    assert_eq!(
        got.positions[..8],
        cube.positions[..],
        "a flat level keeps the cube's points"
    );
    let d = levels(&mut s);
    assert_eq!(d["levels"][0]["vertices"], json!(8));
    assert_eq!(d["levels"][1]["smooth"], json!(false));
}

#[test]
fn undo_and_redo_cross_level_changes_exactly_on_one_and_many_threads() {
    let script = || {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
        let calls = [
            ("level.subdivide", json!({})),
            ("level.subdivide", json!({})),
            ("brush.stroke", stroke([0.0, 0.0, 1.0])),
            ("level.set", json!({ "level": 1 })),
            ("brush.stroke", stroke([0.3, 0.0, 0.95])),
            ("level.step", json!({ "by": 1 })),
            ("brush.stroke", stroke([0.0, 0.3, 0.95])),
            ("level.set", json!({ "level": 3 })),
        ];
        let mut hashes = vec![s.hash()];
        for (command, args) in calls {
            s.ok(command, args);
            hashes.push(s.hash());
        }
        for i in (0..hashes.len() - 1).rev() {
            s.ok("history.undo", json!({}));
            assert_eq!(s.hash(), hashes[i], "after undoing back to step {i}");
        }
        for (i, want) in hashes.iter().enumerate().skip(1) {
            s.ok("history.redo", json!({}));
            assert_eq!(&s.hash(), want, "after redoing step {i}");
        }
        hashes
    };
    let one = gs_par::with_threads(1, script);
    let many = gs_par::with_threads(24, script);
    assert_eq!(one, many, "the same bits on 1 and 24 threads");
    assert_eq!(
        one.windows(2).filter(|w| w[0] == w[1]).count(),
        0,
        "every call changed the scene"
    );
}

#[test]
fn a_switch_with_pending_edits_carries_them_and_undo_brings_them_back() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.set", json!({ "level": 1 }));
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    assert_eq!(levels(&mut s)["pending"], json!(true));
    let low = s.mesh("sphere");
    let up = s.ok("level.set", json!({ "level": 2 }));
    assert!(up.result["carried"].as_u64().is_some_and(|n| n > 0));
    assert_eq!(levels(&mut s)["pending"], json!(false));
    s.ok("history.undo", json!({}));
    assert_eq!(
        json!(s.mesh("sphere")),
        json!(low),
        "the level-1 edit, still pending"
    );
    assert_eq!(levels(&mut s)["pending"], json!(true));
}

#[test]
fn deleting_higher_at_level_2_of_4_keeps_it_and_undoes_to_4() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    for _ in 0..3 {
        s.ok("level.subdivide", json!({}));
    }
    s.ok("level.set", json!({ "level": 2 }));
    let at2 = s.mesh("sphere");
    let before = s.hash();
    let cut = s.ok("level.delete_higher", json!({}));
    assert_eq!(
        (cut.result["count"].clone(), cut.result["active"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(json!(s.mesh("sphere")), json!(at2), "level 2 bit for bit");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before);
    assert_eq!(levels(&mut s)["count"], json!(4));
    s.ok("level.delete_lower", json!({}));
    let d = levels(&mut s);
    assert_eq!(
        (d["count"].clone(), d["active"].clone()),
        (json!(3), json!(1))
    );
    assert_eq!(
        json!(s.mesh("sphere")),
        json!(at2),
        "the new base is the old level 2"
    );
}

#[test]
fn level_numbers_count_from_one_and_stop_at_the_ends() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let err = s.err("level.set", json!({ "level": 2 }));
    assert_eq!(err.code, ErrorCode::ArgsInvalid);
    assert!(err.message.contains("levels 1 to 1"), "{}", err.message);
    s.ok("level.subdivide", json!({}));
    assert_eq!(
        s.err("level.set", json!({ "level": 0 })).code,
        ErrorCode::ArgsInvalid
    );
    let past = s.ok("level.step", json!({ "by": 5 }));
    let reason = past.effect.and_then(|e| e.noop_reason).unwrap_or_default();
    assert!(reason.contains("no level above"), "{reason}");
    let down = s.ok("level.step", json!({ "by": -9 }));
    assert_eq!(down.result["active"], json!(1), "stops at the base");
    let again = s.ok("level.set", json!({ "level": 1 }));
    assert!(again.effect.is_some_and(|e| e.noop_reason.is_some()));
}

#[test]
fn topology_tools_refuse_a_part_with_levels_and_name_the_remedies() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("level.subdivide", json!({}));
    let err = s.err("mesh.voxel_remesh", json!({ "resolution": 32 }));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(
        err.message.contains("'sphere'") && err.message.contains("2 detail levels"),
        "{}",
        err.message
    );
    let hint = err.hint.unwrap_or_default();
    assert!(
        hint.contains("level.delete_higher")
            && hint.contains("level.delete_lower")
            && hint.contains("level.freeze"),
        "{hint}"
    );
    s.ok("level.delete_lower", json!({}));
    s.ok("mesh.voxel_remesh", json!({ "resolution": 32 }));
}

/// A command that refuses frozen levels too (decision 0030, section 2) names only the remedy
/// that lets it run, keeping one level, and not `level.freeze`; following the hint, it runs.
#[test]
fn what_refuses_frozen_levels_too_names_only_keeping_one_level_and_then_runs() {
    let part = json!({ "part": "sphere" });
    for (command, args) in [
        ("mesh.flip", part.clone()),
        ("mesh.symmetrize", json!({ "part": "sphere", "axis": "x" })),
        ("part.mirror_copy", part.clone()),
        ("mesh.reorder", part.clone()),
        ("part.split", json!({ "part": "sphere", "by": "shells" })),
        ("part.merge_down", part.clone()),
        ("symmetry.map", json!({ "part": "sphere", "axis": "x" })),
        ("mesh.refine", json!({ "part": "sphere", "size": 0.2 })),
    ] {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
        s.ok("primitive.cube", json!({ "center": [0, 0, 3] }));
        for _ in 0..2 {
            s.ok("level.subdivide", part.clone());
        }
        let err = s.err(command, args.clone());
        assert_eq!(err.code, ErrorCode::Precondition, "{command}: {err:?}");
        let hint = err.hint.unwrap_or_default();
        assert!(
            hint.contains("level.delete_higher")
                && hint.contains("level.delete_lower")
                && !hint.contains("level.freeze"),
            "{command}: {hint}"
        );
        // The hint: level.delete_higher at the level to keep, then level.delete_lower.
        s.ok("level.set", json!({ "part": "sphere", "level": 2 }));
        s.ok("level.delete_higher", part.clone());
        s.ok("level.delete_lower", part.clone());
        s.ok(command, args);
    }
}

/// Creases on a part with levels, and hidden faces, whose region rule is not built (decision
/// 0037, section 5). A mask no longer stops a divide (tests/engine/masked.rs).
#[test]
fn creases_on_levels_and_hidden_faces_are_refused() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    s.ok("level.subdivide", json!({}));
    let creases = json!({ "creases": [{ "a": 0, "b": 1, "sharpness": 2 }] });
    assert_eq!(
        s.err("level.subdivide", creases).code,
        ErrorCode::Precondition
    );
    let mut cube = fixtures::cube(1);
    let hide = gs_protocol::Attribute {
        domain: gs_protocol::AttrDomain::Face,
        values: gs_protocol::AttrValues::U32(vec![0, 1, 0, 0, 0, 0]),
    };
    cube.attributes.insert("hide".into(), hide);
    s.add_part("hidden", &cube);
    let err = s.err("level.subdivide", json!({ "part": "hidden" }));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(err.message.contains("hidden faces"), "{}", err.message);
}

/// A sphere with three levels, at the top, with protection a lower level shows thinner: a soft
/// mask spot whose faces are hidden, and a second soft mask spot.
fn protected_on_the_top() -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    let pen = |at: [f32; 3]| {
        let samples = [at, [at[0] + 0.04, at[1] + 0.02, at[2]]].map(|p| json!({ "position": p }));
        json!({ "part": "sphere", "brush": "Mask Pen", "size": "6%", "samples": samples })
    };
    s.ok("brush.stroke", pen([0.0, 0.0, 1.0]));
    let masked = json!({ "by": "masked", "threshold": 0.01, "rule": "any" });
    s.ok("visibility.hide", json!({ "select": masked }));
    s.ok("brush.stroke", pen([0.6, 0.0, 0.8]));
    s
}

#[test]
fn whole_part_mask_and_visibility_commands_mean_the_same_from_a_lower_level() {
    // Review 0004, M2: run from level 2, each leaves the top as the same command run on the top.
    let commands = [
        ("visibility.show_all", json!({})),
        ("visibility.invert", json!({})),
        ("mask.set", json!({ "value": 0 })),
        ("mask.set", json!({ "value": 1 })),
        ("mask.set", json!({ "value": 0, "scope": "all" })),
        ("mask.invert", json!({})),
    ];
    for (command, args) in commands {
        let mut s = protected_on_the_top();
        s.ok(command, args.clone());
        let want = s.mesh("sphere");
        let mut s = protected_on_the_top();
        s.ok("level.step", json!({ "by": -1 }));
        s.ok(command, args.clone());
        s.ok("level.step", json!({ "by": 1 }));
        assert!(
            json!(s.mesh("sphere")) == json!(want),
            "{command} {args} from level 2 left another top"
        );
    }
    // And the way out the divide refusal names works from a lower level.
    let mut s = protected_on_the_top();
    s.ok("level.step", json!({ "by": -1 }));
    let err = s.err("level.subdivide", json!({}));
    assert!(err.hint.is_some_and(|h| h.contains("visibility.show_all")));
    s.ok("visibility.show_all", json!({}));
    s.ok("mask.set", json!({ "value": 0 }));
    s.ok("level.subdivide", json!({}));
}

#[test]
fn swapping_a_leveled_part_reports_what_part_get_shows() {
    // Each replaces a part with levels by one with new columns over the same faces: a
    // whole-part command from a lower level (run on the top, the level gathered again), a
    // restore that only drops the mask column, and restores that undo a few positions, group
    // ids or mask values, so only those vertices differ (review 0006, S1). None changes
    // topology, and every effect matches a diff of the scene, as the claims harness checks.
    let pen = |brush: &str, z: f32| {
        let samples = [[0.0, 0.0, z], [0.04, 0.02, z]].map(|p| json!({ "position": p }));
        json!({ "part": "c", "brush": brush, "size": 0.3, "samples": samples })
    };
    let faces = |indexes: &[u32], id: u32| json!({ "select": { "by": "faces", "indexes": indexes }, "id": id });
    let scripts = [
        (
            fixtures::icosphere(2),
            vec![
                ("level.subdivide", json!({ "part": "c", "smooth": true })),
                ("level.set", json!({ "part": "c", "level": 1 })),
                ("visibility.invert", json!({ "part": "c" })),
            ],
        ),
        (
            fixtures::cube(1),
            vec![
                ("level.subdivide", json!({ "smooth": true })),
                ("history.checkpoint", json!({ "name": "base" })),
                ("mask.invert", json!({})),
                ("history.restore", json!({ "name": "base" })),
            ],
        ),
        (
            fixtures::icosphere(2),
            vec![
                ("level.subdivide", json!({ "smooth": true })),
                ("group.assign", faces(&[0], 1)),
                ("brush.stroke", pen("Mask Pen", 1.0)),
                ("history.checkpoint", json!({ "name": "a" })),
                ("brush.stroke", pen("Draw", -1.0)),
                ("history.restore", json!({ "name": "a" })),
                ("history.checkpoint", json!({ "name": "b" })),
                ("group.assign", faces(&[5, 17], 2)),
                ("history.restore", json!({ "name": "b" })),
                ("history.checkpoint", json!({ "name": "c" })),
                ("brush.stroke", pen("Mask Pen", -1.0)),
                ("history.restore", json!({ "name": "c" })),
            ],
        ),
    ];
    for (mesh, calls) in scripts {
        let mut s = agent();
        s.add_part("c", &mesh);
        for (command, args) in calls {
            let (before, was) = (s.scene(), s.hash());
            let call = s.ok(command, args.clone());
            if command == "history.checkpoint" {
                continue; // not an edit: no effect
            }
            let changed = s.hash() != was;
            if let Err(e) = effect_matches(call.effect.as_ref(), &before, &s.scene(), changed) {
                panic!("{command} {args}: {e}");
            }
        }
    }
}

/// Review 0004, m1: the divide reads the top with pending edits carried up. A mask painted on a
/// lower level used to stop it there; since decision 0037 it carries up and comes through the
/// division, as a pending sculpt edit does.
#[test]
fn a_mask_painted_on_a_lower_level_carries_up_and_comes_through_the_divide() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    s.ok("level.step", json!({ "by": -1 }));
    let samples = [[0.0, 0.0, 1.0], [0.04, 0.02, 1.0]].map(|p| json!({ "position": p }));
    let pen = json!({ "part": "sphere", "brush": "Mask Pen", "size": "15%", "samples": samples });
    s.ok("brush.stroke", pen);
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    let made = s.ok("level.subdivide", json!({}));
    assert_eq!(made.result["count"], json!(4));
    assert!(made.result["carried"].as_u64().is_some_and(|n| n > 0));
    let top = s.mesh("sphere");
    let Some(gs_protocol::AttrValues::F32(mask)) = top.attributes.get("mask").map(|a| &a.values)
    else {
        panic!("the new top has no mask");
    };
    let near = |p: &[f32; 3]| p[2] > 0.95 && p[0].abs() < 0.03 && p[1].abs() < 0.03;
    let under = (top.positions.iter().zip(mask)).filter(|(p, _)| near(p));
    assert!(
        under.clone().count() > 0 && under.clone().all(|(_, &m)| m > 0.5),
        "the painted spot is masked on the new top"
    );
}

#[test]
fn creases_keep_a_cube_hard_for_their_levels() {
    let mut s = agent();
    let cube = fixtures::cube(1);
    s.add_part("c", &cube);
    let creases: Vec<Value> = (cube.faces.iter())
        .flat_map(|f| {
            (0..4).map(move |i| json!({ "a": f[i], "b": f[(i + 1) % 4], "sharpness": 1 }))
        })
        .collect();
    s.ok("level.subdivide", json!({ "creases": creases }));
    assert_eq!(
        s.mesh("c").positions[..8],
        cube.positions[..],
        "every corner has three sharp edges for one level"
    );
    s.ok("level.subdivide", json!({}));
    assert_ne!(
        s.mesh("c").positions[..8],
        cube.positions[..],
        "the creases ran out"
    );
}

/// The sphere of `primitive.sphere` with its points as UVs (u = x, v = y), the lower half's
/// faces shifted by 2 in u: two islands, their border round the middle.
fn uv_sphere(s: &mut Session) -> PlainMesh {
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    let mut mesh = s.mesh("sphere");
    let at = |f: &[u32; 4], i: usize| {
        let low = f
            .iter()
            .map(|&v| mesh.positions[v as usize][2])
            .sum::<f32>()
            < 0.0;
        let p = mesh.positions[f[i / 2] as usize];
        p[i % 2] + if low && i.is_multiple_of(2) { 2.0 } else { 0.0 }
    };
    let columns: Vec<(String, Attribute)> = (0..8)
        .map(|i| {
            let values = AttrValues::F32(mesh.faces.iter().map(|f| at(f, i)).collect());
            let name = format!("uv{}.{}", i / 2, ["u", "v"][i % 2]);
            let domain = AttrDomain::Face;
            (name, Attribute { domain, values })
        })
        .collect();
    mesh.attributes.extend(columns);
    mesh
}

/// Every UV corner value's bits, the columns in name order.
fn uv_bits(mesh: &PlainMesh) -> Vec<u32> {
    (mesh.attributes.iter())
        .filter(|(name, _)| name.starts_with("uv"))
        .flat_map(|(_, a)| match &a.values {
            AttrValues::F32(x) => x.iter().map(|x| x.to_bits()).collect(),
            AttrValues::U32(_) => Vec::new(),
        })
        .collect()
}

/// Smooth UVs (decision 0014): the top's layout is smoothed where the default only
/// interpolates, its points and faces are the default's bit for bit, and each lower level shows
/// the layout it had: level 1 the part's own, level 2 what one divide made.
#[test]
fn smooth_uvs_smooth_the_top_layout_and_each_lower_level_keeps_its_own() {
    let mut s = agent();
    let mesh = uv_sphere(&mut s);
    let mut divide = |name: &str, times: usize, smooth_uvs: bool| {
        s.add_part(name, &mesh);
        for _ in 0..times {
            let args = json!({ "part": name, "smooth_uvs": smooth_uvs });
            s.ok("level.subdivide", args);
        }
        s.mesh(name)
    };
    let (on, off, once) = (
        divide("on", 2, true),
        divide("off", 2, false),
        divide("once", 1, true),
    );
    let points = |m: &PlainMesh| m.positions.iter().flatten().map(|x| x.to_bits()).collect();
    let (on_points, off_points): (Vec<u32>, Vec<u32>) = (points(&on), points(&off));
    assert!(on_points == off_points && on.faces == off.faces);
    let moved = (uv_bits(&on).iter().zip(uv_bits(&off)))
        .filter(|(a, b)| **a != *b)
        .count();
    assert!(moved > on.faces.len(), "{moved} corner values smoothed");
    s.ok("level.set", json!({ "part": "on", "level": 2 }));
    assert!(
        uv_bits(&s.mesh("on")) == uv_bits(&once),
        "level 2: one divide's layout"
    );
    s.ok("level.set", json!({ "part": "on", "level": 1 }));
    assert!(
        uv_bits(&s.mesh("on")) == uv_bits(&mesh),
        "level 1: the part's own layout"
    );
}
