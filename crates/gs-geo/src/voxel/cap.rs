//! Holes, closed before the sign is read: every hole of an edge-manifold, consistently wound
//! mesh gets a cap of triangles across its rim, so the mesh becomes closed and the exact row
//! winding can sign it.
//!
//! Vertices at the same position (bit for bit) count as one vertex here, so a mesh whose
//! triangles do not share vertices (as STL files store them) or that is split along seams still
//! reads as closed. Then the directed edges decide:
//!
//! - every edge is used as often in one direction as in the other: closed, nothing to cap;
//! - no directed edge is used twice: each edge has one triangle (a rim edge) or two that run
//!   along it in opposite directions. The rim edges form closed loops, because at every vertex
//!   as many rim edges arrive as leave. Each loop is split into simple loops wherever it passes
//!   through a vertex twice, and each simple loop is capped;
//! - anything else (an edge used by three or more triangles, or two running the same way) is
//!   left to the generalized winding number.
//!
//! A cap is made only of the loop's own vertices. Ears cut off every other vertex, round after
//! round, until three or fewer are left: `k - 2` triangles for a loop of `k`. Each runs along
//! the rim in the opposite direction to the mesh, so mesh and caps together use every edge once
//! each way: closed by construction, whatever the rim's shape, so the row count cannot leak. A
//! flat rim gets a flat cap; any cap lies within the convex hull of its rim.
//!
//! With hole closing 0 ([`CloseHoles::Off`]) the caps still sign the inside, and [`uncap`]
//! removes the surface they made from the remesh afterwards.

use std::collections::BTreeMap;

use super::attrs::Sources;
use crate::math::V3;

/// What the edges say about the mesh.
pub(crate) enum Edges {
    /// Closed: every edge is used as often in one direction as in the other.
    Closed,
    /// Holes, with the triangles that cap them and how many simple loops their rims make.
    Holes { caps: Vec<[u32; 3]>, loops: u64 },
    /// An edge used by three or more triangles, or by two that run the same way.
    Messy,
}

/// What a remesh does about holes: the hole closing setting, 0 or 1 (decision 0002, amended
/// 2026-10-12). Either way the inside is read with each hole capped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseHoles {
    /// 0: the surface the caps made is removed again, so an open mesh stays open about where
    /// its rim was.
    Off,
    /// 1, the default: each hole is closed by the surface across its cap.
    #[default]
    Large,
}

/// Reads the edges of `tris` and caps the holes, if that is what they are. `weld` gives the
/// lowest-numbered vertex at each vertex's position (`crate::weld::exact`), and the caps use
/// those.
pub(crate) fn classify<'a>(tris: &[[u32; 3]], weld: impl FnOnce() -> &'a [u32]) -> Edges {
    // Most meshes are closed as they are; only the others need welding.
    if Census::new(tris).closed() {
        return Edges::Closed;
    }
    let weld = weld();
    let welded: Vec<[u32; 3]> = tris
        .iter()
        .map(|t| t.map(|v| weld[v as usize]))
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[2] != t[0])
        .collect();
    let census = Census::new(&welded);
    if census.closed() {
        return Edges::Closed;
    }
    if census.repeated() {
        return Edges::Messy;
    }
    let loops = loops(&census.rim());
    let mut caps = Vec::new();
    for ring in &loops {
        cap(ring, &mut caps);
    }
    Edges::Holes {
        caps,
        loops: loops.len() as u64,
    }
}

/// The directed edges of a triangle list, as `from << 32 | to`: as used, and reversed. Both
/// sorted.
struct Census {
    forward: Vec<u64>,
    backward: Vec<u64>,
}

impl Census {
    fn new(tris: &[[u32; 3]]) -> Self {
        let edge = |e: usize| {
            let t = tris[e / 3];
            (u64::from(t[e % 3]) << 32) + u64::from(t[(e + 1) % 3])
        };
        let mut forward = gs_par::map(tris.len() * 3, edge);
        let mut backward = gs_par::map(tris.len() * 3, |e| edge(e).rotate_right(32));
        gs_par::sort_keys(&mut forward);
        gs_par::sort_keys(&mut backward);
        Self { forward, backward }
    }

    /// Every edge is used as often in one direction as in the other.
    fn closed(&self) -> bool {
        self.forward == self.backward
    }

    /// Some directed edge is used more than once.
    fn repeated(&self) -> bool {
        self.forward.windows(2).any(|w| w[0] == w[1])
    }

    /// The directed edges whose reverse is never used, sorted.
    fn rim(&self) -> Vec<u64> {
        let (forward, backward) = (&self.forward, &self.backward);
        let alone = gs_par::map(forward.len(), |e| {
            backward.binary_search(&forward[e]).is_err()
        });
        (forward.iter().zip(alone))
            .filter_map(|(&e, alone)| alone.then_some(e))
            .collect()
    }
}

/// Splits the rim edges (sorted, `from << 32 | to`) into simple loops: each loop's vertices in
/// the order the mesh's triangles run along them, no vertex twice.
fn loops(rim: &[u64]) -> Vec<Vec<u32>> {
    let from = |e: u64| (e >> 32) as u32;
    let mut used = vec![false; rim.len()];
    let mut out = Vec::new();
    for first in 0..rim.len() {
        if used[first] {
            continue;
        }
        // A simple path of used edges, and where each of its vertices sits in it. Every rim
        // vertex has as many edges in as out, so the walk can only stop where it started.
        let start = from(rim[first]);
        let mut path = vec![start];
        let mut at = BTreeMap::from([(start, 0)]);
        let mut v = start;
        while let Some(e) = (rim.partition_point(|&e| from(e) < v)..rim.len())
            .take_while(|&e| from(rim[e]) == v)
            .find(|&e| !used[e])
        {
            used[e] = true;
            v = rim[e] as u32;
            if let Some(&k) = at.get(&v) {
                // Back at a vertex of the path: the part from there is a simple loop, and the
                // path ends at that vertex again.
                let ring = path.split_off(k);
                for w in &ring {
                    at.remove(w);
                }
                at.insert(v, k);
                path.push(v);
                out.push(ring);
            } else {
                at.insert(v, path.len());
                path.push(v);
            }
        }
    }
    out
}

/// Adds a cap over a simple loop: ears cut off every other vertex until three or fewer are
/// left. The triangles run against the loop, so each of its edges meets its reverse.
fn cap(ring: &[u32], out: &mut Vec<[u32; 3]>) {
    let mut ring: Vec<u32> = ring.iter().rev().copied().collect();
    while ring.len() >= 3 {
        let n = ring.len();
        for i in (0..n - 1).step_by(2) {
            out.push([ring[i], ring[i + 1], ring[(i + 2) % n]]);
        }
        ring = ring.iter().copied().step_by(2).collect();
    }
}

/// The volume `tris` (indices into `positions`, caps included) enclose, measured from `centre`:
/// the sum of the signed volumes of the tetrahedra each triangle makes with it. Negative for a
/// solid that faces inward. Summed in order, in `f64`, so it is the same at any thread count.
pub(crate) fn volume(positions: &[V3], tris: &[[u32; 3]], centre: [f64; 3]) -> f64 {
    let point = |v: u32| -> [f64; 3] {
        std::array::from_fn(|a| f64::from(positions[v as usize][a]) - centre[a])
    };
    let six: f64 = (tris.iter())
        .map(|t| {
            let [a, b, c] = t.map(point);
            a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum();
    six / 6.0
}

/// Hole closing 0: removes the surface the caps made from a remesh, every face whose source is
/// a cap (`None` in `sources.faces`), then the vertices no kept face uses. What is left keeps
/// its order, its positions and its sources.
pub(crate) fn uncap(sources: &mut Sources, faces: &mut Vec<[u32; 4]>, positions: &mut Vec<V3>) {
    let kept_faces: Vec<bool> = sources.faces.iter().map(Option::is_some).collect();
    *faces = kept(faces, &kept_faces);
    sources.faces = kept(&sources.faces, &kept_faces);
    let mut used = vec![false; positions.len()];
    for &v in faces.iter().flatten() {
        used[v as usize] = true;
    }
    let mut to = vec![0u32; positions.len()];
    let mut next = 0;
    for (v, &u) in used.iter().enumerate() {
        to[v] = next;
        next += u32::from(u);
    }
    for f in faces.iter_mut() {
        *f = f.map(|v| to[v as usize]);
    }
    *positions = kept(positions, &used);
    sources.points = kept(&sources.points, &used);
}

/// The items whose flag in `keep` is set, in order.
fn kept<T: Copy>(items: &[T], keep: &[bool]) -> Vec<T> {
    let pairs = items.iter().zip(keep);
    pairs.filter(|(_, k)| **k).map(|(t, _)| *t).collect()
}
