//! The input mesh as triangles in grid units.

use gs_protocol::{AttrDomain, AttrValues, PlainMesh, corners, face_triangles};

use super::RemeshError;
use super::grid::Grid;
use crate::math::{V3, cross, dot, sub};

/// Triangles below this area (in square voxels, doubled) have no surface to measure from.
const FLAT: f32 = 1e-12;

/// The input as a triangle soup in grid units.
pub(crate) struct Soup {
    /// Every input vertex, in grid units.
    pub verts: Vec<V3>,
    /// Triangles as input vertex indices. Quads are split as they are drawn
    /// ([`face_triangles`]). Caps over holes come last.
    pub tris: Vec<[u32; 3]>,
    /// The input face each triangle came from, or `NONE` for a cap.
    pub face: Vec<u32>,
}

impl Soup {
    pub(crate) fn new(mesh: &PlainMesh, tris: Vec<[u32; 3]>, face: Vec<u32>, grid: &Grid) -> Self {
        let verts = gs_par::map(mesh.positions.len(), |v| grid.to_grid(mesh.positions[v]));
        Self { verts, tris, face }
    }

    /// The corners of triangle `t`.
    pub(crate) fn corners(&self, t: u32) -> [V3; 3] {
        self.tris[t as usize].map(|v| self.verts[v as usize])
    }

    /// True if triangle `t` has (next to) no area, so it has no closest point of its own.
    pub(crate) fn flat(&self, t: u32) -> bool {
        let [a, b, c] = self.corners(t);
        let n = cross(sub(b, a), sub(c, a));
        dot(n, n) <= FLAT
    }
}

/// Splits every face into triangles, dropping triangles that repeat a vertex. Also checks the
/// mesh is usable: finite positions, valid indices, attribute columns of the right length.
pub(crate) fn triangles(mesh: &PlainMesh) -> Result<(Vec<[u32; 3]>, Vec<u32>), RemeshError> {
    if !mesh.positions.iter().flatten().all(|c| c.is_finite()) {
        return Err(RemeshError::NotFinite);
    }
    for (name, attribute) in &mesh.attributes {
        let len = match &attribute.values {
            AttrValues::F32(v) => v.len(),
            AttrValues::U32(v) => v.len(),
        };
        let expected = match attribute.domain {
            AttrDomain::Vertex => mesh.positions.len(),
            AttrDomain::Face => mesh.faces.len(),
        };
        if len != expected {
            return Err(RemeshError::BadAttribute(name.clone()));
        }
    }
    let n = mesh.positions.len();
    let mut tris = Vec::with_capacity(mesh.faces.len());
    let mut face = Vec::with_capacity(mesh.faces.len());
    for (f, &q) in mesh.faces.iter().enumerate() {
        if corners(&q).iter().any(|&v| v as usize >= n) {
            return Err(RemeshError::BadFace(f));
        }
        for t in face_triangles(q, |k| mesh.positions[q[k] as usize]) {
            let t = t.map(|k| q[k]);
            if t[0] != t[1] && t[1] != t[2] && t[2] != t[0] {
                tris.push(t);
                face.push(f as u32);
            }
        }
    }
    if tris.is_empty() {
        return Err(RemeshError::NoFaces);
    }
    Ok((tris, face))
}

/// Bounding box of the vertices, in `f64`.
pub(crate) fn bounds(mesh: &PlainMesh) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in &mesh.positions {
        for a in 0..3 {
            lo[a] = lo[a].min(f64::from(p[a]));
            hi[a] = hi[a].max(f64::from(p[a]));
        }
    }
    (lo, hi)
}
