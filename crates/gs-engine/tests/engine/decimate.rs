//! `mesh.decimate` through the engine (decision 0034): the faces it reaches, what it keeps
//! exactly, symmetry, and its refusals. gs-geo tests the pieces underneath.

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};
use serde_json::json;

use gs_testkit::{Session, code};

fn sphere(n: u32) -> Session {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": n }));
    s
}

fn points(f: &[u32; 4]) -> Vec<u32> {
    f.iter().copied().filter(|&v| v != NO_VERTEX).collect()
}

fn centroid(mesh: &PlainMesh, f: &[u32; 4]) -> [f32; 3] {
    let c = points(f);
    let mut sum = [0.0; 3];
    for &v in &c {
        for (total, x) in sum.iter_mut().zip(mesh.positions[v as usize]) {
            *total += x / c.len() as f32;
        }
    }
    sum
}

fn radius(p: [f32; 3]) -> f32 {
    (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
}

fn column<'a>(mesh: &'a PlainMesh, name: &str) -> &'a AttrValues {
    &mesh.attributes[name].values
}

fn f32s(values: &AttrValues) -> &[f32] {
    match values {
        AttrValues::F32(v) => v,
        AttrValues::U32(_) => panic!("not an f32 column"),
    }
}

#[test]
fn a_million_face_sphere_at_ten_percent_keeps_a_tenth_close_by_and_the_same_every_time() {
    let mut s = sphere(204);
    s.ok("level.subdivide", json!({}));
    s.ok("level.delete_lower", json!({}));
    let before = s.mesh("sphere");
    assert_eq!(before.faces.len(), 998_784);
    let (low, high) = (before.positions.iter().copied().map(radius))
        .fold((f32::MAX, 0.0_f32), |(lo, hi), r| (lo.min(r), hi.max(r)));
    let start = s.hash();
    let reply = s.query("mesh.decimate", json!({ "percent": 10 }));
    let first = s.hash();
    let mesh = s.mesh("sphere");
    let faces = mesh.faces.len() as f64;
    assert!(
        (faces - 99_878.0).abs() <= 99.878,
        "{faces} faces for 99,878"
    );
    assert_eq!(reply["target"], json!(99_878), "{reply}");
    assert_eq!(reply["reached"], json!(true), "{reply}");
    // Every point is the sphere's own, so a face strays from it only by its sag.
    let worst = (mesh.faces.iter())
        .map(|f| low - radius(centroid(&mesh, f)))
        .fold(0.0_f32, f32::max);
    assert!(worst < 1e-3, "a face sags {worst} inside the sphere");
    assert!(
        mesh.positions
            .iter()
            .all(|&p| (low..=high).contains(&radius(p)))
    );
    let error = reply["error"].as_f64().unwrap_or(f64::MAX);
    assert!(error < 1e-2, "error {error}");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), start, "one undo step back to the sphere");
    s.ok("mesh.decimate", json!({ "percent": 10 }));
    assert_eq!(s.hash(), first, "the same mesh a second time");
}

/// The quadrant of a point across x = 0 and y = 0, as a group.
fn quadrant(p: [f32; 3]) -> u32 {
    u32::from(p[0] > 0.0) + 2 * u32::from(p[1] > 0.0)
}

/// The points where faces of two groups meet, by position.
fn group_borders(mesh: &PlainMesh) -> BTreeSet<[u32; 3]> {
    let AttrValues::U32(groups) = column(mesh, "group") else {
        panic!("no group column");
    };
    let mut seen: Vec<Option<u32>> = vec![None; mesh.positions.len()];
    let mut border = BTreeSet::new();
    for (f, &g) in mesh.faces.iter().zip(groups) {
        for v in points(f) {
            if *seen[v as usize].get_or_insert(g) != g {
                border.insert(mesh.positions[v as usize].map(f32::to_bits));
            }
        }
    }
    border
}

/// Each corner's UV is its point's x and y, shifted by 10 in u below the seam, and a face's
/// corners all lie on one side of it.
fn uvs_follow_points(mesh: &PlainMesh, what: &str) {
    for (j, f) in mesh.faces.iter().enumerate() {
        let shifts: BTreeSet<u32> = (0..points(f).len())
            .map(|k| {
                let p = mesh.positions[f[k] as usize];
                let (u, v) = (
                    f32s(column(mesh, &format!("uv{k}.u")))[j],
                    f32s(column(mesh, &format!("uv{k}.v")))[j],
                );
                assert_eq!(v, p[1], "{what}: face {j} corner {k}");
                let shift = [p[0], p[0] + 10.0].map(f32::to_bits);
                assert!(shift.contains(&u.to_bits()), "{what}: face {j} corner {k}");
                u32::from(u.to_bits() != shift[0])
            })
            .collect();
        assert_eq!(
            shifts.len(),
            1,
            "{what}: face {j} lies on one side of the seam"
        );
    }
}

#[test]
fn group_borders_and_uvs_come_through_exactly() {
    let mut s = sphere(16);
    let mut mesh = s.mesh("sphere");
    let nf = mesh.faces.len();
    let groups = (mesh.faces.iter())
        .map(|f| quadrant(centroid(&mesh, f)))
        .collect();
    // UVs follow the points, with a seam round z = 0: the lower half's are shifted by 10.
    let mut uvs = vec![vec![0.0; nf]; 8];
    for (j, f) in mesh.faces.iter().enumerate() {
        let shift = if centroid(&mesh, f)[2] < 0.0 {
            10.0
        } else {
            0.0
        };
        for (k, &v) in f.iter().enumerate() {
            let p = mesh.positions[v as usize];
            (uvs[2 * k][j], uvs[2 * k + 1][j]) = (p[0] + shift, p[1]);
        }
    }
    let face = |values| Attribute {
        domain: AttrDomain::Face,
        values,
    };
    mesh.attributes
        .insert("group".into(), face(AttrValues::U32(groups)));
    for (i, values) in uvs.into_iter().enumerate() {
        let name = format!("uv{}.{}", i / 2, ["u", "v"][i % 2]);
        mesh.attributes.insert(name, face(AttrValues::F32(values)));
    }
    s.add_part("marked", &mesh);
    uvs_follow_points(&s.mesh("marked"), "made");
    s.ok("mesh.decimate", json!({ "part": "marked", "percent": 25 }));
    let out = s.mesh("marked");
    assert!(out.faces.len() < nf / 2, "{} faces", out.faces.len());
    assert_eq!(group_borders(&out), group_borders(&mesh), "group borders");
    let AttrValues::U32(groups) = column(&out, "group") else {
        panic!("no group column");
    };
    // Each corner on its group's side of x = 0 and y = 0, or exactly on the plane: a triangle
    // along a border may have every corner on it, and then its centroid tells no side.
    for (f, &g) in out.faces.iter().zip(groups) {
        let side = |c: f32, bit: u32| c.abs() < f32::MIN_POSITIVE || (c > 0.0) == (g & bit != 0);
        for p in points(f).into_iter().map(|v| out.positions[v as usize]) {
            assert!(
                side(p[0], 1) && side(p[1], 2),
                "a face of group {g} has corner {p:?}"
            );
        }
    }
    uvs_follow_points(&out, "decimated");
}

#[test]
fn a_masked_area_keeps_every_face_while_the_rest_gives_up_more() {
    let mut s = sphere(32);
    let mut mesh = s.mesh("sphere");
    let mask = (mesh.positions.iter())
        .map(|p| if p[2] > 0.9 { 1.0 } else { 0.0 })
        .collect();
    let point = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), point);
    s.add_part("capped", &mesh);
    let mask = f32s(column(&mesh, "mask"));
    let cap: Vec<&[u32; 4]> = (mesh.faces.iter())
        .filter(|f| f.iter().all(|&v| mask[v as usize] >= 0.5))
        .collect();
    assert!(cap.len() > 100, "{} masked quads", cap.len());
    let reply = s.query("mesh.decimate", json!({ "part": "capped", "percent": 30 }));
    assert_eq!(reply["reached"], json!(true), "{reply}");
    let out = s.mesh("capped");
    // A triangle by where its corners are, in any order.
    let key = |m: &PlainMesh, c: [u32; 3]| {
        let mut k = c.map(|v| m.positions[v as usize].map(f32::to_bits));
        k.sort_unstable();
        k
    };
    let kept: BTreeSet<_> = (out.faces.iter())
        .map(|f| key(&out, [f[0], f[1], f[2]]))
        .collect();
    for &&[a, b, c, d] in &cap {
        let has = |x, y| kept.contains(&key(&mesh, x)) && kept.contains(&key(&mesh, y));
        assert!(
            has([a, b, c], [a, c, d]) || has([a, b, d], [b, c, d]),
            "masked quad {a} {b} {c} {d} is still two triangles"
        );
    }
    let rest =
        (out.faces.len() - 2 * cap.len()) as f64 / (2 * (mesh.faces.len() - cap.len())) as f64;
    assert!(rest < 0.25, "the rest keeps {rest} of its triangles");
}

/// The open edges (on one face only), by where their ends are.
fn open_edges(mesh: &PlainMesh) -> BTreeSet<[[u32; 3]; 2]> {
    let mut edges: Vec<[u32; 2]> = (mesh.faces.iter())
        .flat_map(|f| {
            let c = points(f);
            (0..c.len())
                .map(|k| {
                    let (a, b) = (c[k], c[(k + 1) % c.len()]);
                    [a.min(b), a.max(b)]
                })
                .collect::<Vec<_>>()
        })
        .collect();
    edges.sort_unstable();
    let at = |v: u32| mesh.positions[v as usize].map(f32::to_bits);
    (edges.chunk_by(|a, b| a == b))
        .filter(|same| same.len() == 1)
        .map(|same| {
            let mut ends = same[0].map(at);
            ends.sort_unstable();
            ends
        })
        .collect()
}

#[test]
fn keep_borders_keeps_every_open_edge_and_without_it_they_thin_out() {
    let n = 20_u32;
    let side = n + 1;
    let mesh = PlainMesh {
        positions: (0..side * side)
            .map(|i| {
                let (x, y) = ((i % side) as f32 / n as f32, (i / side) as f32 / n as f32);
                [x, y, 0.3 * (x * (1.0 - x) + y * (1.0 - y))]
            })
            .collect(),
        faces: (0..n * n)
            .map(|f| {
                let v = f % n + f / n * side;
                [v, v + 1, v + side + 1, v + side]
            })
            .collect(),
        attributes: std::collections::BTreeMap::new(),
    };
    for (keep, same) in [(true, true), (false, false)] {
        let mut s = Session::new();
        s.add_part("sheet", &mesh);
        // As the part holds it, placed in the scene.
        let rim = open_edges(&s.mesh("sheet"));
        assert_eq!(rim.len(), 4 * n as usize);
        let args = json!({ "percent": 20, "keep_borders": keep });
        s.ok("mesh.decimate", args);
        let out = s.mesh("sheet");
        assert_eq!(open_edges(&out) == rim, same, "keep_borders {keep}");
    }
}

#[test]
fn a_symmetric_part_stays_exactly_symmetric_and_holds_its_map() {
    let mut s = sphere(16);
    let reply = s.query("mesh.decimate", json!({ "faces": 600, "symmetry": "x" }));
    assert_eq!(reply["reached"], json!(true), "{reply}");
    let mesh = s.mesh("sphere");
    assert!(
        (590..=600).contains(&mesh.faces.len()),
        "{} faces",
        mesh.faces.len()
    );
    assert!(mesh.attributes.contains_key("mirror.x"), "the map");
    let at: BTreeSet<[u32; 3]> = (mesh.positions.iter())
        .map(|p| p.map(f32::to_bits))
        .collect();
    for p in &mesh.positions {
        let q = [-p[0] + 0.0, p[1], p[2]];
        assert!(
            at.contains(&q.map(f32::to_bits)),
            "{p:?} has its mirror image"
        );
    }
    s.ok("symmetry.map", json!({ "axis": "x", "tolerance": 0 }));
}

#[test]
fn levels_a_lopsided_part_and_a_too_high_target_are_refused_and_no_target_is_half() {
    let mut s = sphere(4);
    s.ok("level.subdivide", json!({}));
    let err = s.err("mesh.decimate", json!({ "percent": 50 }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    let hint = err.hint.unwrap_or_default();
    assert!(
        hint.contains("level.freeze") && hint.contains("level.delete_lower"),
        "{hint}"
    );
    // Frozen levels ride across to the decimated base.
    s.ok("level.freeze", json!({}));
    s.ok("mesh.decimate", json!({ "percent": 50 }));
    s.ok("level.unfreeze", json!({}));
    let mut s = sphere(4);
    let half = s.query("mesh.decimate", json!({}));
    assert_eq!(half["target"], json!(48), "no target is half: {half}");
    let mut s = sphere(4);
    let err = s.err("mesh.decimate", json!({ "faces": 96 }));
    assert_eq!(code(err.code), "E_PRECONDITION", "not fewer faces");
    s.ok("mesh.jitter", json!({ "amount": "2%", "seed": 3 }));
    let err = s.err("mesh.decimate", json!({ "percent": 50, "symmetry": "x" }));
    assert_eq!(code(err.code), "E_PRECONDITION", "lopsided");
    assert!(err.hint.unwrap_or_default().contains("without symmetry"));
}
