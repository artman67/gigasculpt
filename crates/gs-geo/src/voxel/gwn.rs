//! Sign for open or inconsistently wound input: the generalized winding number (Jacobson et al.
//! 2013), summed fast with a tree (Barill et al. 2018).
//!
//! The winding number at a point is the solid angle the mesh covers, seen from that point,
//! divided by a full sphere. It is 1 inside a closed mesh and 0 outside, and it degrades
//! gracefully on open meshes: holes get a smooth "soap film" where it crosses one half. The
//! tree sums distant clusters of triangles as one dipole each, so a query costs about the log
//! of the triangle count instead of the count.

use crate::math::{cross, dot, morton, sub};

/// Triangles per leaf.
const LEAF: usize = 8;
/// A cluster is summed as a dipole when the query is more than this many cluster radii away.
const BETA: f64 = 3.0;
/// A full turn: the solid angles at a point inside a closed mesh add up to twice this.
const TURN: f64 = std::f64::consts::TAU;

type P = [f64; 3];

fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}

/// A cluster of triangles: those in `range` of the sorted list.
struct Node {
    /// Mean of the triangles' centroids.
    center: P,
    /// Squared distance beyond which the dipole stands in for the cluster.
    far2: f64,
    /// Sum of the triangles' vector areas (half the cross product of two edges).
    area: P,
    range: [u32; 2],
    /// Child nodes; `[0, 0]` for a leaf (the root is never a child).
    kids: [u32; 2],
}

/// A winding number tree over a triangle list.
pub(crate) struct Gwn {
    tris: Vec<[P; 3]>,
    nodes: Vec<Node>,
}

impl Gwn {
    /// Builds the tree. Triangles are ordered along a Z curve of their centroids, so each node
    /// is a compact cluster.
    pub(crate) fn new(mut tris: Vec<[P; 3]>) -> Self {
        // Triangles with no area cover no solid angle; drop them before they divide by zero.
        tris.retain(|t| {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            dot(n, n) >= f64::MIN_POSITIVE
        });
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in tris.iter().flatten() {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let scale = (0..3)
            .map(|a| hi[a] - lo[a])
            .fold(f64::MIN_POSITIVE, f64::max);
        let key = |t: &[P; 3]| {
            let c: [u32; 3] = std::array::from_fn(|a| {
                let mean = (t[0][a] + t[1][a] + t[2][a]) / 3.0;
                ((mean - lo[a]) / scale * 1023.0) as u32
            });
            morton(c, 10)
        };
        let mut keyed: Vec<(u64, usize)> = tris.iter().map(key).zip(0..).collect();
        gs_par::sort(&mut keyed);
        tris = keyed.iter().map(|&(_, t)| tris[t]).collect();
        let mut gwn = Self {
            tris,
            nodes: Vec::new(),
        };
        if !gwn.tris.is_empty() {
            gwn.node(0, gwn.tris.len());
        }
        gwn
    }

    /// Adds the node for triangles `lo..hi` and its subtree; returns its index.
    fn node(&mut self, lo: usize, hi: usize) -> u32 {
        let index = self.nodes.len();
        let tris = &self.tris[lo..hi];
        let mut center = [0.0; 3];
        let mut area = [0.0; 3];
        for t in tris {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            for a in 0..3 {
                center[a] += (t[0][a] + t[1][a] + t[2][a]) / (3 * tris.len()) as f64;
                area[a] += n[a] / 2.0;
            }
        }
        let radius = tris
            .iter()
            .flatten()
            .map(|p| norm(sub(*p, center)))
            .fold(0.0, f64::max);
        self.nodes.push(Node {
            center,
            far2: (BETA * radius) * (BETA * radius),
            area,
            range: [lo as u32, hi as u32],
            kids: [0, 0],
        });
        if hi - lo > LEAF {
            let mid = lo + (hi - lo) / 2;
            let kids = [self.node(lo, mid), self.node(mid, hi)];
            self.nodes[index].kids = kids;
        }
        index as u32
    }

    /// True if `q` is inside: the winding number is at least one half. Also returns how many
    /// terms (exact triangles plus dipoles) the sum took.
    pub(crate) fn inside(&self, q: P) -> (bool, u64) {
        let (sum, terms) = self.angle(q);
        (sum >= TURN, terms)
    }

    /// The winding number at `q`, and the number of terms summed.
    pub(crate) fn number(&self, q: P) -> (f64, u64) {
        let (sum, terms) = self.angle(q);
        (sum / (2.0 * TURN), terms)
    }

    /// The total solid angle the mesh covers seen from `q`, and the number of terms summed.
    fn angle(&self, q: P) -> (f64, u64) {
        let mut sum = 0.0;
        let mut terms = 0;
        let mut stack = Vec::new();
        if !self.nodes.is_empty() {
            stack.push(0u32);
        }
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            let d = sub(node.center, q);
            let d2 = dot(d, d);
            if d2 >= node.far2 {
                sum += dot(d, node.area) / (d2 * d2.sqrt());
                terms += 1;
            } else if node.kids == [0, 0] {
                for t in &self.tris[node.range[0] as usize..node.range[1] as usize] {
                    sum += solid_angle(q, t);
                    terms += 1;
                }
            } else {
                stack.extend(node.kids);
            }
        }
        (sum, terms)
    }
}

/// The signed solid angle triangle `t` covers seen from `q` (Van Oosterom and Strackee 1983).
fn solid_angle(q: P, t: &[P; 3]) -> f64 {
    let [a, b, c] = t.map(|p| sub(p, q));
    let (la, lb, lc) = (norm(a), norm(b), norm(c));
    let det = dot(a, cross(b, c));
    let denominator = la * lb * lc + dot(a, b) * lc + dot(b, c) * la + dot(c, a) * lb;
    2.0 * libm::atan2(det, denominator)
}
