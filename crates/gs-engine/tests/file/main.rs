//! `file.import` and `file.export` through the engine. Kinds from ARCHITECTURE 10.1: round
//! trips through commands (exact for what each format holds), import as one undo step, replay
//! from the log with the file's content pinned, every import and export option against what it
//! promises, damaged and refused files, and ZTL tools and ZPR projects built by gs-io's own
//! test writer (no real file).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap in tests"
)]

#[expect(
    dead_code,
    reason = "only the container half of the brush writer is needed"
)]
#[path = "../../../gs-io/tests/zbp/writer.rs"]
mod brush;
#[expect(
    dead_code,
    reason = "the sample builds more files than these tests read"
)]
#[path = "../../../gs-io/tests/ztl/sample.rs"]
mod sample;
#[expect(dead_code, reason = "the writer has pieces these tests do not use")]
#[path = "../../../gs-io/tests/ztl/writer.rs"]
mod writer;

mod export;
mod import;
mod level_uvs;
mod ztl;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

pub const NO: u32 = NO_VERTEX;

/// A fresh folder for one test's files, on disk under the target folder.
pub fn folder() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("file-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `bytes` to `name` in `dir`; its absolute path.
pub fn put(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// A path in `dir`, as text.
pub fn at(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn column(domain: AttrDomain, values: AttrValues) -> Attribute {
    Attribute { domain, values }
}

/// Two quads and a triangle sharing edges, with face groups, colors, a mask an 8-bit mask
/// holds, and UVs. Faces first use the vertices in order, so merging the corners of its
/// triangles gives the vertices back in order.
pub fn painted() -> PlainMesh {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [1.0, -0.0, 0.25],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1e-7],
        [2.5, 0.0, 0.1],
        [2.0, 1.0, 0.3],
        [3.0, 0.5, 1.0 / 3.0],
    ];
    let faces = vec![[0, 1, 2, 3], [1, 4, 5, 2], [4, 6, 5, NO]];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "group".to_string(),
        column(AttrDomain::Face, AttrValues::U32(vec![3, 9, 9])),
    );
    let colors = (0..7u8).map(|i| u32::from_le_bytes([i * 30, 255 - i, 7, 255]));
    attributes.insert(
        "color".into(),
        column(AttrDomain::Vertex, AttrValues::U32(colors.collect())),
    );
    let mask = [0u8, 255, 128, 1, 0, 77, 200].map(|m| 1.0 - f32::from(m) / 255.0);
    attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask.to_vec())),
    );
    for (c, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate() {
        // A triangle's fourth corner holds no UV.
        let values = (0..3)
            .map(|f| {
                if f == 2 && c >= 6 {
                    0.0
                } else {
                    (f * 8 + c) as f32 / 16.0
                }
            })
            .collect();
        attributes.insert(
            (*name).to_string(),
            column(AttrDomain::Face, AttrValues::F32(values)),
        );
    }
    PlainMesh {
        positions,
        faces,
        attributes,
    }
}

/// `mesh` with only the named columns.
pub fn only(mesh: &PlainMesh, names: &[&str]) -> PlainMesh {
    let kept =
        (names.iter()).filter_map(|&n| Some((n.to_string(), mesh.attributes.get(n)?.clone())));
    PlainMesh {
        attributes: kept.collect(),
        ..mesh.clone()
    }
}

/// Positions, faces and columns as bits, for exact comparison.
pub fn bits(mesh: &PlainMesh) -> String {
    let positions: Vec<[u32; 3]> = mesh.positions.iter().map(|p| p.map(f32::to_bits)).collect();
    let columns: Vec<(String, Vec<u32>)> = (mesh.attributes.iter())
        .map(|(name, a)| {
            let values = match &a.values {
                AttrValues::F32(v) => v.iter().map(|x| x.to_bits()).collect(),
                AttrValues::U32(v) => v.clone(),
            };
            (name.clone(), values)
        })
        .collect();
    format!("{positions:?} {:?} {columns:?}", mesh.faces)
}

/// A session holding `painted()` as part `p`.
pub fn with_painted() -> Session {
    let mut s = Session::new();
    s.ok("part.create", json!({ "name": "p", "mesh": painted() }));
    s
}

/// The `parts` of an import's result.
pub fn parts(result: &Value) -> Vec<Value> {
    result["parts"].as_array().unwrap().clone()
}

/// The names of the scene's parts.
pub fn names(s: &mut Session) -> Vec<String> {
    let scene = s.scene();
    scene.keys().cloned().collect()
}
