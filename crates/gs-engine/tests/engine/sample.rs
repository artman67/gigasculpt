//! `mesh.sample`: reading a few vertices of a part without exporting it, either listed ids or
//! the `k` nearest a point, checked against `part.get` and slow references over its data.
//!
//! Arguments: `part`, then either `ids` (a list of vertex ids) or `near` (a point) with `k`,
//! and optionally `attributes` (column names). The result is `{ "vertices": [...] }`, each
//! entry `{ "id", "position", "normal", "attributes": { name: value } }`.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, Kind, PlainMesh};
use gs_testkit::fixtures::{cube, icosphere};
use gs_testkit::measure::{P, dot, nearest, norm, vertex_normals};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::support::agent;

/// The sampled vertices of a `mesh.sample` call that must succeed.
fn sample(s: &mut Session, args: Value) -> Vec<Value> {
    let result = s.query("mesh.sample", args);
    result["vertices"]
        .as_array()
        .unwrap_or_else(|| panic!("mesh.sample gave no vertices list: {result}"))
        .clone()
}

fn id_of(entry: &Value) -> u32 {
    let id = entry["id"].as_u64();
    id.and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("no vertex id in {entry}"))
}

fn vector(entry: &Value, key: &str) -> [f32; 3] {
    serde_json::from_value(entry[key].clone())
        .unwrap_or_else(|e| panic!("bad {key} in {entry}: {e}"))
}

/// A unit icosphere of 2,562 vertices with a vertex mask, a vertex tag and a face group, all
/// varied per element, jittered so normals are not just the positions.
fn ball(s: &mut Session) -> PlainMesh {
    let mut mesh = icosphere(4);
    let n = mesh.positions.len() as u32;
    let column = |domain, values| Attribute { domain, values };
    let mask = (0..n).map(|v| (v * 7919 % 1000) as f32 / 999.0).collect();
    let tag = (0..n).map(|v| v * 31 % 97).collect();
    let group = (0..mesh.faces.len() as u32).map(|f| f % 5).collect();
    mesh.attributes = BTreeMap::from([
        (
            "mask".into(),
            column(AttrDomain::Vertex, AttrValues::F32(mask)),
        ),
        (
            "tag".into(),
            column(AttrDomain::Vertex, AttrValues::U32(tag)),
        ),
        (
            "group".into(),
            column(AttrDomain::Face, AttrValues::U32(group)),
        ),
    ]);
    s.add_part("ball", &mesh);
    s.ok(
        "mesh.jitter",
        json!({ "part": "ball", "amount": "2%", "seed": 3 }),
    );
    let jittered = s.mesh("ball");
    assert_eq!(jittered.positions.len(), 2562, "the ball's size");
    jittered
}

/// The value of vertex `v` in a column, as JSON compares it, with `f32` values as their bits.
fn want_value(column: &Attribute, v: u32) -> (bool, u64) {
    match &column.values {
        AttrValues::F32(values) => (true, u64::from(values[v as usize].to_bits())),
        AttrValues::U32(values) => (false, u64::from(values[v as usize])),
    }
}

fn got_value(value: &Value, float: bool) -> (bool, u64) {
    let read = if float {
        value.as_f64().map(|x| u64::from((x as f32).to_bits()))
    } else {
        value.as_u64()
    };
    (
        float,
        read.unwrap_or_else(|| panic!("bad attribute value {value}")),
    )
}

/// Checks each entry against vertex `id` of `mesh`: the position bit for bit, the normal
/// within a hair of the slow reference, and exactly the columns `names` with their values.
fn check_entries(entries: &[Value], mesh: &PlainMesh, names: &[&str], what: &str) {
    let normals = vertex_normals(mesh);
    for entry in entries {
        let id = id_of(entry);
        let v = id as usize;
        assert_eq!(
            vector(entry, "position").map(f32::to_bits),
            mesh.positions[v].map(f32::to_bits),
            "{what}: vertex {id}'s position"
        );
        let normal: P = vector(entry, "normal").map(f64::from);
        assert!(
            (norm(normal) - 1.0).abs() < 1e-5 && dot(normal, normals[v]) > 1.0 - 1e-6,
            "{what}: vertex {id}'s normal {normal:?}, want {:?}",
            normals[v]
        );
        let got = entry["attributes"]
            .as_object()
            .unwrap_or_else(|| panic!("{what}: no attributes in {entry}"));
        let keys: Vec<&str> = got.keys().map(String::as_str).collect();
        assert_eq!(keys, names, "{what}: vertex {id}'s columns");
        for &name in names {
            let want = want_value(&mesh.attributes[name], id);
            assert_eq!(
                got_value(&got[name], want.0),
                want,
                "{what}: vertex {id}'s {name}"
            );
        }
    }
}

#[test]
fn listed_ids_read_what_part_get_reads() {
    let mut s = agent();
    let info = s
        .engine
        .catalog()
        .commands
        .iter()
        .find(|c| c.name == "mesh.sample");
    assert_eq!(info.map(|c| c.kind), Some(Kind::Query), "a query");
    let mesh = ball(&mut s);
    let before = s.hash();
    let ids = [2561, 0, 1234, 17, 5, 999, 12];
    let entries = sample(&mut s, json!({ "part": "ball", "ids": ids }));
    let got: Vec<u32> = entries.iter().map(id_of).collect();
    assert_eq!(got, ids, "the listed vertices, in the order listed");
    check_entries(&entries, &mesh, &["mask", "tag"], "every vertex column");
    let entries = sample(
        &mut s,
        json!({ "part": "ball", "ids": ids, "attributes": ["tag"] }),
    );
    check_entries(&entries, &mesh, &["tag"], "the tag alone");
    assert_eq!(s.hash(), before, "reading changed nothing");
}

#[test]
fn nearest_vertices_match_a_brute_force_search() {
    let mut s = agent();
    let mesh = ball(&mut s);
    let points: [P; 4] = [
        [1.1, 0.0, 0.0],
        [0.3, -0.9, 0.4],
        [-2.0, 1.0, 0.5],
        [0.01, 0.02, -0.03],
    ];
    for point in points {
        for k in [1, 7, 50] {
            let entries = sample(&mut s, json!({ "part": "ball", "near": point, "k": k }));
            let got: Vec<u32> = entries.iter().map(id_of).collect();
            assert_eq!(got, nearest(&mesh, point, k), "{k} nearest {point:?}");
            check_entries(&entries, &mesh, &["mask", "tag"], "nearest");
        }
    }
}

/// `mesh` with vertex `v` renumbered to `v * stride % n`, so ids do not follow space.
fn renumbered(mesh: &PlainMesh, stride: usize) -> PlainMesh {
    let n = mesh.positions.len();
    let to = |v: u32| (v as usize * stride % n) as u32;
    let mut positions = vec![[0.0; 3]; n];
    for (v, &p) in mesh.positions.iter().enumerate() {
        positions[to(v as u32) as usize] = p;
    }
    let keep = |v: u32| if v == u32::MAX { v } else { to(v) };
    PlainMesh {
        positions,
        faces: mesh.faces.iter().map(|f| f.map(keep)).collect(),
        attributes: BTreeMap::new(),
    }
}

#[test]
fn equally_near_vertices_come_lowest_id_first() {
    let mut s = agent();
    let grid = renumbered(&cube(4), 37);
    s.add_part("grid", &grid);
    let mesh = s.mesh("grid");
    // On the +z side's lattice of half units: four corners at exactly the same distance, then
    // a ring of eight more.
    let point = [0.25, 0.25, 1.0];
    for k in [2, 3, 4, 9, 12] {
        let entries = sample(&mut s, json!({ "part": "grid", "near": point, "k": k }));
        let got: Vec<u32> = entries.iter().map(id_of).collect();
        assert_eq!(got, nearest(&mesh, point, k), "{k} nearest, ties by id");
        check_entries(&entries, &mesh, &[], "grid");
    }
}

#[test]
fn bad_ids_and_too_many_vertices_are_refused() {
    let mut s = agent();
    ball(&mut s);
    for (ids, path) in [
        (vec![0, 1, 2562], "/ids/2"),
        (vec![0, 2562, 1], "/ids/1"),
        (vec![u32::MAX], "/ids/0"),
    ] {
        let err = s.err("mesh.sample", json!({ "part": "ball", "ids": ids }));
        assert_eq!(code(err.code), "E_ARGS_INVALID", "{ids:?}: past the end");
        assert_eq!(err.path.as_deref(), Some(path), "{ids:?}: the bad entry");
    }
    let thousand: Vec<u32> = (0..1000).collect();
    let entries = sample(&mut s, json!({ "part": "ball", "ids": thousand }));
    assert_eq!(entries.len(), 1000, "1,000 listed is allowed");
    let entries = sample(
        &mut s,
        json!({ "part": "ball", "near": [0, 0, 1], "k": 1000 }),
    );
    assert_eq!(entries.len(), 1000, "1,000 nearest is allowed");
    let too_many: Vec<u32> = (0..1001).collect();
    s.err("mesh.sample", json!({ "part": "ball", "ids": too_many }));
    s.err(
        "mesh.sample",
        json!({ "part": "ball", "near": [0, 0, 1], "k": 1001 }),
    );
}
