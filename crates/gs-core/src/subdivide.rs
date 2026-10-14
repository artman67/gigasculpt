//! Subdivision numbering: the one numbering of a subdivision step (decision 0006), which detail
//! levels are built on (decision 0014).
//!
//! The numbering is the one ZTL files use for their levels (decision 0005), so a
//! level read from a file and a level made here number their vertices and faces alike:
//!
//! - New vertices are numbered after the old ones, so vertex `i` keeps its index.
//! - Faces are visited in order. Each edge of a face, in corner order (first to second corner,
//!   ..., last to first), takes the next number the first time any face uses it; then the
//!   face's center takes the next number.
//! - Each face is replaced in place by one quad per corner: the corner, its outgoing edge
//!   point, the center and its incoming edge point, turned so the corner sits at the corner's
//!   own position, except that the last corner always sits at position 3. A quad
//!   `(a, b, c, d)` becomes `(a, ab, m, da)`, `(ab, b, bc, m)`, `(m, bc, c, cd)`,
//!   `(da, m, cd, d)`, and a triangle `(a, b, c)` becomes `(a, ab, m, ca)`, `(ab, b, bc, m)`,
//!   `(ca, m, bc, c)`.
//!
//! So every child quad has exactly one corner from the level below (its parent corner), the
//! face point across from it and two edge points: the level code reads a fine point's kind and
//! parents from that alone.

use crate::math::{NO, arity};

/// The new vertices of one subdivision step.
pub(crate) struct Step {
    /// The vertex on each corner's outgoing edge, at `face * 4 + corner` (`NO` in a triangle's
    /// unused slot).
    pub edge: Vec<u32>,
    /// Each face's center vertex.
    pub center: Vec<u32>,
    /// The first new vertex each face numbers (one more entry at the end: `count`).
    pub start: Vec<u32>,
    /// The vertex count after the step.
    pub count: u32,
}

/// The two ends of corner `i`'s outgoing edge, lower first.
pub(crate) fn ends(f: [u32; 4], i: usize) -> (u32, u32) {
    let (a, b) = (f[i], f[(i + 1) % arity(f)]);
    (a.min(b), a.max(b))
}

/// For every corner slot (`face * 4 + corner`), the first slot in face order whose edge is the
/// same one, and the number of edges. `None` if a face refers to a vertex at or past
/// `vertex_count`, or the slots do not fit a `u32`.
fn first_uses(faces: &[[u32; 4]], vertex_count: u32) -> Option<(Vec<u32>, u64)> {
    let slots = u32::try_from(faces.len().checked_mul(4)?).ok()?;
    // Bucket the slots by their edge's lower end, in slot order; then each bucket sorted by
    // the upper end and the slot starts every run of one edge with its first use.
    let mut start = vec![0u32; vertex_count as usize + 1];
    for &f in faces {
        for i in 0..arity(f) {
            let (lo, hi) = ends(f, i);
            if hi >= vertex_count {
                return None;
            }
            start[lo as usize + 1] += 1;
        }
    }
    for v in 1..start.len() {
        start[v] += start[v - 1];
    }
    let mut bucket = vec![0u64; start[vertex_count as usize] as usize];
    for (fi, &f) in faces.iter().enumerate() {
        for i in 0..arity(f) {
            let (lo, hi) = ends(f, i);
            let at = &mut start[lo as usize];
            bucket[*at as usize] = (u64::from(hi) << 32) + (fi * 4 + i) as u64;
            *at += 1;
        }
    }
    // Each bucket's start moved to its end: bucket `v` now ends at `start[v]`.
    let mut first = vec![NO; slots as usize];
    let mut edges = 0;
    let mut from = 0;
    for &to in &start[..vertex_count as usize] {
        let run = &mut bucket[from..to as usize];
        run.sort_unstable();
        let mut last = (u64::MAX, NO);
        for &entry in &*run {
            let slot = entry as u32;
            if entry >> 32 != last.0 {
                last = (entry >> 32, slot);
                edges += 1;
            }
            first[slot as usize] = last.1;
        }
        from = to as usize;
    }
    Some((first, edges))
}

impl Step {
    /// Numbers the new vertices of `faces` over `vertex_count` old ones. `None` on a bad face
    /// or an overflow.
    pub fn new(faces: &[[u32; 4]], vertex_count: u32) -> Option<Self> {
        let (first, _) = first_uses(faces, vertex_count)?;
        Self::numbered(faces, vertex_count, &first)
    }

    /// Numbers the new vertices from each corner slot's first use: the first slot in face order
    /// whose edge is the same (`NO` in a triangle's unused slot). A level finds it from its
    /// store's rings in parallel instead of [`first_uses`]' bucket sort. `None` on an overflow.
    pub fn numbered(faces: &[[u32; 4]], vertex_count: u32, first: &[u32]) -> Option<Self> {
        // New vertices per face (its first-used edges and its center), then each face's first
        // new number; every face then numbers its own, in parallel.
        let fresh =
            |fi: usize| (fi * 4..fi * 4 + arity(faces[fi])).filter(|&s| first[s] as usize == s);
        let made = gs_par::map(faces.len(), |fi| fresh(fi).count() as u64 + 1);
        let mut start = Vec::with_capacity(faces.len() + 1);
        let mut next = u64::from(vertex_count);
        for m in made {
            start.push(u32::try_from(next).ok()?);
            next += m;
        }
        let count = u32::try_from(next).ok()?;
        start.push(count);
        let own = gs_par::map(faces.len(), |fi| {
            let mut e = [NO; 4];
            let mut k = start[fi];
            for s in fresh(fi) {
                e[s - fi * 4] = k;
                k += 1;
            }
            (e, k)
        });
        let edge = gs_par::map(faces.len() * 4, |s| {
            let f = first[s];
            if f == NO {
                NO
            } else {
                own[f as usize / 4].0[f as usize % 4]
            }
        });
        let center = own.iter().map(|o| o.1).collect();
        Some(Self {
            edge,
            center,
            start,
            count,
        })
    }

    /// The new vertices face `fi` numbers: its first-used edges' points, then its center.
    pub fn made_by(&self, fi: usize) -> std::ops::Range<u32> {
        self.start[fi]..self.start[fi + 1]
    }

    /// The edge points of face `fi`'s corners.
    pub fn edges(&self, fi: usize) -> [u32; 4] {
        let mut e = [NO; 4];
        e.copy_from_slice(&self.edge[fi * 4..fi * 4 + 4]);
        e
    }

    /// Where each new vertex comes from, by its number minus the old vertex count: the slot of
    /// the edge's first use for an edge point, `face * 4` for a face's center. New vertex `v`
    /// with source `s` is a center exactly when `center[s / 4] == v`. Every new vertex is made
    /// by one face (see [`Step::made_by`]), found by a search, so this runs in parallel.
    pub fn sources(&self, old: u32) -> Vec<u32> {
        gs_par::map((self.count - old) as usize, |i| {
            let v = old + i as u32;
            let fi = self.start.partition_point(|&s| s <= v) - 1;
            if self.center[fi] == v {
                return (fi * 4) as u32;
            }
            let slot = (fi * 4..fi * 4 + 4).find(|&s| self.edge[s] == v);
            slot.map_or(NO, |s| s as u32)
        })
    }
}

/// Child `k` of an `n`-cornered face, from the face's corners, its corners' edge points and its
/// center: the corner, its outgoing edge point, the center and its incoming edge point, turned
/// so the corner sits at position `k` (the last corner at 3). The same for vertex numbers and
/// for values kept per corner.
pub(crate) fn child<T: Copy>(
    k: usize,
    n: usize,
    corner: &[T; 4],
    edge: &[T; 4],
    center: T,
) -> [T; 4] {
    let mut c = [corner[k], edge[k], center, edge[(k + n - 1) % n]];
    c.rotate_right(if k + 1 == n { 3 } else { k });
    c
}

/// Where corner `k` of an `n`-cornered face sits in its child `k`: `k`, or 3 for the last.
#[inline]
pub(crate) fn corner_place(k: usize, n: usize) -> usize {
    if k + 1 == n { 3 } else { k }
}

/// The children of face `f` with corner edge points `e` and center `m`: one quad per corner.
pub(crate) fn children(f: [u32; 4], e: [u32; 4], m: u32) -> ([[u32; 4]; 4], usize) {
    let n = arity(f);
    let mut out = [[NO; 4]; 4];
    for (k, slot) in out.iter_mut().enumerate().take(n) {
        *slot = child(k, n, &f, &e, m);
    }
    (out, n)
}

/// One subdivision step in the numbering ZTL files use (see the module docs): the
/// faces of the level above and its vertex count. Every face becomes one quad per corner.
/// `None` if a face refers to a vertex at or past `vertex_count`, or a count would not fit.
pub fn subdivide_faces(faces: &[[u32; 4]], vertex_count: u32) -> Option<(Vec<[u32; 4]>, u32)> {
    let step = Step::new(faces, vertex_count)?;
    let mut out = Vec::new();
    for (fi, &f) in faces.iter().enumerate() {
        let (c, n) = children(f, step.edges(fi), step.center[fi]);
        out.extend_from_slice(&c[..n]);
    }
    Some((out, step.count))
}

/// Values kept per face corner (UVs), carried through one [`subdivide_faces`] step: each
/// child corner takes the value of its corner, of the middle of its edge, or of the face's
/// center (the mean of the corners). `values` holds four per face (a triangle's last is
/// unused), `corners(f)` is face `f`'s corner count, and `emit` gets each child's four values
/// in the order [`subdivide_faces`] makes the children (every child is a quad).
pub fn subdivide_corners(
    values: &[[[f32; 2]; 4]],
    corners: impl Fn(usize) -> usize,
    mut emit: impl FnMut([[f32; 2]; 4]),
) {
    for (f, v) in values.iter().enumerate() {
        let n = corners(f);
        for k in 0..n {
            emit(corner_child(v, n, k));
        }
    }
}

/// Child `k`'s four corner values of a face with `n` corners holding `v` (decision 0006's
/// linear rule: edge middles and the face's center).
pub(crate) fn corner_child<const D: usize>(v: &[[f32; D]; 4], n: usize, k: usize) -> [[f32; D]; 4] {
    let mean = |v: &[[f32; D]]| {
        let mut sum = [0.0f32; D];
        for x in v {
            for (s, c) in sum.iter_mut().zip(x) {
                *s += c;
            }
        }
        let w = 1.0 / v.len() as f32;
        sum.map(|s| s * w)
    };
    let mut edge = [[0.0; D]; 4];
    for (j, e) in edge.iter_mut().enumerate().take(n) {
        *e = mean(&[v[j], v[(j + 1) % n]]);
    }
    child(k, n, v, &edge, mean(&v[..n]))
}

/// How many vertices each level has, from the base up to `levels` steps of
/// [`subdivide_faces`] above it, without building the faces; `None` on a bad face or a count
/// past 64 bits.
pub fn level_vertex_counts(faces: &[[u32; 4]], vertex_count: u32, levels: u32) -> Option<Vec<u64>> {
    let (_, edges) = first_uses(faces, vertex_count)?;
    let mut corners: u64 = faces.iter().map(|&f| arity(f) as u64).sum();
    let (mut v, mut e, mut f) = (u64::from(vertex_count), edges, faces.len() as u64);
    let mut counts = vec![v];
    for _ in 0..levels {
        let next = v.checked_add(e)?.checked_add(f)?;
        (v, e, f) = (next, e.checked_mul(2)?.checked_add(corners)?, corners);
        corners = f.checked_mul(4)?;
        counts.push(v);
    }
    Some(counts)
}
