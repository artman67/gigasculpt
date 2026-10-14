//! Small hand-made meshes whose measurements are known exactly.

use std::collections::BTreeMap;

use gs_protocol::{NO_VERTEX, PlainMesh};
pub use gs_testkit::fixtures::tetra;

pub const TRI: u32 = NO_VERTEX;

/// A mesh from positions and triangles.
pub fn triangles(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> PlainMesh {
    PlainMesh {
        positions: positions.to_vec(),
        faces: triangles.iter().map(|&[a, b, c]| [a, b, c, TRI]).collect(),
        attributes: BTreeMap::new(),
    }
}

/// `mesh` with every vertex moved by `offset`.
pub fn moved(mesh: &PlainMesh, offset: [f32; 3]) -> PlainMesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = std::array::from_fn(|i| p[i] + offset[i]);
    }
    out
}

/// `a` and `b` as one mesh, with `b`'s faces renumbered after `a`'s vertices.
pub fn joined(a: &PlainMesh, b: &PlainMesh) -> PlainMesh {
    let shift = a.positions.len() as u32;
    let mut out = a.clone();
    out.positions.extend(&b.positions);
    out.faces.extend(
        b.faces
            .iter()
            .map(|face| face.map(|v| if v == TRI { TRI } else { v + shift })),
    );
    out
}

/// How many times each item occurs.
pub fn tally<T: Ord>(items: impl Iterator<Item = T>) -> BTreeMap<T, u32> {
    let mut out = BTreeMap::new();
    for item in items {
        *out.entry(item).or_insert(0) += 1;
    }
    out
}
