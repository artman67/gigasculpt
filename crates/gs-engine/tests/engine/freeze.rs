//! Frozen detail levels (decision 0030): freeze leaves the base and gives the levels back
//! exactly when nothing changed it, voxel remesh runs on a frozen part and unfreeze projects
//! the old top onto the new base within the stated tolerance (a dense remesh undivided, a
//! distance of 0 refused) with its color and mask, a stroke or a mask on the frozen base comes
//! back as level 1's edits, a frozen part whose transform is written into its points unfreezes
//! as the part left placed and a distance in units reaches as far on it, undo and redo cross
//! every step exactly, and what would drop the levels refuses a frozen part, naming
//! `level.unfreeze`.

use gs_engine::Request;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh, RunParams};
use gs_testkit::measure::{self, norm, sub};
use gs_testkit::{Session, failure, fixtures, text};
use serde_json::{Value, json};

use crate::support::{agent, levels, stroke, worst};

/// A sphere with three levels, detail sculpted on its top and an edit pending on level 2.
fn detailed() -> Session {
    detailed_with(3)
}

/// [`detailed`] with `count` levels.
fn detailed_with(count: u32) -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    for _ in 1..count {
        s.ok("level.subdivide", json!({}));
    }
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    s.ok("level.set", json!({ "level": 2 }));
    s.ok("brush.stroke", stroke([0.3, 0.0, 0.95]));
    s
}

/// The top level's mesh, the part left at the level it was at.
fn top(s: &mut Session) -> PlainMesh {
    let d = levels(s);
    s.ok("level.set", json!({ "level": d["count"] }));
    let m = s.mesh("sphere");
    s.ok("history.undo", json!({}));
    m
}

fn to64(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

/// The length of the diagonal of `m`'s bounding box.
fn diagonal(m: &PlainMesh) -> f64 {
    let [lo, hi] = measure::bounds(&m.positions).unwrap_or_default();
    norm(sub(to64(hi), to64(lo)))
}

#[test]
fn freeze_leaves_the_base_and_unfreeze_gives_back_the_exact_part() {
    let mut s = detailed();
    let (before, described) = (s.hash(), levels(&mut s));
    let frozen = s.ok("level.freeze", json!({}));
    assert_eq!(
        (&frozen.result["count"], &frozen.result["active"]),
        (&json!(1), &json!(1))
    );
    assert!(
        frozen.result["carried"].as_u64().is_some_and(|n| n > 0),
        "the pending edit was carried up first"
    );
    let d = levels(&mut s);
    assert_eq!(
        (&d["count"], &d["frozen"], &d["levels"][0]["vertices"]),
        (&json!(1), &json!(3), &described["levels"][0]["vertices"])
    );
    assert_ne!(s.hash(), before, "the scene hash sees the freeze");
    let err = s.err("level.freeze", json!({}));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(err.message.contains("frozen already"), "{}", err.message);
    let back = s.ok("level.unfreeze", json!({}));
    assert_eq!(back.result["how"], json!("restored"));
    assert_eq!(
        s.hash(),
        before,
        "the exact part, the pending edit included"
    );
    assert_eq!(levels(&mut s), described);
    let err = s.err("level.unfreeze", json!({}));
    assert_eq!(err.code, ErrorCode::Precondition);
}

/// Section 4's stated tolerance, under the hybrid rule (decision 0037): every point of the new
/// top is on the old top's surface (within 1e-5 of the diagonal, every point being in reach
/// here at 2.5%), and the old top points are on average within one old top edge length (the mean) of a
/// new top point. The worst case is reported, not promised, but on this sphere, which has
/// nothing narrower than a cell for the remesh to lose, every old top point is within one edge
/// length (the longest). Four levels, as the catalog's Check for `geometry.freeze-levels` asks.
#[test]
fn voxel_remesh_runs_on_a_frozen_part_and_unfreeze_projects_the_top_detail() {
    let mut s = detailed_with(4);
    let old = top(&mut s);
    s.ok("level.freeze", json!({}));
    let remeshed = s.ok("mesh.voxel_remesh", json!({ "resolution": 16 }));
    let faces = remeshed.result["faces"].as_u64().unwrap_or(0);
    assert_eq!(
        levels(&mut s)["frozen"],
        json!(4),
        "the remesh kept the levels"
    );
    // At Draw's strength the default 2% of the diagonal leaves 6 of 4,394 points out of reach
    // (the reach, not the search: decision 0037), so this test reaches a little farther.
    let back = s.ok("level.unfreeze", json!({ "distance": "2.5%" }));
    assert_eq!(back.result["how"], json!("projected"));
    let levels_now = levels(&mut s);
    let count = levels_now["count"].as_u64().unwrap_or(0);
    let top_faces = levels_now["levels"][count as usize - 1]["faces"]
        .as_u64()
        .unwrap_or(0);
    let want = old.faces.len() as u64;
    assert_eq!(
        top_faces,
        faces * 4u64.pow(count as u32 - 1),
        "each level divides the remeshed base"
    );
    // From no division to the level count plus two.
    assert!(
        (0..=6).all(|k| {
            let other = faces * 4u64.pow(k);
            k == count as u32 - 1 || other.abs_diff(want) >= top_faces.abs_diff(want)
        }),
        "{top_faces} faces is the closest to the old top's {want}"
    );
    assert_eq!(levels_now["frozen"], json!(0));
    let new = s.mesh("sphere");
    assert_eq!(
        back.result["projected"].as_u64(),
        Some(new.positions.len() as u64),
        "every point was in reach"
    );
    let diag = diagonal(&old);
    // The new top's points alone, so the farthest sample is the farthest point.
    let points = PlainMesh {
        positions: new.positions.clone(),
        ..PlainMesh::default()
    };
    let on = measure::hausdorff_from(&points, &old);
    assert!(on <= 1e-5 * diag, "a new top point is {on} off the old top");
    let edges: Vec<f64> = (old.faces.iter())
        .flat_map(|f| {
            let n = if f[3] == u32::MAX { 3 } else { 4 };
            (0..n).map(move |k| (f[k], f[(k + 1) % n]))
        })
        .map(|(a, b)| {
            norm(sub(
                to64(old.positions[a as usize]),
                to64(old.positions[b as usize]),
            ))
        })
        .collect();
    let edge = edges.iter().copied().fold(0.0, f64::max);
    let mut total = 0.0;
    for p in old.positions.iter().copied().map(to64) {
        let near = (new.positions.iter().copied().map(to64))
            .map(|q| norm(sub(p, q)))
            .fold(f64::INFINITY, f64::min);
        assert!(near <= edge, "an old top point is {near} from the new top");
        total += near;
    }
    let mean = total / old.positions.len() as f64;
    let mean_edge = edges.iter().sum::<f64>() / edges.len() as f64;
    assert!(
        mean <= mean_edge,
        "old top points are {mean} from the new top on average"
    );
}

/// A remesh as dense as the old top or denser is not divided: it is the top itself, a part of
/// one level, projected (review 0007, m3: it was divided once all the same, so resolution 20
/// came back with 7,056 faces for an old top of 1,536, and resolution 96 with 165,232).
#[test]
fn a_remesh_denser_than_the_old_top_comes_back_undivided() {
    for (resolution, near) in [(20, true), (96, false)] {
        let mut s = detailed();
        let want = top(&mut s).faces.len() as u64;
        s.ok("level.freeze", json!({}));
        let remeshed = s.ok("mesh.voxel_remesh", json!({ "resolution": resolution }));
        let faces = remeshed.result["faces"].as_u64().unwrap_or(0);
        assert!(faces > want, "resolution {resolution}: {faces} faces");
        let back = s.ok("level.unfreeze", json!({})).result;
        assert_eq!(back["how"], json!("projected"));
        let d = levels(&mut s);
        assert_eq!(
            (&d["count"], &d["levels"][0]["faces"], &d["frozen"]),
            (&json!(1), &json!(faces), &json!(0)),
            "resolution {resolution}: the remesh is the top"
        );
        let points = s.mesh("sphere").positions.len() as u64;
        assert_eq!(back["projected"].as_u64(), Some(points), "all in reach");
        if near {
            assert!(faces < want + want / 4, "{faces} faces, near {want}");
        }
    }
}

/// Paint finer than the base comes back with the detail (research 24, wrong result 7): a cube
/// of three levels whose top alone holds a red band and a mask band, each narrower than a base
/// face and clear of every base point, is frozen, remeshed and unfrozen. Every new top point
/// more than one old top edge inside a band reads the band's value, and every one more than an
/// edge outside it reads the unpainted value, as the old top does at the same place.
#[test]
fn unfreeze_onto_a_remeshed_base_brings_back_paint_finer_than_the_base() {
    const RED: u32 = 0xff00_00ff;
    // Base points lie at -1, -0.5, 0, 0.5 and 1 on each axis; these bands are 0.05 to 0.45.
    let off = |c: f32| (c - 0.25).abs() - 0.2;
    let mut s = agent();
    s.add_part("cube", &fixtures::cube(4));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    let mut painted = s.mesh("cube");
    let red: Vec<u32> = (painted.positions.iter())
        .map(|p| if off(p[1]) < 0.0 { RED } else { u32::MAX })
        .collect();
    let mask: Vec<f32> = (painted.positions.iter())
        .map(|p| if off(p[0]) < 0.0 { 1.0 } else { 0.0 })
        .collect();
    for (name, values) in [
        ("color", AttrValues::U32(red)),
        ("mask", AttrValues::F32(mask)),
    ] {
        let domain = AttrDomain::Vertex;
        painted
            .attributes
            .insert(name.into(), Attribute { domain, values });
    }
    let part = json!({ "part": "painted" });
    s.add_part("painted", &painted);
    s.ok("level.reconstruct", part.clone());
    s.ok("level.reconstruct", part.clone());
    let d = s.query("level.describe", part.clone());
    assert_eq!((&d["count"], &d["active"]), (&json!(3), &json!(3)));
    let old = s.mesh("painted");
    assert_eq!(json!(old), json!(painted), "the top holds the paint");
    s.ok("level.freeze", part.clone());
    s.ok(
        "mesh.voxel_remesh",
        json!({ "part": "painted", "resolution": 16 }),
    );
    let back = s.ok(
        "level.unfreeze",
        json!({ "part": "painted", "distance": "5%" }),
    );
    assert_eq!(back.result["how"], json!("projected"));
    let d = s.query("level.describe", part);
    assert_eq!(d["active"], d["count"], "the new top is active");
    let new = s.mesh("painted");
    assert_eq!(
        back.result["projected"].as_u64(),
        Some(new.positions.len() as u64),
        "every point was in reach"
    );
    let edge = (old.faces.iter())
        .flat_map(|f| (0..4).map(move |k| (f[k], f[(k + 1) % 4])))
        .map(|(a, b)| {
            let (p, q) = (old.positions[a as usize], old.positions[b as usize]);
            norm(sub(to64(p), to64(q))) as f32
        })
        .fold(0.0, f32::max);
    let column = |name: &str| match new.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&c| f64::from(c)).collect(),
        Some(AttrValues::F32(v)) => v.iter().map(|&m| f64::from(m)).collect(),
        None => vec![0.0; new.positions.len()],
    };
    let (colors, masks) = (column("color"), column("mask"));
    let mut wrong = Vec::new();
    for (what, axis, values, inside, outside) in [
        ("color", 1, &colors, f64::from(RED), f64::from(u32::MAX)),
        ("mask", 0, &masks, 1.0, 0.0),
    ] {
        let into = |p: &[f32; 3]| -off(p[axis]);
        let [ins, outs, bad] = band_reads(&new.positions, values, into, edge, [inside, outside]);
        if bad > 0 || ins < 20 || outs < 20 {
            wrong.push(format!(
                "{what}: {ins} right inside, {outs} right outside, {bad} wrong"
            ));
        }
    }
    assert!(wrong.is_empty(), "edge {edge}: {wrong:?}");
}

/// A band read on `values`: the points more than `edge` inside it (`into` above `edge`) that
/// read `want[0]`, those more than `edge` outside it that read `want[1]`, and those of either
/// that read anything else.
fn band_reads(
    positions: &[[f32; 3]],
    values: &[f64],
    into: impl Fn(&[f32; 3]) -> f32,
    edge: f32,
    want: [f64; 2],
) -> [usize; 3] {
    let mut counts = [0; 3];
    for (p, &v) in positions.iter().zip(values) {
        let d = into(p);
        let side = if d > edge {
            0
        } else if d < -edge {
            1
        } else {
            continue;
        };
        let right = (v - want[side]).abs() <= 1e-6;
        counts[if right { side } else { 2 }] += 1;
    }
    counts
}

/// A distance of 0 could move no point, so projecting at it would drop the frozen detail
/// (review 0007, S1): refused, whichever way the levels would come back.
#[test]
fn a_distance_of_0_is_refused_and_changes_nothing() {
    let mut s = detailed();
    s.ok("level.freeze", json!({}));
    for remesh in [false, true] {
        if remesh {
            s.ok("mesh.voxel_remesh", json!({ "resolution": 16 }));
        }
        for distance in [json!(0.0), json!("0%"), json!(-1.0)] {
            let before = s.hash();
            let err = s.err("level.unfreeze", json!({ "distance": distance }));
            assert_eq!(err.code, ErrorCode::ArgsInvalid, "{distance}");
            assert!(err.message.contains("more than 0"), "{}", err.message);
            assert_eq!(s.hash(), before, "{distance}: nothing changed");
        }
    }
}

#[test]
fn undo_and_redo_cross_freeze_remesh_and_unfreeze_exactly() {
    let mut s = detailed();
    let mut hashes = vec![s.hash()];
    for (command, args) in [
        ("level.freeze", json!({})),
        ("mesh.voxel_remesh", json!({ "resolution": 16 })),
        ("level.unfreeze", json!({})),
    ] {
        s.ok(command, args);
        hashes.push(s.hash());
    }
    for i in (0..hashes.len() - 1).rev() {
        s.ok("history.undo", json!({}));
        assert_eq!(s.hash(), hashes[i], "undo back to step {i}");
    }
    for (i, want) in hashes.iter().enumerate().skip(1) {
        s.ok("history.redo", json!({}));
        assert_eq!(&s.hash(), want, "redo to step {i}");
    }
}

#[test]
fn a_stroke_on_the_frozen_base_comes_back_as_level_1_edits() {
    let base_stroke = stroke([0.0, 0.3, 0.95]);
    let mut frozen = detailed();
    frozen.ok("level.freeze", json!({}));
    frozen.ok("brush.stroke", base_stroke.clone());
    let back = frozen.ok("level.unfreeze", json!({}));
    assert_eq!(back.result["how"], json!("carried"));
    let d = levels(&mut frozen);
    assert_eq!(
        (&d["count"], &d["active"], &d["pending"]),
        (&json!(3), &json!(1), &json!(true))
    );
    let mut plain = detailed();
    plain.ok("level.set", json!({ "level": 1 }));
    plain.ok("brush.stroke", base_stroke);
    assert_eq!(
        frozen.hash(),
        plain.hash(),
        "the same as the stroke made on level 1"
    );
    for s in [&mut frozen, &mut plain] {
        s.ok("level.set", json!({ "level": 3 }));
    }
    assert_eq!(frozen.hash(), plain.hash(), "and carried up the same");
}

/// Review 0008, M1: `part.apply_transform` on a frozen part writes its transform into the
/// frozen levels too, so unfreeze still finds the base as freeze left it and restores the
/// levels, at the same level and in the same place as on the part left placed. Before, a scale
/// came back carried, at level 1, with the detail at its old size.
#[test]
fn a_frozen_part_baked_unfreezes_as_the_part_left_placed() {
    let about_origin =
        |op: Value| json!({ "frame": { "origin": [0, 0, 0], "axes": [0, 0, 0, 1] }, "op": op });
    let turn = about_origin(json!({ "rotate": { "axis": [1.0, 2.0, 3.0], "degrees": 35 } }));
    let shift = about_origin(json!({ "move": [0.3, -0.2, 0.7] }));
    let grow = about_origin(json!({ "scale": [2.0, 2.0, 2.0] }));
    let cases = [
        ("scale", vec![grow.clone()]),
        ("turn, move, scale", vec![turn.clone(), shift.clone(), grow]),
        ("move", vec![shift]),
        ("turn", vec![turn]),
    ];
    for (what, calls) in cases {
        let run = |bake: bool| {
            let mut s = detailed();
            s.ok("level.freeze", json!({}));
            for args in &calls {
                s.ok("transform.apply", args.clone());
            }
            if bake {
                s.ok("part.apply_transform", json!({}));
            }
            let back = s.ok("level.unfreeze", json!({})).result;
            let active = back["levels"]["active"].clone();
            (back["how"].clone(), active, top(&mut s).positions)
        };
        let (placed, baked) = (run(false), run(true));
        assert_eq!(placed.0, json!("restored"), "{what}: left placed");
        assert_eq!(baked.0, json!("restored"), "{what}: baked");
        assert_eq!(baked.1, placed.1, "{what}: the same active level");
        let apart = worst(&placed.2, &baked.2);
        assert!(apart < 1e-5, "{what}: the top levels are {apart} apart");
    }
}

/// Review 0008, m1: an unfreeze distance in scene units is divided by the part's scale, so a
/// part scaled by 2 reaches as many points at a distance as the same part baked. Before, it
/// reached twice as far: placed at 0.005 projected as many points as baked at 0.01.
#[test]
fn an_unfreeze_distance_in_units_reaches_as_far_on_a_scaled_part_as_on_it_baked() {
    let grow = json!({ "frame": { "origin": [0, 0, 0], "axes": [0, 0, 0, 1] },
                       "op": { "scale": [2.0, 2.0, 2.0] } });
    let run = |bake: bool| {
        let mut s = detailed_with(2);
        s.ok("transform.apply", grow.clone());
        if bake {
            s.ok("part.apply_transform", json!({}));
        }
        s.ok("level.freeze", json!({}));
        s.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
        // 143 of 2462 points within 0.02 (none within 0.01 since decision 0036's spacer).
        let back = s.ok("level.unfreeze", json!({ "distance": 0.02 })).result;
        let projected = back["projected"].as_u64().unwrap_or(u64::MAX);
        (projected, top(&mut s).positions)
    };
    let (placed, baked) = (run(false), run(true));
    let points = baked.1.len() as u64;
    assert!(
        0 < baked.0 && baked.0 < points,
        "{} of {points} points in reach, so the distance decides",
        baked.0
    );
    assert_eq!(placed.0, baked.0, "as many points in reach");
    let apart = worst(&placed.1, &baked.1);
    assert!(apart < 1e-5, "the top levels are {apart} apart");
}

/// A mask set or cleared on the frozen base gains or drops a point column, and unfreeze carries
/// it too, moving no point (review 0007, M2: it projected, and every level moved). A first
/// mask comes back as level 1's pending edit, carried up as a mask stroke on level 1 would be;
/// `mask.set` on level 1 runs on the top instead (review 0004, M2), so the two agree on what
/// level 1 shows and, once carried, on everything. A mask cleared everywhere is released on
/// every level, as `mask.set` with 0 on level 1 does, so nothing is left pending.
#[test]
fn a_mask_set_or_cleared_on_the_frozen_base_comes_back_without_moving_a_point() {
    let mask = |v: f32| json!({ "value": v });
    let part = json!({ "part": "sphere" });
    // (the mask before freezing, the mask set on the frozen base)
    for (before, value) in [(None, 1.0), (Some(1.0), 0.0)] {
        let (mut frozen, mut plain) = (detailed(), detailed());
        for s in [&mut frozen, &mut plain] {
            if let Some(v) = before {
                s.ok("mask.set", mask(v));
            }
        }
        frozen.ok("level.freeze", json!({}));
        frozen.ok("mask.set", mask(value));
        let back = frozen.ok("level.unfreeze", json!({}));
        assert_eq!(back.result["how"], json!("carried"), "mask {value}");
        plain.ok("level.set", json!({ "level": 1 }));
        plain.ok("mask.set", mask(value));
        assert_eq!(
            frozen.query("mesh.hash", part.clone()),
            plain.query("mesh.hash", part.clone()),
            "mask {value}: what level 1 shows"
        );
        let gained = before.is_none();
        assert_eq!(
            levels(&mut frozen)["pending"],
            json!(gained),
            "mask {value}"
        );
        if !gained {
            assert_eq!(
                frozen.hash(),
                plain.hash(),
                "mask {value}: released everywhere"
            );
        }
        for s in [&mut frozen, &mut plain] {
            s.ok("level.set", json!({ "level": 3 }));
        }
        assert_eq!(
            frozen.hash(),
            plain.hash(),
            "mask {value}: carried up the same"
        );
    }
}

#[test]
fn what_would_drop_frozen_levels_refuses_and_names_unfreeze() {
    let mut s = detailed();
    s.ok("level.freeze", json!({}));
    for (command, args) in [
        ("level.subdivide", json!({})),
        ("level.set", json!({ "level": 1 })),
        ("level.step", json!({ "by": 1 })),
        ("level.delete_higher", json!({})),
        ("level.delete_lower", json!({})),
        ("level.reconstruct", json!({})),
        ("part.split", json!({ "part": "sphere", "by": "shells" })),
    ] {
        let before = s.hash();
        let err = s.err(command, args);
        assert_eq!(err.code, ErrorCode::Precondition, "{command}");
        assert!(
            err.hint
                .as_deref()
                .is_some_and(|h| h.contains("level.unfreeze")),
            "{command}: {err:?}"
        );
        assert_eq!(s.hash(), before, "{command} changed nothing");
    }
    let mut ask = stroke([0.0, 0.0, 1.0]);
    ask["live_detail"] = json!({});
    let done = s.query("brush.stroke", ask);
    assert!(
        text(&done["live_detail"]["why_not"]).contains("level.unfreeze"),
        "{done}"
    );
    s.ok("part.duplicate", json!({ "part": "sphere" }));
    let copy = s.query("level.describe", json!({ "part": "sphere-2" }));
    assert_eq!(
        copy["frozen"],
        json!(3),
        "a duplicate shares the frozen levels"
    );
}

/// A dry run asks each command's `check`, which the window reads to draw a control disabled
/// with why: freeze, unfreeze and the level commands give the reason and the fix a real call
/// would.
#[test]
fn a_dry_run_says_why_a_level_command_cannot_run() {
    let dry = |s: &mut Session, command: &str, args: Value| {
        let params = RunParams::new(command, args).dry();
        failure(s.request(Request::Run(params)), "a dry run")
    };
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let err = dry(&mut s, "level.freeze", json!({}));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert_eq!(
        (err.message.as_str(), err.hint.as_deref()),
        (
            "part 'sphere' has one level: there are no levels to freeze",
            Some("level.subdivide adds levels")
        )
    );
    let err = dry(&mut s, "level.unfreeze", json!({}));
    assert!(err.message.contains("is not frozen"), "{err:?}");
    let mut s = detailed();
    s.ok("level.freeze", json!({}));
    let err = dry(&mut s, "level.step", json!({ "by": -1 }));
    assert!(err.message.contains("frozen detail levels"), "{err:?}");
}
