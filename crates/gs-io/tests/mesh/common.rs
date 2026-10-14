//! Meshes to write and compare.

use std::collections::BTreeMap;

use gs_io::mesh::{MeshFile, Object, UV_COLUMNS, WriteOptions};
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};

pub const NO: u32 = NO_VERTEX;

pub fn column(domain: AttrDomain, values: AttrValues) -> Attribute {
    Attribute { domain, values }
}

/// Positions that are hard to write as text: negative zero, a subnormal, tiny and huge values,
/// and values with no short decimal form.
pub fn hard_floats() -> Vec<[f32; 3]> {
    vec![
        [0.0, -0.0, 1.0],
        [1e-40, -3.0e-30, 0.1],
        [1.0e18, -7.25e17, 16_777_217.0],
        [std::f32::consts::PI, -std::f32::consts::E, 1.0 / 3.0],
        [0.3, 0.7, -0.9],
        [2.5, -1.5, 1e-7],
    ]
}

/// A quad and two triangles over six vertices, with every column the open formats can hold:
/// sparse face groups, colors, a mask from the values an 8-bit mask can hold, and UVs.
pub fn full_mesh() -> PlainMesh {
    let faces = vec![[0, 1, 2, 3], [1, 4, 2, NO], [3, 2, 5, NO]];
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "group".to_string(),
        column(AttrDomain::Face, AttrValues::U32(vec![7, 0, 65535])),
    );
    let colors = [
        [255, 0, 0, 255],
        [0, 128, 255, 255],
        [1, 2, 3, 255],
        [255; 4],
    ];
    let colors = (0..6).map(|i| u32::from_le_bytes(colors[i % 4]));
    attributes.insert(
        "color".into(),
        column(AttrDomain::Vertex, AttrValues::U32(colors.collect())),
    );
    let mask = [0u8, 255, 128, 1, 0, 77].map(|m| 1.0 - f32::from(m) / 255.0);
    attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask.to_vec())),
    );
    let uvs = [
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        [[1.0, 0.0], [0.25, -0.0], [1.0, 1.0], [0.0, 0.0]],
        [[0.0, 1.0], [0.9, 0.1], [3.5, 1e-9], [0.0, 0.0]],
    ];
    for (c, name) in UV_COLUMNS.iter().enumerate() {
        let values = uvs.iter().map(|f| f[c / 2][c % 2]).collect();
        attributes.insert(
            (*name).to_string(),
            column(AttrDomain::Face, AttrValues::F32(values)),
        );
    }
    PlainMesh {
        positions: hard_floats(),
        faces,
        attributes,
    }
}

/// Only positions and faces.
pub fn bare(mesh: &PlainMesh) -> PlainMesh {
    PlainMesh {
        attributes: BTreeMap::new(),
        ..mesh.clone()
    }
}

/// `mesh` with only the named columns.
pub fn only(mesh: &PlainMesh, names: &[&str]) -> PlainMesh {
    let mut out = mesh.clone();
    out.attributes
        .retain(|name, _| names.contains(&name.as_str()));
    out
}

/// Positions, faces and columns (name, per vertex, values) as bits.
type Bits = (Vec<[u32; 3]>, Vec<[u32; 4]>, Vec<(String, bool, Vec<u32>)>);

/// Everything about a mesh as bits, so equality is exact (negative zero and NaN included).
pub fn bits(mesh: &PlainMesh) -> Bits {
    let positions = mesh.positions.iter().map(|p| p.map(f32::to_bits)).collect();
    let columns = (mesh.attributes.iter())
        .map(|(name, a)| {
            let values = match &a.values {
                AttrValues::F32(v) => v.iter().map(|x| x.to_bits()).collect(),
                AttrValues::U32(v) => v.clone(),
            };
            (name.clone(), a.domain == AttrDomain::Vertex, values)
        })
        .collect();
    (positions, mesh.faces.clone(), columns)
}

/// Asserts two meshes are the same, bit for bit.
#[track_caller]
pub fn assert_same(got: &PlainMesh, want: &PlainMesh) {
    assert_eq!(bits(got), bits(want), "the meshes differ");
}

/// Writes named meshes as one file of the format `kind` (obj, ply or stl).
pub fn write_as(kind: &str, meshes: &[(&str, &PlainMesh)], options: WriteOptions) -> Vec<u8> {
    let objects: Vec<Object> = (meshes.iter())
        .map(|&(name, mesh)| Object {
            name,
            mesh,
            positions: &mesh.positions,
        })
        .collect();
    let mut out = Vec::new();
    let done = match kind {
        "obj" => gs_io::obj::write(&mut out, &objects, &options),
        "ply" => gs_io::ply::write(&mut out, &objects, &options),
        _ => gs_io::stl::write(&mut out, &objects, &options),
    };
    done.unwrap();
    out
}

/// The only object of a file.
#[track_caller]
pub fn single(file: &MeshFile) -> &PlainMesh {
    assert_eq!(file.objects.len(), 1, "one object");
    &file.objects[0].mesh
}

/// Lines in a file, counting a last line without an end.
pub fn line_count(bytes: &[u8]) -> u64 {
    bytes.split(|&b| b == b'\n').count() as u64
}

/// Faces index real vertices, and columns have one value per vertex or face.
pub fn consistent(file: &MeshFile) -> bool {
    file.objects.iter().all(|o| {
        let m = &o.mesh;
        let faces_ok = (m.faces.iter()).all(|f| {
            f[..3].iter().all(|&v| (v as usize) < m.positions.len())
                && (f[3] == NO || (f[3] as usize) < m.positions.len())
        });
        let columns_ok = m.attributes.values().all(|a| {
            let len = match &a.values {
                AttrValues::F32(v) => v.len(),
                AttrValues::U32(v) => v.len(),
            };
            len == match a.domain {
                AttrDomain::Vertex => m.positions.len(),
                AttrDomain::Face => m.faces.len(),
            }
        });
        faces_ok && columns_ok
    })
}
