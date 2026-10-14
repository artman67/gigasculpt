//! `PlainMesh` import and export: round trips, and import rejecting what the store cannot hold.

use gs_core::{
    Aabb, AttrDef, AttrKind, Domain, MeshError, MeshStore, NO, NodeConfig, Vec3, math::morton,
};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::common::{
    canonical, check, check_closed, fan, fingerprint, small_nodes, snap, sphere, store,
};

#[test]
fn export_of_an_import_is_the_same_mesh_and_imports_to_the_same_store() {
    let cases = [
        (0.0, small_nodes()),
        (0.4, small_nodes()),
        (1.0, NodeConfig::default()),
        (0.5, NodeConfig::new(2, 3).unwrap()),
    ];
    for (tris, cfg) in cases {
        let m = sphere(10, tris, 3, true);
        let s = store(&m, cfg);
        let sn = snap(&s);
        check(&sn).unwrap();
        check_closed(&s).unwrap();
        assert_eq!(s.vertex_count(), m.positions.len(), "vertices kept");
        assert_eq!(s.face_count(), m.faces.len(), "faces kept");
        let out = s.to_plain();
        assert!(canonical(&out) == canonical(&m), "round trip at {tris}");
        let again = store(&out, cfg);
        assert_eq!(
            fingerprint(&snap(&again)),
            fingerprint(&sn),
            "re-import at {tris}"
        );
    }
}

#[test]
fn nodes_are_cut_at_the_fill_size() {
    let m = sphere(10, 0.5, 2, false);
    let s = store(&m, small_nodes());
    let n = s.node_count();
    assert_eq!(n, m.positions.len().div_ceil(64), "node count");
    for k in 0..n as u32 - 1 {
        assert_eq!(s.node(k).pos.len(), 64, "node {k} filled");
    }
}

#[test]
fn attributes_keep_their_definitions_and_only_held_columns_export() {
    let mut m = sphere(6, 0.5, 4, true);
    let nv = m.positions.len();
    m.attributes.insert(
        "weight".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(vec![0.5; nv]),
        },
    );
    let mut s = store(&m, small_nodes());
    let def = |name: &str| s.attrs()[s.attr_id(name).unwrap().index()].clone();
    assert_eq!(def("mask"), AttrDef::mask());
    assert_eq!(def("group"), AttrDef::face_group());
    assert_eq!(def("color"), AttrDef::color());
    assert_eq!(
        def("weight"),
        AttrDef::new("weight", Domain::Vertex, AttrKind::Scalar)
    );
    let extra = s
        .add_attr(AttrDef::new("paint", Domain::Face, AttrKind::Label))
        .unwrap();
    assert_eq!(
        s.add_attr(AttrDef::new("paint", Domain::Face, AttrKind::Label)),
        Ok(extra)
    );
    assert!(
        s.add_attr(AttrDef::new("paint", Domain::Vertex, AttrKind::Label))
            .is_err()
    );
    let out = s.to_plain();
    assert!(
        !out.attributes.contains_key("paint"),
        "an unwritten column is not exported"
    );
    assert_eq!(out.attributes.len(), 4, "every held column is");
}

#[test]
fn the_attribute_registry_holds_61_columns() {
    let mut s = store(&sphere(2, 0.0, 1, false), small_nodes());
    for i in 0..61 {
        let d = AttrDef::new(&format!("a{i}"), Domain::Vertex, AttrKind::Scalar);
        assert!(s.add_attr(d).is_ok(), "attribute {i}");
    }
    assert!(s.add_attr(AttrDef::mask()).is_err(), "the 62nd");
}

fn with_attr(name: &str, domain: AttrDomain, values: AttrValues) -> PlainMesh {
    let mut m = sphere(3, 0.5, 1, false);
    m.attributes
        .insert(name.into(), Attribute { domain, values });
    m
}

#[test]
fn import_rejects_meshes_the_store_cannot_hold() {
    let good = sphere(3, 0.5, 1, false);
    let (nv, nf) = (good.positions.len(), good.faces.len());
    let mut cases: Vec<(&str, PlainMesh)> = Vec::new();
    let mut m = good.clone();
    m.faces[5][1] = nv as u32;
    cases.push(("index out of range", m));
    let mut m = good.clone();
    m.faces[5][3] = NO - 1;
    cases.push(("fourth index out of range", m));
    let mut m = good.clone();
    m.faces[7][2] = m.faces[7][0];
    cases.push(("repeated vertex", m));
    let mut m = good.clone();
    m.positions[9][1] = f32::NAN;
    cases.push(("NaN position", m));
    let mut m = good.clone();
    m.positions[9][2] = f32::INFINITY;
    cases.push(("infinite position", m));
    let f = AttrValues::F32(vec![0.0; nv]);
    cases.push((
        "mask on faces",
        with_attr("mask", AttrDomain::Face, AttrValues::F32(vec![0.0; nf])),
    ));
    cases.push((
        "mask as u32",
        with_attr("mask", AttrDomain::Vertex, AttrValues::U32(vec![0; nv])),
    ));
    cases.push((
        "color as f32",
        with_attr("color", AttrDomain::Vertex, f.clone()),
    ));
    cases.push((
        "short column",
        with_attr("w", AttrDomain::Vertex, AttrValues::F32(vec![0.0; nv - 1])),
    ));
    cases.push((
        "face column by vertex count",
        with_attr("w", AttrDomain::Face, f),
    ));
    // Review 0002 M5: a NaN mask turns the first stroke over it into a dead slot.
    let mut nan = vec![0.0; nv];
    nan[4] = f32::NAN;
    let nan_mask = with_attr("mask", AttrDomain::Vertex, AttrValues::F32(nan));
    cases.push(("NaN mask value", nan_mask));
    let inf = AttrValues::F32(vec![f32::NEG_INFINITY; nf]);
    cases.push(("infinite face value", with_attr("w", AttrDomain::Face, inf)));
    // Review 0002 m3: a column set names 61 attribute columns.
    let mut many = good.clone();
    for i in 0..62 {
        let column = Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(vec![0.5; nv]),
        };
        many.attributes.insert(format!("a{i:02}"), column);
    }
    cases.push(("62 attribute columns", many));
    for (name, m) in cases {
        assert!(
            matches!(
                MeshStore::from_plain(&m, small_nodes()),
                Err(MeshError::Input(_))
            ),
            "{name}"
        );
        let taken = MeshStore::import(m, small_nodes());
        assert!(matches!(taken, Err(MeshError::Input(_))), "{name} by value");
    }
}

#[test]
fn an_import_says_where_every_vertex_and_face_went() {
    let m = sphere(10, 0.4, 5, true);
    let (s, placed) = MeshStore::import(m.clone(), small_nodes()).unwrap();
    assert_eq!(
        fingerprint(&snap(&s)),
        fingerprint(&snap(&store(&m, small_nodes()))),
        "the same store as from_plain"
    );
    let out = s.to_plain();
    for (i, &at) in placed.vertices.iter().enumerate() {
        assert_eq!(out.positions[at as usize], m.positions[i], "vertex {i}");
    }
    let group = |mesh: &PlainMesh, f: usize| match &mesh.attributes["group"].values {
        AttrValues::U32(v) => v[f],
        AttrValues::F32(_) => u32::MAX,
    };
    for (i, given) in m.faces.iter().enumerate() {
        let (j, first) = (placed.faces[i] as usize, placed.first_corner[i] as usize);
        let n = if given[3] == NO { 3 } else { 4 };
        let stored = out.faces[j];
        for c in 0..n {
            let v = given[(first + c) % n];
            assert_eq!(
                stored[c], placed.vertices[v as usize],
                "face {i} corner {c}"
            );
        }
        assert_eq!(group(&out, j), group(&m, i), "face {i}'s value");
    }
}

/// A sphere with every face stored `times` times: about `2 × times` faces per vertex, the
/// shape of a real OBJ (735,637 triangles on 250,000 vertices, a third of them repeated) that
/// gave one node more faces than it has slots.
fn repeated_faces(times: usize) -> PlainMesh {
    let mut m = sphere(10, 1.0, 2, false);
    let faces = m.faces.clone();
    for _ in 1..times {
        m.faces.extend_from_slice(&faces);
    }
    m
}

#[test]
fn nodes_with_many_faces_per_vertex_are_cut_smaller() {
    let cfg = small_nodes();
    let m = repeated_faces(6);
    let s = store(&m, cfg);
    check(&snap(&s)).unwrap();
    assert!(canonical(&s.to_plain()) == canonical(&m), "the same mesh");
    let most = 4 * cfg.fill() as usize;
    for k in 0..s.node_count() as u32 {
        let node = s.node(k);
        assert!(node.pos.len() <= cfg.fill() as usize, "node {k} vertices");
        assert!(
            node.faces.len() <= most,
            "node {k}: {} faces",
            node.faces.len()
        );
    }
    let even = m.positions.len().div_ceil(cfg.fill() as usize);
    assert!(
        s.node_count() > even,
        "cut smaller than every {} vertices",
        cfg.fill()
    );
}

#[test]
fn a_vertex_may_have_255_faces_and_no_more() {
    assert!(
        MeshStore::from_plain(&fan(255), NodeConfig::default()).is_ok(),
        "255"
    );
    assert!(
        MeshStore::from_plain(&fan(256), NodeConfig::default()).is_err(),
        "256"
    );
}

#[test]
fn a_node_holds_four_faces_per_vertex_slot_and_no_more() {
    // Four vertex slots, one vertex per node: the center's node owns every fan face.
    let cfg = NodeConfig::new(2, 1).unwrap();
    assert!(MeshStore::from_plain(&fan(16), cfg).is_ok(), "16 faces");
    assert!(MeshStore::from_plain(&fan(17), cfg).is_err(), "17 faces");
}

#[test]
fn ids_limit_the_node_count() {
    // 2^20 slots per node leaves room for 1,024 nodes.
    let cfg = NodeConfig::new(20, 1).unwrap();
    let pts = |n: usize| PlainMesh {
        positions: (0..n).map(|i| [i as f32, 0.0, 0.0]).collect(),
        ..PlainMesh::default()
    };
    assert!(
        MeshStore::from_plain(&pts(1024), cfg).is_ok(),
        "1,024 nodes"
    );
    assert_eq!(
        MeshStore::from_plain(&pts(1025), cfg).err(),
        Some(MeshError::TooLarge(1025))
    );
}

#[test]
fn errors_say_what_went_wrong() {
    let err = |r: Result<MeshStore, MeshError>| r.err().unwrap().to_string();
    assert!(err(MeshStore::from_plain(&fan(256), NodeConfig::default())).contains("256 faces"));
    let line = PlainMesh {
        positions: (0..1025).map(|i| [i as f32, 0.0, 0.0]).collect(),
        ..PlainMesh::default()
    };
    assert!(
        err(MeshStore::from_plain(
            &line,
            NodeConfig::new(20, 1).unwrap()
        ))
        .contains("1025 nodes")
    );
    assert!(
        NodeConfig::new(1, 1)
            .unwrap_err()
            .to_string()
            .contains("shift 1")
    );
}

#[test]
fn morton_codes_interleave_the_quantized_axes() {
    let b = Aabb {
        lo: Vec3::new(-1.0, -2.0, 0.5),
        hi: Vec3::new(1.0, 3.0, 0.75),
    };
    let q =
        |v: f32, lo: f32, hi: f32| (((v - lo) * (2_097_151.0 / (hi - lo))) as u64).min(0x1f_ffff);
    for i in 0..400u64 {
        let h = crate::common::mix(i);
        let t = |k: u64| (crate::common::unit(crate::common::mix(h ^ k)) + 1.0) * 0.6 - 0.1;
        let p = Vec3::new(
            b.lo.x + 2.0 * t(1),
            b.lo.y + 5.0 * t(2),
            b.lo.z + 0.25 * t(3),
        );
        let axes = [
            q(p.x, b.lo.x, b.hi.x),
            q(p.y, b.lo.y, b.hi.y),
            q(p.z, b.lo.z, b.hi.z),
        ];
        let mut want = 0u64;
        for bit in 0..21 {
            for (k, a) in axes.iter().enumerate() {
                want += ((a >> bit) & 1) << (3 * bit + k);
            }
        }
        assert_eq!(morton(p, b), want, "point {i}");
    }
}

#[test]
fn node_sizes_outside_the_supported_range_are_refused() {
    for (shift, fill, ok) in [
        (2, 4, true),
        (20, 1, true),
        (1, 1, false),
        (21, 1, false),
        (6, 0, false),
        (6, 65, false),
        (6, 64, true),
    ] {
        assert_eq!(
            NodeConfig::new(shift, fill).is_ok(),
            ok,
            "shift {shift}, fill {fill}"
        );
    }
}
