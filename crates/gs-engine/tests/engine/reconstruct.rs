//! `level.reconstruct` (decision 0014, section 12): levels deleted below a divided top come
//! back exactly, a mesh that is no division fails naming why and changes nothing, face groups
//! may follow the coarse faces but not cut through one, the top keeps every point, face and UV
//! corner, and undo is exact.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

use crate::support::agent;

fn levels(s: &mut Session) -> Value {
    s.query("level.describe", json!({}))
}

/// The mesh as geometry, whatever its numbering: each face as its corners' positions and UVs
/// (bit patterns), turned to start at its smallest corner, the faces sorted.
fn shape(m: &PlainMesh) -> Vec<Vec<Vec<u32>>> {
    let uv = |name: &str| match m.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::F32(v)) => v.clone(),
        _ => vec![0.0; m.faces.len()],
    };
    let cols: Vec<Vec<f32>> = gs_io::mesh::UV_COLUMNS.iter().map(|n| uv(n)).collect();
    let mut out: Vec<Vec<Vec<u32>>> = (m.faces.iter().enumerate())
        .map(|(f, face)| {
            let n = if face[3] == u32::MAX { 3 } else { 4 };
            let mut corners: Vec<Vec<u32>> = (0..n)
                .map(|k| {
                    let p = m.positions[face[k] as usize].map(f32::to_bits);
                    let t = [cols[2 * k][f].to_bits(), cols[2 * k + 1][f].to_bits()];
                    [p.as_slice(), t.as_slice()].concat()
                })
                .collect();
            let low = (0..n).min_by_key(|&k| corners[k].clone()).unwrap_or(0);
            corners.rotate_left(low);
            corners
        })
        .collect();
    out.sort();
    out
}

fn column(values: AttrValues) -> Attribute {
    Attribute {
        domain: AttrDomain::Face,
        values,
    }
}

/// `fixtures::cube(n)` (each side an `n` by `n` grid: for n = 2 one division of a cube as
/// topology, for n = 4 two) with a different UV at every face corner and face group
/// `group(face)`.
fn grid_cube(n: u32, group: impl Fn(usize) -> u32) -> PlainMesh {
    let mut m = fixtures::cube(n);
    let faces = m.faces.len();
    for (i, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate() {
        let values = (0..faces).map(|f| (f * 8 + i) as f32 / 256.0).collect();
        m.attributes
            .insert((*name).into(), column(AttrValues::F32(values)));
    }
    let groups = (0..faces).map(group).collect();
    m.attributes
        .insert("group".into(), column(AttrValues::U32(groups)));
    m
}

#[test]
fn levels_deleted_below_a_divided_top_come_back_exactly() {
    let mut s = agent();
    s.add_part("c", &fixtures::cube(1));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    let meshes: Vec<PlainMesh> = (1..=3)
        .map(|k| {
            s.ok("level.set", json!({ "level": k }));
            s.mesh("c")
        })
        .collect();
    let whole = s.hash();
    s.ok("level.delete_lower", json!({}));
    let once = s.ok("level.reconstruct", json!({}));
    assert_eq!(
        (once.result["count"].clone(), once.result["active"].clone()),
        (json!(2), json!(2)),
        "the old base is level 2 and stays active"
    );
    assert_eq!(once.result["vertices"], json!(98), "the top did not change");
    s.ok("level.reconstruct", json!({}));
    assert_eq!(levels(&mut s)["levels"][0]["vertices"], json!(8));
    assert_eq!(s.hash(), whole, "the same three levels, numbering and all");
    for (k, want) in meshes.iter().enumerate() {
        s.ok("level.set", json!({ "level": k + 1 }));
        assert_eq!(json!(s.mesh("c")), json!(want), "level {}", k + 1);
    }
}

#[test]
fn a_triangle_soup_fails_naming_why_and_changes_nothing() {
    let positions = (0..9)
        .map(|i| [(i / 3) as f32 * 3.0 + (i % 3) as f32, (i % 2) as f32, 0.0])
        .collect();
    let soup = PlainMesh {
        positions,
        faces: vec![
            [0, 1, 2, u32::MAX],
            [3, 4, 5, u32::MAX],
            [6, 7, 8, u32::MAX],
        ],
        attributes: BTreeMap::new(),
    };
    let mut s = agent();
    s.add_part("soup", &soup);
    let before = s.hash();
    let err = s.err("level.reconstruct", json!({}));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(
        err.message.contains("'soup'") && err.message.contains("has a triangle (face 0)"),
        "{}",
        err.message
    );
    assert_eq!(s.hash(), before);
    // A mesh of quads that is no division of anything fails too, naming the piece.
    s.add_part("one", &fixtures::cube(1));
    let err = s.err("level.reconstruct", json!({ "part": "one" }));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(err.message.contains("piece 1"), "{}", err.message);
}

#[test]
fn groups_may_follow_the_coarse_faces_but_not_cut_one() {
    let mut s = agent();
    // One group per side of the cube: the borders run along the coarse faces.
    let sides = grid_cube(2, |f| (f / 4) as u32);
    s.add_part("sides", &sides);
    let made = s.ok("level.reconstruct", json!({}));
    assert_eq!(
        (made.result["count"].clone(), made.result["active"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(levels(&mut s)["levels"][0]["faces"], json!(6));
    assert_eq!(
        shape(&s.mesh("sides")),
        shape(&sides),
        "every point, face and UV corner of the top kept"
    );
    // One face of a side in a group of its own: the border cuts through a coarse face.
    let cut = grid_cube(2, |f| u32::from(f == 5));
    s.add_part("cut", &cut);
    let before = s.hash();
    let err = s.err("level.reconstruct", json!({ "part": "cut" }));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(
        err.message.contains("face-group border runs through"),
        "{}",
        err.message
    );
    assert_eq!(s.hash(), before);
}

#[test]
fn a_piece_split_along_coarse_faces_reconstructs_and_one_split_across_fails() {
    let mut s = agent();
    // Three sides in group 1: the piece split off is half the cube, whole coarse faces.
    s.add_part("half", &grid_cube(2, |f| u32::from(f < 12)));
    s.ok("part.split", json!({ "by": "groups" }));
    s.ok("level.reconstruct", json!({ "part": "half" }));
    s.ok("level.set", json!({ "part": "half", "level": 1 }));
    assert_eq!(s.mesh("half").faces.len(), 3, "three coarse quads");
    // One fine face in group 1: the piece left has a hole through a coarse face.
    s.add_part("holed", &grid_cube(2, |f| u32::from(f == 5)));
    s.ok("part.split", json!({ "part": "holed", "by": "groups" }));
    let before = s.hash();
    let err = s.err("level.reconstruct", json!({ "part": "holed" }));
    assert_eq!(err.code, ErrorCode::Precondition);
    assert!(err.message.contains("piece 1"), "{}", err.message);
    assert_eq!(s.hash(), before);
}

#[test]
fn reconstruct_is_one_undo_step() {
    let mut s = agent();
    let grid = grid_cube(4, |_| 0);
    s.add_part("c", &grid);
    s.ok("level.reconstruct", json!({}));
    let before = s.hash();
    s.ok("level.set", json!({ "level": 1 }));
    let low = s.hash();
    s.ok("level.reconstruct", json!({}));
    let d = levels(&mut s);
    assert_eq!(
        (d["count"].clone(), d["active"].clone()),
        (json!(3), json!(2)),
        "the same mesh stays active"
    );
    assert_eq!(d["levels"][0]["faces"], json!(6));
    let after = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), low);
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before);
    s.ok("history.redo", json!({}));
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after);
    s.ok("level.set", json!({ "level": 3 }));
    assert_eq!(shape(&s.mesh("c")), shape(&grid), "the top kept");
}
