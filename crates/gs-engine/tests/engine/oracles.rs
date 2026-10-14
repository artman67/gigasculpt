//! Oracle self-tests: the hasher and the validator must reject what they exist to catch.

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use serde_json::{Value, json};

use gs_testkit::fixtures::tetra;
use gs_testkit::hashcheck::check_hasher;
use gs_testkit::{Session, code, text};

use crate::support::agent;

const TRI: u32 = u32::MAX;

fn hash_of(s: &mut Session, mesh: &Value) -> String {
    text(&s.query("mesh.hash", json!({ "mesh": mesh }))["hash"])
}

/// The test kit's hasher check on `mesh.hash`: the same mesh hashes the same, and every
/// one-bit change and every change of connectivity or attribute layout hashes differently.
#[test]
fn flipping_any_bit_changes_the_mesh_hash() {
    let mut s = agent();
    let checked = check_hasher(&tetra(), |mesh| hash_of(&mut s, &json!(mesh)));
    assert!(
        checked.as_ref().is_ok_and(|&n| n > 1000),
        "non-trivial and collision-free: {checked:?}"
    );
}

#[test]
fn the_mesh_hash_covers_structure_as_well_as_bytes() {
    let mut s = agent();
    let base = tetra();
    let edit = |change: &dyn Fn(&mut PlainMesh)| {
        let mut m = base.clone();
        change(&mut m);
        json!(m)
    };
    let words: Vec<f32> = (1..=12u32).map(f32::from_bits).collect();
    let twelve_words: Vec<&[f32]> = words.chunks(3).collect();
    let zeros = |values| Attribute {
        domain: AttrDomain::Vertex,
        values,
    };
    let variants = [
        json!(base),
        edit(&|m| {
            let mask = m.attributes.remove("mask");
            m.attributes.extend(mask.map(|a| ("mosk".to_string(), a)));
        }),
        edit(&|m| {
            m.attributes
                .values_mut()
                .for_each(|a| a.domain = AttrDomain::Face);
        }),
        edit(&|m| {
            m.attributes
                .insert("z".into(), zeros(AttrValues::F32(vec![0.0; 4])));
        }),
        edit(&|m| {
            m.attributes
                .insert("z".into(), zeros(AttrValues::U32(vec![0; 4])));
        }),
        edit(&|m| {
            m.attributes
                .insert("z".into(), zeros(AttrValues::U32(Vec::new())));
        }),
        edit(&|m| m.attributes.clear()),
        // The same twelve words as positions or as faces: only the counts tell them apart.
        json!({ "positions": twelve_words, "faces": [] }),
        json!({ "positions": [], "faces": [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]] }),
        // Values split differently between two attributes.
        json!({ "positions": [], "faces": [], "attributes": {
            "a": { "domain": "vertex", "values": { "u32": [1, 2] } },
            "b": { "domain": "vertex", "values": { "u32": [3] } } } }),
        json!({ "positions": [], "faces": [], "attributes": {
            "a": { "domain": "vertex", "values": { "u32": [1] } },
            "b": { "domain": "vertex", "values": { "u32": [2, 3] } } } }),
    ];
    let hashes: BTreeSet<String> = variants.iter().map(|m| hash_of(&mut s, m)).collect();
    assert_eq!(
        hashes.len(),
        variants.len(),
        "every structural variant hashes differently"
    );
    assert_eq!(
        hash_of(&mut s, &json!(tetra())),
        hash_of(&mut s, &json!(base)),
        "equal data, equal hash"
    );
}

#[test]
fn the_scene_hash_covers_part_names_and_contents() {
    let other = || {
        let mut m = tetra();
        m.positions[0][0] = 9.0;
        m
    };
    let scene = |parts: &[(&str, PlainMesh)]| {
        let mut s = agent();
        for (name, mesh) in parts {
            s.ok("part.create", json!({ "name": name, "mesh": mesh }));
        }
        s.hash()
    };
    let empty = scene(&[]);
    let a = scene(&[("a", tetra())]);
    let hashes = BTreeSet::from([
        empty.clone(),
        a.clone(),
        scene(&[("b", tetra())]),
        scene(&[("a", tetra()), ("b", other())]),
        scene(&[("a", other()), ("b", tetra())]),
    ]);
    assert_eq!(hashes.len(), 5, "names, contents and pairing all count");
    assert_eq!(
        scene(&[("a", tetra())]),
        a,
        "the same scene twice hashes the same"
    );
    let mut s = agent();
    s.ok("part.create", json!({ "name": "a", "mesh": tetra() }));
    let part = text(&s.query("mesh.hash", json!({ "part": "a" }))["hash"]);
    assert_eq!(
        part,
        hash_of(&mut s, &json!(tetra())),
        "a part hashes like its mesh"
    );
    assert_ne!(part, a, "a scene of one part is not the part");
}

fn problems(s: &mut Session, mesh: &Value) -> Value {
    let checked = s.query("mesh.validate", json!({ "mesh": mesh }));
    assert_eq!(
        checked["valid"],
        json!(checked["problems"] == json!([])),
        "valid means no problems"
    );
    checked["problems"].clone()
}

#[test]
fn valid_meshes_pass_the_validator() {
    let mut s = agent();
    for n in 1..=4 {
        s.ok(
            "primitive.sphere",
            json!({ "name": format!("s{n}"), "subdivisions": n }),
        );
        s.ok(
            "primitive.cube",
            json!({ "name": format!("c{n}"), "subdivisions": n }),
        );
    }
    s.ok("part.create", json!({ "name": "t", "mesh": tetra() }));
    let checked = s.query("mesh.validate", json!({}));
    assert_eq!(
        checked,
        json!({ "valid": true, "problems": [] }),
        "every part is valid"
    );
    let open = json!({ "positions": [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 0, 0]],
                       "faces": [[0, 1, 2, 3], [1, 4, 2, TRI]] });
    for mesh in [
        open,
        json!({ "positions": [], "faces": [] }),
        json!(tetra()),
    ] {
        assert_eq!(problems(&mut s, &mesh), json!([]), "{mesh} is valid");
    }
}

/// The tetrahedron with the value at JSON pointer `at` replaced.
fn tetra_with(at: &str, value: Value) -> Value {
    let mut mesh = json!(tetra());
    if let Some(slot) = mesh.pointer_mut(at) {
        *slot = value;
    }
    mesh
}

/// The tetrahedron without attributes, plus one vertex and optionally one face using it.
fn tetra_plus(face: Option<[u32; 4]>) -> Value {
    let mut mesh = tetra();
    mesh.attributes.clear();
    mesh.positions.push([5.0, 5.0, 5.0]);
    mesh.faces.extend(face);
    json!(mesh)
}

fn problem(kind: &str, attribute: Option<&str>, count: u64, first: &[u64]) -> Value {
    let mut found = json!({ "kind": kind, "count": count, "first": first });
    if let Some(name) = attribute {
        found["attribute"] = json!(name);
    }
    found
}

/// Meshes with one kind of damage each, with the problems the validator must report.
fn damaged_meshes() -> Vec<(Value, Vec<Value>)> {
    let pair = json!({ "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
                       "faces": [[0, 1, 2, TRI], [0, 1, 3, TRI]] });
    let points = json!({ "positions": vec![[0.0, 0.0, 0.0]; 12], "faces": [] });
    let mask = Some("mask");
    vec![
        (
            tetra_with("/positions/2/1", json!(1e39)),
            vec![problem("non_finite_position", None, 1, &[2])],
        ),
        (
            tetra_with("/faces/0", json!([0, TRI, 2, 1])),
            vec![problem("misplaced_marker", None, 1, &[0])],
        ),
        (
            tetra_with("/faces/3", json!([1, 2, 4, TRI])),
            vec![problem("index_out_of_range", None, 1, &[3])],
        ),
        (
            tetra_with("/faces/3", json!([1, 2, 2, TRI])),
            vec![problem("repeated_vertex", None, 1, &[3])],
        ),
        (
            tetra_with("/faces/1", json!([0, 1, 3, 1])),
            vec![problem("repeated_vertex", None, 1, &[1])],
        ),
        (
            tetra_plus(Some([0, 1, 4, TRI])),
            vec![
                problem("non_manifold_edge", None, 1, &[0]),
                problem("flipped_edge", None, 1, &[0]),
            ],
        ),
        (pair, vec![problem("flipped_edge", None, 1, &[0])]),
        (
            tetra_plus(None),
            vec![problem("unused_vertex", None, 1, &[4])],
        ),
        (
            points,
            vec![problem(
                "unused_vertex",
                None,
                12,
                &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            )],
        ),
        (
            tetra_with("/attributes/mask/values/f32", json!([0, 0, 0])),
            vec![problem("attribute_length", mask, 1, &[3])],
        ),
        (
            tetra_with("/attributes/group/values/u32", json!([0, 0, 0, 0, 0])),
            vec![problem("attribute_length", Some("group"), 1, &[5])],
        ),
        (
            tetra_with("/attributes/mask/values/f32/2", json!(1e39)),
            vec![problem("non_finite_attribute", mask, 1, &[2])],
        ),
    ]
}

/// Each kind of damage is reported as exactly that, with the right count and indices.
#[test]
fn every_kind_of_damage_is_caught() {
    let mut s = agent();
    for (mesh, expected) in damaged_meshes() {
        assert_eq!(
            problems(&mut s, &mesh),
            json!(expected),
            "validating {mesh}"
        );
        let refused = s.err("part.create", json!({ "name": "x", "mesh": mesh }));
        assert_eq!(
            code(refused.code),
            "E_ARGS_INVALID",
            "part.create refuses {mesh}"
        );
        assert_eq!(
            refused.path.as_deref(),
            Some("/mesh"),
            "pointing at the mesh"
        );
        assert!(refused.hint.is_some(), "says how to see the details");
        let kind = expected[0]["kind"].as_str().unwrap_or_default();
        assert!(
            refused.message.contains(kind),
            "{} names {kind}",
            refused.message
        );
    }
    assert_eq!(
        s.query("mesh.stats", json!({}))["parts"],
        json!([]),
        "nothing invalid got in"
    );
}
