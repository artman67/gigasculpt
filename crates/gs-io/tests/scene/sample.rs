//! Scenes to write: small stores with every kind of slot a store can hold (dead slots, free
//! lists, columns only some nodes hold), stacks of detail levels with edits not carried up,
//! and a way to rewrite a file's manifest as a newer writer would, for the version and damage
//! tests.

use std::sync::Arc;

use gs_core::{
    AttrDef, AttrKind, AttrVec, ColumnSet, Domain, Levels, MeshStore, NO, NodeConfig, Numbering,
    Placement, Settings, StoredLevels,
};
use gs_io::scene::{FileKind, NumberingOut, PartOut, SceneOut, write};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use serde_json::{Value, json};

/// A bumpy grid of `n` by `n` quads, each split into two triangles, with a mask and groups.
pub fn grid(n: u32, seed: u32) -> PlainMesh {
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let h = ((i * 7 + j * 13 + seed) % 11) as f32 * 0.01;
            positions.push([i as f32 / n as f32, h, j as f32 / n as f32]);
        }
    }
    let at = |i: u32, j: u32| j * (n + 1) + i;
    let mut faces = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            faces.push([a, b, c, NO]);
            faces.push([a, c, d, NO]);
        }
    }
    let nv = positions.len();
    let mut mesh = PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    };
    let mask = (0..nv).map(|v| (v % 5) as f32 * 0.2).collect();
    let groups = (0..mesh.faces.len()).map(|f| (f % 3) as u32).collect();
    let attr = |domain, values| Attribute { domain, values };
    mesh.attributes.insert(
        "mask".into(),
        attr(AttrDomain::Vertex, AttrValues::F32(mask)),
    );
    mesh.attributes.insert(
        "group".into(),
        attr(AttrDomain::Face, AttrValues::U32(groups)),
    );
    mesh
}

/// A store of `grid(n, seed)` in small nodes, with edges split (so some nodes were cut in two
/// and hold dead slots and free lists) and a column only two nodes hold.
pub fn store(n: u32, seed: u32) -> MeshStore {
    let cfg = NodeConfig::new(6, 40).expect("node size");
    let mut s = MeshStore::from_plain(&grid(n, seed), cfg).expect("store");
    for _ in 0..50 {
        let faces: Vec<Vec<[u32; 4]>> = (0..s.node_count() as u32)
            .map(|k| {
                s.node(k)
                    .faces
                    .iter()
                    .filter(|f| f[0] != NO)
                    .copied()
                    .collect()
            })
            .collect();
        let mut editor = s.edit_topology();
        for node in faces {
            node.iter()
                .find(|f| editor.split_edge(f[0], f[1]).is_some());
        }
    }
    let free = (0..s.node_count() as u32).any(|k| !s.free_slots(k).0.is_empty());
    assert!(free, "the splits cut nodes in two and left free slots");
    let weight = s
        .add_attr(AttrDef::new("weight", Domain::Vertex, AttrKind::Scalar))
        .expect("attribute");
    let picked = s
        .add_attr(AttrDef::new("picked", Domain::Face, AttrKind::Flag))
        .expect("attribute");
    let some: Vec<u32> = (0..s.node_count().min(2) as u32).collect();
    let mut w = s.write(&some, ColumnSet::attr(weight) | ColumnSet::attr(picked));
    w.each(|node| {
        if let AttrVec::F32(v) = node.attr(weight) {
            v.fill(0.5);
        }
        if let AttrVec::Bits(b) = node.attr(picked) {
            b.set(0, true);
        }
    });
    drop(w);
    s
}

/// Properties with floats that only an exact reader brings back.
pub fn props(i: u32) -> Value {
    json!({ "voxel": { "voxel": 0.1 + f64::from(i) / 3.0, "limit": 640 }, "export": { "scale": 1.0f32 / 3.0, "offset": [0.1f32, -2.7e-8f32, 1e30f32] } })
}

/// The numbering a Part made from `grid` would have: any permutation will do for the format.
pub fn numbering(s: &MeshStore) -> (Vec<u32>, Vec<u32>, Vec<u8>) {
    let (nv, nf) = (s.vertex_count() as u32, s.face_count() as u32);
    let vertices = (0..nv).map(|v| (v * 7 + 3) % nv).collect();
    let faces = (0..nf).map(|f| nf - 1 - f).collect();
    let first = (0..nf).map(|f| (f % 3) as u8).collect();
    (vertices, faces, first)
}

/// A stack of detail levels over `grid(n, seed)` with creases on its first faces, divided
/// twice, at level 1 with edits not carried up: points moved, a group changed, a column made
/// there and the mask dropped from one node.
pub fn levels(n: u32, seed: u32) -> Levels {
    let mesh = grid(n, seed);
    let creases = (mesh.faces[..2].iter()).map(|f| (f[0], f[1], 2)).collect();
    let settings = Settings {
        creases,
        pin_corners: true,
        corners: Vec::new(),
    };
    let cfg = NodeConfig::new(6, 40).expect("node size");
    let (store, placed) = MeshStore::import(mesh, cfg).expect("store");
    let numbering = Arc::new(Numbering::from_placement(&store, placed));
    let mut l = Levels::new(store, numbering, &settings).expect("levels");
    l.divide(true).expect("divide");
    l.divide(true).expect("divide");
    l.set(1).expect("switch");
    let s = l.store_mut();
    let mask = s.attr_id("mask").expect("mask");
    let group = s.attr_id("group").expect("group");
    let weight = s
        .add_attr(AttrDef::new("weight", Domain::Vertex, AttrKind::Scalar))
        .expect("attribute");
    let mut w = s.write(&[0, 2], ColumnSet::POSITION | ColumnSet::attr(group));
    w.each(|node| {
        for p in node.positions().iter_mut() {
            *p = *p * 1.01;
        }
        if let AttrVec::U32(g) = node.attr(group) {
            g[0] = 9;
        }
    });
    drop(w);
    let mut w = s.write(&[1], ColumnSet::attr(weight) | ColumnSet::attr(mask));
    w.each(|node| {
        if let AttrVec::F32(v) = node.attr(weight) {
            v.fill(0.25);
        }
        node.clear_attr(mask);
    });
    drop(w);
    assert!(l.pending(), "the edits are pending");
    l
}

/// A scene file of the given stores, the first in store order and the rest numbered, in memory.
pub fn file(stores: &[MeshStore], preview: Option<&[u8]>) -> Vec<u8> {
    file_with(stores, &[], preview)
}

/// [`file`], then a Part for each stack of levels: its top, the top's numbering, its levels.
pub fn file_with(stores: &[MeshStore], stacks: &[Levels], preview: Option<&[u8]>) -> Vec<u8> {
    let numberings: Vec<_> = stores.iter().map(numbering).collect();
    let names: Vec<String> = (0..stores.len() + stacks.len())
        .map(|i| format!("part-{i}"))
        .collect();
    let mut parts: Vec<PartOut> = (stores.iter().zip(&numberings).zip(&names).enumerate())
        .map(|(i, ((s, (v, f, c)), name))| PartOut {
            name,
            props: props(i as u32),
            store: s,
            numbering: if i == 0 {
                NumberingOut::Spatial
            } else {
                NumberingOut::Given {
                    vertices: v,
                    faces: f,
                    first_corner: c,
                }
            },
            levels: None,
            frozen: None,
        })
        .collect();
    let stored: Vec<(Placement, StoredLevels)> = (stacks.iter())
        .map(|l| (l.level_numbering(l.top_level()).placement(), l.stored()))
        .collect();
    for (i, (l, (placed, levels))) in stacks.iter().zip(&stored).enumerate() {
        let i = stores.len() + i;
        parts.push(PartOut {
            name: &names[i],
            props: props(i as u32),
            store: l.top(),
            numbering: NumberingOut::Given {
                vertices: &placed.vertices,
                faces: &placed.faces,
                first_corner: &placed.first_corner,
            },
            levels: Some(levels),
            frozen: None,
        });
    }
    let scene = SceneOut {
        kind: FileKind::Scene,
        writer: "gigasculpt test".into(),
        preview,
        scene: json!({ "id": "0123456789abcdef" }),
        session: json!({ "seed": 7, "counter": 42 }),
        parts,
        references: Vec::new(),
    };
    let mut out = Vec::new();
    write(&mut out, &scene).expect("write");
    out
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

/// The manifest of a file, as JSON.
pub fn manifest(file: &[u8]) -> Value {
    let t = &file[file.len() - 64..];
    let (at, stored, raw) = (
        u64_at(t, 0) as usize,
        u64_at(t, 8) as usize,
        u64_at(t, 16) as usize,
    );
    let json = zstd::bulk::decompress(&file[at..at + stored], raw).expect("manifest");
    serde_json::from_slice(&json).expect("json")
}

/// The file with its manifest replaced by `manifest`, as a writer that knows more would write
/// it: everything before the manifest kept, the manifest, then a trailer that fits.
pub fn with_manifest(file: &[u8], manifest: &Value) -> Vec<u8> {
    with_manifest_json(file, &serde_json::to_vec(manifest).expect("json"))
}

/// [`with_manifest`] with the manifest given as JSON text, which may be larger than any
/// `Value` a test would want to hold.
pub fn with_manifest_json(file: &[u8], json: &[u8]) -> Vec<u8> {
    let at = u64_at(&file[file.len() - 64..], 0) as usize;
    finish(file[..at].to_vec(), json)
}

/// `file` with `bytes` laid after its chunks and listed in its manifest as `entry`, its place,
/// sizes and hash filled in: a chunk no writer would add, such as a second kept Part.
pub fn with_chunk(file: &[u8], mut entry: Value, bytes: &[u8]) -> Vec<u8> {
    let at = u64_at(&file[file.len() - 64..], 0) as usize;
    let mut out = file[..at].to_vec();
    entry["offset"] = json!(out.len());
    entry["stored"] = json!(bytes.len());
    entry["raw"] = json!(bytes.len());
    entry["hash"] = json!(&blake3::hash(bytes).to_hex()[..32]);
    out.extend_from_slice(bytes);
    out.resize(out.len().next_multiple_of(8), 0);
    let mut m = manifest(file);
    m["chunks"].as_array_mut().expect("chunks").push(entry);
    finish(out, &serde_json::to_vec(&m).expect("json"))
}

/// The entry of a Part kept beside the scene under `name`, for [`with_chunk`].
pub fn kept(name: &str) -> Value {
    json!({ "kind": "reference", "name": name, "filter": "bytes", "codec": "stored", "required": false })
}

/// A file of `kind` holding `stores` and keeping `references` beside them, in memory.
pub fn file_of(kind: FileKind, stores: &[MeshStore], references: &[(&str, &MeshStore)]) -> Vec<u8> {
    fn part<'a>(name: &'a str, store: &'a MeshStore) -> PartOut<'a> {
        PartOut {
            name,
            props: Value::Null,
            store,
            numbering: NumberingOut::Spatial,
            levels: None,
            frozen: None,
        }
    }
    let names: Vec<String> = (0..stores.len()).map(|i| format!("part-{i}")).collect();
    let file = SceneOut {
        kind,
        writer: "gigasculpt test".into(),
        preview: None,
        scene: Value::Null,
        session: Value::Null,
        parts: (names.iter().zip(stores))
            .map(|(n, s)| part(n, s))
            .collect(),
        references: (references.iter())
            .map(|(name, s)| (name.to_string(), part("part", s)))
            .collect(),
    };
    let mut out = Vec::new();
    write(&mut out, &file).expect("write");
    out
}

/// The manifest's JSON and a trailer that fits, after `out`, which ends where the manifest
/// goes.
fn finish(mut out: Vec<u8>, json: &[u8]) -> Vec<u8> {
    let at = out.len();
    let stored = zstd::bulk::compress(json, 1).expect("zstd");
    out.extend_from_slice(&stored);
    out.resize(out.len().next_multiple_of(8), 0);
    let mut trailer = [0u8; 64];
    trailer[..8].copy_from_slice(&(at as u64).to_le_bytes());
    trailer[8..16].copy_from_slice(&(stored.len() as u64).to_le_bytes());
    trailer[16..24].copy_from_slice(&(json.len() as u64).to_le_bytes());
    trailer[24..40].copy_from_slice(&blake3::hash(&stored).as_bytes()[..16]);
    trailer[40..48].copy_from_slice(&(out.len() as u64 + 64).to_le_bytes());
    trailer[56..64].copy_from_slice(b"GSS-END\n");
    out.extend_from_slice(&trailer);
    out
}
