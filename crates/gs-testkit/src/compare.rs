//! Comparing numbers and meshes. The one place tests compare floats.
//!
//! Research 06 (section 2.4) found a hand-rolled `fold(0.0, f32::max)` that ignored NaN, so a
//! broken result full of NaN looked identical to the reference. Every comparison here treats NaN
//! as an infinite difference.
//!
//! Tolerances come from measurement, not habit: run the comparison on known-good results, take
//! the largest difference observed, and allow ten times that ([`Observed`]). Record the
//! tolerance next to the claim it belongs to.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, PlainMesh};

/// The larger of two differences, where NaN counts as infinite.
pub fn worst(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::INFINITY
    } else {
        a.max(b)
    }
}

/// True if `diff` is at most `tolerance`. NaN is never within any tolerance.
pub fn within(diff: f64, tolerance: f64) -> bool {
    diff <= tolerance
}

/// The largest difference between matching numbers, in `f64`. A NaN on either side, or a
/// length mismatch, is an infinite difference.
pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs())
        .fold(0.0, worst)
}

/// How far two meshes' vertices are apart.
#[derive(Clone, Copy, Debug)]
pub struct VertexDiff {
    /// The largest coordinate difference of any vertex; infinite if the vertex counts differ
    /// or a coordinate is NaN.
    pub max: f64,
    /// The vertex with that difference (the last one, among ties), if there are vertices to
    /// compare.
    pub vertex: Option<usize>,
}

/// Per-vertex comparison of positions, vertex `i` against vertex `i`. Faces and attributes are
/// not compared; use [`identical`] for that.
pub fn vertex_diff(a: &PlainMesh, b: &PlainMesh) -> VertexDiff {
    if a.positions.len() != b.positions.len() {
        return VertexDiff {
            max: f64::INFINITY,
            vertex: None,
        };
    }
    let found = a
        .positions
        .iter()
        .zip(&b.positions)
        .map(|(p, q)| max_abs_diff(p, q))
        .enumerate()
        .max_by(|x, y| x.1.total_cmp(&y.1));
    VertexDiff {
        max: found.map_or(0.0, |(_, d)| d),
        vertex: found.map(|(i, _)| i),
    }
}

/// Each vertex's bits of point column `name` (zero where the mesh has no such column), keyed by
/// its position's bits, so two meshes whose points are numbered differently compare by place.
pub fn bits_by_position(mesh: &PlainMesh, name: &str) -> BTreeMap<[u32; 3], u32> {
    let value = |i: usize| match mesh.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::F32(v)) => v[i].to_bits(),
        Some(AttrValues::U32(v)) => v[i],
        None => 0,
    };
    (mesh.positions.iter().enumerate())
        .map(|(i, p)| (p.map(f32::to_bits), value(i)))
        .collect()
}

/// The largest difference seen so far, for setting tolerances from measurement.
#[derive(Clone, Copy, Debug, Default)]
pub struct Observed {
    /// The largest difference noted (NaN noted as infinite).
    pub max: f64,
}

impl Observed {
    /// Notes one difference.
    pub fn note(&mut self, diff: f64) {
        self.max = worst(self.max, diff);
    }

    /// The tolerance policy: ten times the largest difference observed.
    pub fn tolerance(self) -> f64 {
        self.max * 10.0
    }
}

/// A canonical byte form of a mesh: every count, every bit of every number, and every
/// attribute's name, domain and value type. Equal bytes mean bit-identical meshes, so `-0.0`
/// and `0.0` differ. Independent of the engine's `mesh.hash`.
pub fn canonical(mesh: &PlainMesh) -> Vec<u8> {
    let mut out = b"mesh".to_vec();
    let coordinates = mesh.positions.iter().flatten().map(|c| c.to_bits());
    words(&mut out, mesh.positions.len(), coordinates);
    words(
        &mut out,
        mesh.faces.len(),
        mesh.faces.iter().flatten().copied(),
    );
    count(&mut out, mesh.attributes.len());
    for (name, attribute) in &mesh.attributes {
        bytes(&mut out, name.as_bytes());
        out.push(match attribute.domain {
            AttrDomain::Vertex => b'v',
            AttrDomain::Face => b'f',
        });
        match &attribute.values {
            AttrValues::F32(values) => {
                out.push(b'r');
                words(&mut out, values.len(), values.iter().map(|v| v.to_bits()));
            }
            AttrValues::U32(values) => {
                out.push(b'u');
                words(&mut out, values.len(), values.iter().copied());
            }
        }
    }
    out
}

/// The canonical byte form of a scene: every part's name and canonical mesh, in name order.
pub fn scene_bytes(scene: &BTreeMap<String, PlainMesh>) -> Vec<u8> {
    let mut out = b"scene".to_vec();
    count(&mut out, scene.len());
    for (name, mesh) in scene {
        bytes(&mut out, name.as_bytes());
        bytes(&mut out, &canonical(mesh));
    }
    out
}

/// True if two meshes are bit-identical.
pub fn identical(a: &PlainMesh, b: &PlainMesh) -> bool {
    canonical(a) == canonical(b)
}

fn count(out: &mut Vec<u8>, n: usize) {
    out.extend((n as u64).to_le_bytes());
}

fn bytes(out: &mut Vec<u8>, data: &[u8]) {
    count(out, data.len());
    out.extend(data);
}

/// `n` (the number of elements, not of words), then the words.
fn words(out: &mut Vec<u8>, n: usize, words: impl Iterator<Item = u32>) {
    count(out, n);
    out.extend(words.flat_map(u32::to_le_bytes));
}
