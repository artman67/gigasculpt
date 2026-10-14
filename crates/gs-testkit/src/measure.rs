//! Geometry measures, computed the slow and obvious way in `f64`: counts, area, volume,
//! bounding box, Hausdorff distance, vertex normals and nearest vertices. Independent of the engine's code, so engine results can
//! be checked against them.
//!
//! Quads count as the two triangles they are drawn as, split along the shorter diagonal
//! (`gs_protocol::face_triangles`), the convention `PlainMesh` documents.

use std::collections::BTreeMap;

use gs_protocol::{PlainMesh, face_triangles};

use crate::compare::worst;

/// A point or vector in `f64`.
pub type P = [f64; 3];

/// Mesh statistics.
#[derive(Debug, PartialEq)]
pub struct Stats {
    /// Number of vertices.
    pub vertices: u64,
    /// Number of faces.
    pub faces: u64,
    /// Faces with three corners.
    pub triangles: u64,
    /// Faces with four corners.
    pub quads: u64,
    /// Distinct edges.
    pub edges: u64,
    /// Edges used by exactly one face.
    pub boundary_edges: u64,
    /// Surface area.
    pub area: f64,
    /// Signed volume about the origin: positive for a closed surface wound counter-clockwise
    /// seen from outside. Only meaningful for closed surfaces.
    pub volume: f64,
}

/// The statistics of a mesh. Panics on face indices past the end of the vertex list, so
/// validate first.
pub fn stats(mesh: &PlainMesh) -> Stats {
    let mut uses: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let (mut area, mut volume, mut triangles) = (0.0, 0.0, 0);
    for face in &mesh.faces {
        let corners = corners(face);
        triangles += u64::from(corners.len() == 3);
        for (a, b) in edges(corners) {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
        for [p, q, r] in split(mesh, face) {
            area += norm(cross(sub(q, p), sub(r, p))) / 2.0;
            volume += dot(p, cross(q, r)) / 6.0;
        }
    }
    let faces = mesh.faces.len() as u64;
    Stats {
        vertices: mesh.positions.len() as u64,
        faces,
        triangles,
        quads: faces - triangles,
        edges: uses.len() as u64,
        boundary_edges: uses.values().filter(|&&n| n == 1).count() as u64,
        area,
        volume,
    }
}

/// Smallest and largest corner of some points, or `None` if there are none.
pub fn bounds(points: &[[f32; 3]]) -> Option<[[f32; 3]; 2]> {
    let (first, rest) = points.split_first()?;
    Some(rest.iter().fold([*first, *first], |[lo, hi], p| {
        [
            std::array::from_fn(|i| lo[i].min(p[i])),
            std::array::from_fn(|i| hi[i].max(p[i])),
        ]
    }))
}

/// The unit normal of every vertex: the sum of the area normals of the faces using it (a quad's
/// is the sum of its two triangles', whichever way it is split), normalized. Zero for a vertex no face uses.
pub fn vertex_normals(mesh: &PlainMesh) -> Vec<P> {
    let mut sums = vec![[0.0; 3]; mesh.positions.len()];
    for face in &mesh.faces {
        let corners = corners(face);
        let area = split(mesh, face)
            .into_iter()
            .fold([0.0; 3], |sum, [p, q, r]| {
                add(sum, cross(sub(q, p), sub(r, p)))
            });
        for &v in corners {
            sums[v as usize] = add(sums[v as usize], area);
        }
    }
    sums.into_iter()
        .map(|n| {
            let length = norm(n);
            if length > 0.0 {
                scale(n, 1.0 / length)
            } else {
                n
            }
        })
        .collect()
}

/// The ids of the `k` vertices nearest `point`, nearest first, ties broken by lower id. Brute
/// force over squared distances in `f64`: from `f32` positions each difference and square is
/// exact, so only the final sum rounds.
pub fn nearest(mesh: &PlainMesh, point: P, k: usize) -> Vec<u32> {
    let mut order: Vec<(f64, u32)> = (0..mesh.positions.len() as u32)
        .map(|v| {
            let d = sub(self::point(mesh, v), point);
            (dot(d, d), v)
        })
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    order.into_iter().take(k).map(|(_, v)| v).collect()
}

/// Two-sided Hausdorff distance: the larger of the farthest any sample of `a` lies from the
/// surface of `b`, and the other way round. Samples are every vertex and every face center,
/// so the result is a lower bound of the exact distance, exact when the farthest point is a
/// sample. Brute force (every sample against every triangle), meant for test-sized meshes.
///
/// Infinite when either mesh has a coordinate that is not finite, or when one side has
/// samples and the other has no faces to measure against.
pub fn hausdorff(a: &PlainMesh, b: &PlainMesh) -> f64 {
    worst(hausdorff_from(a, b), hausdorff_from(b, a))
}

/// One-sided Hausdorff distance: the farthest any sample of `from` lies from the surface of
/// `to`. Zero when `from` lies on `to`, however much more of `to` there is. Same sampling and
/// infinities as [`hausdorff`].
pub fn hausdorff_from(from: &PlainMesh, to: &PlainMesh) -> f64 {
    if !(finite(from) && finite(to)) {
        return f64::INFINITY;
    }
    let targets: Vec<[P; 3]> = to.faces.iter().flat_map(|face| split(to, face)).collect();
    samples(from)
        .into_iter()
        .map(|p| {
            let distances = targets.iter().map(|t| point_triangle(p, t));
            distances.fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, worst)
}

/// Every vertex, then every face center.
fn samples(mesh: &PlainMesh) -> Vec<P> {
    let centers = mesh.faces.iter().map(|face| {
        let corners = corners(face);
        let sum = corners
            .iter()
            .fold([0.0; 3], |sum, &v| add(sum, point(mesh, v)));
        scale(sum, 1.0 / corners.len() as f64)
    });
    mesh.positions
        .iter()
        .map(|p| p.map(f64::from))
        .chain(centers)
        .collect()
}

/// Distance from `p` to the triangle `t`: to its plane when `p` lies over the triangle, else
/// to its nearest edge.
fn point_triangle(p: P, t: &[P; 3]) -> f64 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let sides = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
    let over = sides
        .iter()
        .all(|&(a, b)| dot(n, cross(sub(b, a), sub(p, a))) >= 0.0);
    let edge = sides
        .iter()
        .map(|&(a, b)| point_segment(p, a, b))
        .fold(f64::INFINITY, f64::min);
    let plane = (dot(n, sub(p, t[0])) / norm(n)).abs();
    if over { edge.min(plane) } else { edge }
}

/// Distance from `p` to the segment from `a` to `b`.
fn point_segment(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    // A zero-length segment gives 0 / tiny = 0 rather than NaN: the distance to its point.
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
    norm(sub(p, add(a, scale(ab, t))))
}

/// True if every coordinate is finite.
pub(crate) fn finite(mesh: &PlainMesh) -> bool {
    mesh.positions.iter().flatten().all(|c| c.is_finite())
}

pub(crate) use gs_protocol::corners;

/// The edges around a face, in winding order, closing back to the first corner.
pub(crate) fn edges(corners: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    let next = corners.iter().cycle().skip(1);
    corners.iter().copied().zip(next.copied())
}

/// A face as the triangles it is drawn as: a quad split along its shorter diagonal.
pub(crate) fn split(mesh: &PlainMesh, face: &[u32; 4]) -> Vec<[P; 3]> {
    let tris = face_triangles(*face, |k| mesh.positions[face[k] as usize]);
    tris.iter()
        .map(|t| t.map(|k| point(mesh, face[k])))
        .collect()
}

/// A vertex position in `f64`.
pub(crate) fn point(mesh: &PlainMesh, v: u32) -> P {
    mesh.positions[v as usize].map(f64::from)
}

/// `f` applied to each axis of `a` and `b`.
fn lanes(a: P, b: P, f: impl Fn(f64, f64) -> f64) -> P {
    std::array::from_fn(|i| f(a[i], b[i]))
}

/// `a + b`.
pub fn add(a: P, b: P) -> P {
    lanes(a, b, |x, y| x + y)
}

/// `a - b`.
pub fn sub(a: P, b: P) -> P {
    lanes(a, b, |x, y| x - y)
}

/// `v * s`.
pub fn scale(v: P, s: f64) -> P {
    v.map(|x| x * s)
}

/// Dot product.
pub fn dot(a: P, b: P) -> f64 {
    lanes(a, b, |x, y| x * y).iter().sum()
}

/// Cross product: component `i` pairs the two axes after `i`, cyclically.
pub fn cross(a: P, b: P) -> P {
    std::array::from_fn(|i| {
        let (j, k) = ([1, 2, 0][i], [2, 0, 1][i]);
        a[j] * b[k] - a[k] * b[j]
    })
}

/// Length of a vector.
pub fn norm(v: P) -> f64 {
    dot(v, v).sqrt()
}
