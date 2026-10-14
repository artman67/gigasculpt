//! The face group commands against the catalog's checks (decision 0026), through commands the
//! way an agent calls them: what `group.assign`, `group.merge` and `group.make` give and leave
//! alone, on detail levels too, and `group.list`'s colors from one session to the next. Exact
//! undo of every group edit is the claims harness's (`claims`), which runs each example.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::fixtures;
use serde_json::{Value, json};

use crate::support::agent;

/// Column `name` of the exported mesh, one number per face or point (ids exact), or `len`
/// zeros when the mesh has none.
fn column(m: &PlainMesh, name: &str, len: usize) -> Vec<f64> {
    match m.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&x| f64::from(x)).collect(),
        Some(AttrValues::F32(v)) => v.iter().map(|&x| f64::from(x)).collect(),
        _ => vec![0.0; len],
    }
}

/// A face column as integers.
pub(crate) fn ids(m: &PlainMesh, name: &str) -> Vec<u32> {
    (column(m, name, m.faces.len()).into_iter())
        .map(|x| x as u32)
        .collect()
}

pub(crate) fn groups(m: &PlainMesh) -> Vec<u32> {
    ids(m, "group")
}

pub(crate) fn mask(m: &PlainMesh) -> Vec<f64> {
    column(m, "mask", m.positions.len())
}

/// Each face's corners.
pub(crate) fn corners(m: &PlainMesh, f: usize) -> Vec<usize> {
    (m.faces[f].iter())
        .filter(|&&v| v != u32::MAX)
        .map(|&v| v as usize)
        .collect()
}

/// Each face's center and unit normal (from its diagonals, or its edges for a triangle).
fn frames(m: &PlainMesh) -> Vec<([f32; 3], [f32; 3])> {
    let p = |v: usize| m.positions[v];
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (0..m.faces.len())
        .map(|f| {
            let c = corners(m, f);
            let k = c.len() as f32;
            let center = (0..3).map(|i| c.iter().map(|&v| p(v)[i]).sum::<f32>() / k);
            let center: Vec<f32> = center.collect();
            let (a, b) = if c.len() == 4 {
                (sub(p(c[2]), p(c[0])), sub(p(c[3]), p(c[1])))
            } else {
                (sub(p(c[1]), p(c[0])), sub(p(c[2]), p(c[0])))
            };
            let n = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            ([center[0], center[1], center[2]], n.map(|x| x / len))
        })
        .collect()
}

/// How many edge-connected pieces the faces `keep` picks form.
fn pieces(m: &PlainMesh, keep: impl Fn(usize) -> bool) -> usize {
    let mut by_edge: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for f in (0..m.faces.len()).filter(|&f| keep(f)) {
        let c = corners(m, f);
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(f);
        }
    }
    let mut piece: BTreeMap<usize, usize> = BTreeMap::new();
    let mut count = 0;
    for start in (0..m.faces.len()).filter(|&f| keep(f)) {
        if piece.contains_key(&start) {
            continue;
        }
        count += 1;
        piece.insert(start, count);
        let mut stack = vec![start];
        while let Some(f) = stack.pop() {
            let c = corners(m, f);
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                for &g in &by_edge[&(a.min(b), a.max(b))] {
                    if piece.insert(g, count).is_none() {
                        stack.push(g);
                    }
                }
            }
        }
    }
    count
}

fn make(maker: &Value) -> Value {
    json!({ "maker": maker })
}

#[test]
fn assign_gives_exactly_the_chosen_faces_and_merge_folds_groups_in() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    s.ok("group.make", make(&json!({ "by": "normals" })));
    let mut want = groups(&s.mesh("cube"));
    let mut sorted = want.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6], "one group per side");
    let assigned = s.ok(
        "group.assign",
        json!({ "select": { "by": "faces", "indexes": [0, 2] }, "id": 7 }),
    );
    assert_eq!(assigned.result["ids"], json!([7, 7]));
    (want[0], want[2]) = (7, 7);
    assert_eq!(groups(&s.mesh("cube")), want, "faces 0 and 2, no other");
    // A new id is one above the largest any face holds.
    let fresh = s.ok(
        "group.assign",
        json!({ "select": { "by": "faces", "indexes": [5] }, "id": "new" }),
    );
    assert_eq!(fresh.result["ids"], json!([8, 8]));
    want[5] = 8;
    s.ok("group.merge", json!({ "sources": [2, 4], "target": 5 }));
    let merged: Vec<u32> = (want.iter())
        .map(|&g| if g == 2 || g == 4 { 5 } else { g })
        .collect();
    assert_eq!(groups(&s.mesh("cube")), merged);
}

/// Group ids stop at 2147483647, the largest Blender's face sets hold (review 0007, m1: an id
/// of `u32::MAX` was taken, and every later new group panicked on the overflow, or in a build
/// without overflow checks wrapped to 0, no group). A larger id is refused where it comes in;
/// once no id is left, a new group answers `E_LIMIT` and changes nothing (`err` checks the
/// hash), while joining the largest group still works.
#[test]
fn group_ids_stop_at_the_ceiling_and_a_new_group_past_it_answers_e_limit() {
    const CEILING: u32 = i32::MAX as u32;
    let faces =
        |ix: u32, id: Value| json!({ "select": { "by": "faces", "indexes": [ix] }, "id": id });
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    for (command, args, path) in [
        ("group.assign", faces(0, json!(u32::MAX)), "/id"),
        ("group.assign", faces(0, json!(CEILING + 1)), "/id"),
        (
            "group.merge",
            json!({ "sources": [0], "target": u32::MAX }),
            "/target",
        ),
    ] {
        let err = s.err(command, args);
        assert_eq!(
            (err.code, err.path.as_deref()),
            (ErrorCode::ArgsInvalid, Some(path)),
            "{command}: {err:?}"
        );
    }
    s.ok("group.assign", faces(0, json!(CEILING - 1)));
    let sides = s.err("group.make", make(&json!({ "by": "normals" })));
    assert_eq!(sides.code, ErrorCode::Limit, "six sides, one id left");
    assert!(sides.message.contains("6 new groups"), "{}", sides.message);
    let last = s.ok("group.assign", faces(1, json!("new")));
    assert_eq!(last.result["ids"], json!([CEILING, CEILING]));
    let listed = s.query("group.list", json!({}));
    assert_eq!(
        (&listed["largest"], &listed["next"]),
        (&json!(CEILING), &Value::Null),
        "no next id"
    );
    for (command, args) in [
        ("group.assign", faces(2, json!("new"))),
        ("group.make", make(&json!({ "by": "shells" }))),
        ("group.make", make(&json!({ "by": "visible" }))),
    ] {
        let err = s.err(command, args);
        assert_eq!(err.code, ErrorCode::Limit, "{command}: {err:?}");
        assert!(err.message.contains("no id is left"), "{}", err.message);
    }
    let joined = s.ok(
        "group.make",
        json!({ "maker": { "by": "front", "additive": true }, "camera": "front" }),
    );
    assert_eq!(
        joined.result["ids"],
        json!([CEILING, CEILING]),
        "joins the largest"
    );
    // A mesh holding a larger id is refused as it comes in, as a file's or an import's would be.
    let mut held = fixtures::cube(1);
    let values = AttrValues::U32(vec![u32::MAX; held.faces.len()]);
    let domain = AttrDomain::Face;
    held.attributes
        .insert("group".into(), Attribute { domain, values });
    let err = s.err("part.create", json!({ "name": "held", "mesh": held }));
    assert!(err.message.contains("2147483647"), "{err:?}");
}

#[test]
fn a_group_on_a_detail_level_holds_when_switching_and_covers_the_faces_below() {
    let leveled = || {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        s.ok("level.subdivide", json!({}));
        s.ok("level.subdivide", json!({}));
        s
    };
    let pick = json!({ "select": { "by": "faces", "indexes": [0, 1, 2, 40, 900] }, "id": 4 });
    let mut s = leveled();
    s.ok("group.assign", pick);
    let written = groups(&s.mesh("sphere"));
    assert_eq!(written.iter().filter(|&&g| g == 4).count(), 5);
    s.ok("level.set", json!({ "level": 1 }));
    s.ok("level.set", json!({ "level": 3 }));
    assert_eq!(
        groups(&s.mesh("sphere")),
        written,
        "level 3's group after 1 and back"
    );
    // Written on level 1, a group covers the 16 top faces each level 1 face became.
    let mut s = leveled();
    s.ok("level.set", json!({ "level": 1 }));
    let one = json!({ "select": { "by": "faces", "indexes": [5, 17] }, "id": 9 });
    s.ok("group.assign", one);
    s.ok("level.set", json!({ "level": 3 }));
    let top = groups(&s.mesh("sphere"));
    assert_eq!(
        top.iter().filter(|&&g| g == 9).count(),
        32,
        "16 top faces each"
    );
    assert!(
        top.iter().all(|&g| g == 9 || g == 0),
        "and no other face changed"
    );
}

/// Two copies of `m` as one mesh, the second moved 4 along x: two separate pieces, the first's
/// faces first.
pub(crate) fn twice(m: &PlainMesh) -> PlainMesh {
    let mut two = m.clone();
    let n = m.positions.len() as u32;
    (two.positions).extend(m.positions.iter().map(|p| [p[0] + 4.0, p[1], p[2]]));
    two.faces
        .extend((m.faces.iter()).map(|f| f.map(|v| if v == u32::MAX { v } else { v + n })));
    two
}

#[test]
fn two_separate_cubes_make_two_groups_and_a_cube_six_whatever_the_angle() {
    let cube = fixtures::cube(3);
    let two = twice(&cube);
    let mut s = agent();
    s.ok("part.create", json!({ "name": "two", "mesh": two }));
    let made = s.ok("group.make", make(&json!({ "by": "shells" })));
    assert_eq!(made.result["ids"], json!([1, 2]));
    assert_eq!(made.result["groups"], json!(2));
    let g = groups(&s.mesh("two"));
    let half = cube.faces.len();
    assert!(g[..half].iter().all(|&x| x == g[0]) && g[half..].iter().all(|&x| x == g[half]));
    assert_ne!(g[0], g[half], "one group per cube");
    for angle in [1, 30, 60, 89] {
        let mut s = agent();
        s.ok("part.create", json!({ "name": "cube", "mesh": cube }));
        let made = s.ok(
            "group.make",
            make(&json!({ "by": "normals", "max_angle": angle })),
        );
        assert_eq!(
            made.result["ids"],
            json!([1, 6]),
            "a cube at {angle} degrees"
        );
    }
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let made = s.ok("group.make", make(&json!({ "by": "normals" })));
    assert_eq!(made.result["ids"], json!([1, 1]), "a sphere is one group");
}

#[test]
fn group_visible_covers_its_share_the_same_way_for_a_seed_in_larger_patches_as_cluster_rises() {
    let hidden_half = || {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
        let facing = json!({ "select": { "by": "facing" }, "camera": "front" });
        s.ok("visibility.hide", facing);
        s
    };
    let mut s = hidden_half();
    let hidden = ids(&s.mesh("sphere"), "hide");
    let visible = hidden.iter().filter(|&&h| h == 0).count();
    assert!(
        visible > 100 && visible < hidden.len() - 100,
        "about half hidden"
    );
    s.ok("group.make", make(&json!({ "by": "visible" })));
    let all = groups(&s.mesh("sphere"));
    for (f, (&g, &h)) in all.iter().zip(&hidden).enumerate() {
        assert_eq!(
            g,
            u32::from(h == 0),
            "face {f}: every visible face, no hidden one"
        );
    }
    let picked = |s: &mut gs_testkit::Session, args: Value| {
        s.ok("group.make", make(&args));
        let g = groups(&s.mesh("sphere"));
        s.ok("history.undo", json!({}));
        g
    };
    let mut s = hidden_half();
    let half = json!({ "by": "visible", "coverage": 0.5, "seed": 3 });
    let a = picked(&mut s, half.clone());
    let count = a.iter().filter(|&&g| g == 1).count();
    assert_eq!(
        count,
        (visible as f64 * 0.5).round() as usize,
        "half the visible faces"
    );
    assert!(
        a.iter().zip(&hidden).all(|(&g, &h)| h == 0 || g == 0),
        "none hidden"
    );
    assert_eq!(picked(&mut s, half), a, "the same seed, the same faces");
    let other = json!({ "by": "visible", "coverage": 0.5, "seed": 4 });
    assert_ne!(picked(&mut s, other), a, "another seed, other faces");
    let mesh = s.mesh("sphere");
    let patches = |s: &mut gs_testkit::Session, cluster: f64| {
        let g = picked(
            s,
            json!({ "by": "visible", "coverage": 0.3, "cluster": cluster }),
        );
        pieces(&mesh, |f| g[f] == 1)
    };
    let (loose, tight) = (patches(&mut s, 0.0), patches(&mut s, 1.0));
    assert!(
        tight * 3 < loose,
        "{tight} patches at cluster 1, {loose} at cluster 0"
    );
}

#[test]
fn group_front_takes_faces_turned_to_the_camera_inner_ones_too_and_adds_with_additive() {
    let torus = || {
        let mut s = agent();
        s.ok(
            "part.create",
            json!({ "name": "torus", "mesh": fixtures::torus() }),
        );
        s
    };
    let front = |angle: u32, additive: bool, camera: &str| {
        let maker = json!({ "by": "front", "angle": angle, "additive": additive });
        json!({ "maker": maker, "camera": camera })
    };
    let mut s = torus();
    let mesh = s.mesh("torus");
    let at = frames(&mesh);
    let made = s.ok("group.make", front(90, false, "front"));
    let id = made.result["ids"][0].as_u64().map(|i| i as u32);
    assert_eq!(id, Some(1));
    let g = groups(&s.mesh("torus"));
    let mut inner = 0;
    for (f, &(c, n)) in at.iter().enumerate() {
        // The camera is in front, at +z: well-turned faces are clear either way.
        if n[2] > 0.5 {
            assert_eq!(g[f], 1, "face {f} turned to the camera");
            inner += usize::from(c[2] < -0.5);
        } else if n[2] < -0.5 {
            assert_eq!(g[f], 0, "face {f} turned away keeps its group");
        }
    }
    assert!(
        inner > 0,
        "faces behind the front that face the camera count too"
    );
    // Additive: a second call from the right adds to the same group.
    let again = s.ok("group.make", front(90, true, "right"));
    assert_eq!(again.result["ids"], json!([1, 1]));
    let g = groups(&s.mesh("torus"));
    for (f, &(_, n)) in at.iter().enumerate() {
        if n[0] > 0.5 || n[2] > 0.5 {
            assert_eq!(g[f], 1, "face {f} after the additive call");
        }
    }
    let count = |angle| {
        let mut s = torus();
        s.ok("group.make", front(angle, false, "front"));
        groups(&s.mesh("torus")).iter().filter(|&&g| g == 1).count()
    };
    assert!(
        count(30) < count(90),
        "30 degrees takes fewer faces than 90"
    );
}

#[test]
fn group_masked_groups_the_masked_faces_and_clear_mask_undoes_in_one_step() {
    let painted = || {
        let mut s = agent();
        s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
        let samples = [[0.0, 0.0, 1.0], [0.2, 0.1, 0.98]].map(|p| json!({ "position": p }));
        let pen = json!({ "brush": "Mask Pen", "size": "20%", "samples": samples });
        s.ok("brush.stroke", pen);
        s
    };
    let masked_faces = |m: &PlainMesh| -> Vec<bool> {
        let k = mask(m);
        (0..m.faces.len())
            .map(|f| {
                let c = corners(m, f);
                c.iter().map(|&v| k[v]).sum::<f64>() / c.len() as f64 >= 0.5
            })
            .collect()
    };
    let mut s = painted();
    let before = s.mesh("sphere");
    let want = masked_faces(&before);
    assert!(want.iter().filter(|&&m| m).count() > 10, "a painted patch");
    let made = s.ok("group.make", make(&json!({ "by": "masked" })));
    assert_eq!(made.result["ids"], json!([1, 1]));
    let after = s.mesh("sphere");
    let g = groups(&after);
    assert!(
        g.iter().zip(&want).all(|(&g, &m)| g == u32::from(m)),
        "the masked faces"
    );
    assert_eq!(mask(&after), mask(&before), "the mask is unchanged");
    // With clear_mask, no point is masked after, and one undo brings both back.
    let mut s = painted();
    let start = s.hash();
    s.ok(
        "group.make",
        make(&json!({ "by": "masked", "clear_mask": true })),
    );
    let after = s.mesh("sphere");
    assert_eq!(groups(&after), g, "the same group");
    assert!(mask(&after).iter().all(|&k| k == 0.0), "no point masked");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), start, "one undo restores groups and mask");
    // With no mask every visible face gets the new group.
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok(
        "visibility.hide",
        json!({ "select": { "by": "faces", "indexes": [0, 1] } }),
    );
    s.ok("group.make", make(&json!({ "by": "masked" })));
    let g = groups(&s.mesh("sphere"));
    assert_eq!(&g[..3], &[0, 0, 1], "hidden faces keep theirs");
    assert!(g[2..].iter().all(|&x| x == 1), "every visible face");
}

#[test]
fn group_colors_are_the_same_in_every_session() {
    let listed = || {
        let mut s = agent();
        s.ok("primitive.cube", json!({}));
        s.ok("group.make", make(&json!({ "by": "normals" })));
        s.query("group.list", json!({}))
    };
    let a = listed();
    assert_eq!(a, listed(), "two sessions list the same");
    assert_eq!(
        a["groups"][0]["color"],
        json!("#3c5daf"),
        "group 1's color, as Blender draws it"
    );
    assert_eq!(
        (a["largest"].clone(), a["next"].clone()),
        (json!(6), json!(7))
    );
}
