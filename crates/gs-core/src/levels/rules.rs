//! Catmull-Clark rules with integer creases, and the frame details are kept in (decision 0014,
//! sections 3 and 5).
//!
//! Every rule reads one level's topology in canonical indices and sums its inputs sorted by
//! canonical index (faces by their sorted corners), so a result depends only on the canonical
//! mesh, never on how a store is cut. The whole-level divide and the sparse carry-up call the
//! same functions, so they agree bit for bit. The rules place any value that adds and scales:
//! positions, and UVs when they are smoothed ([`super::uvs`]).

use std::ops::{Add, Mul};
use std::sync::Arc;

use crate::columns::NodeTopo;
use crate::config::{FaceId, NodeConfig, VertId};
use crate::math::{NO, Vec3, arity, face_cross};
use crate::numbering::Numbering;

/// A value the rules place: a position, or a UV.
pub(crate) trait Point: Copy + Add<Output = Self> + Mul<f32, Output = Self> {
    const ZERO: Self;
}

impl Point for Vec3 {
    const ZERO: Self = Vec3::ZERO;
}

/// A UV coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Uv(pub [f32; 2]);

impl Add for Uv {
    type Output = Uv;
    #[inline]
    fn add(self, o: Uv) -> Uv {
        Uv([self.0[0] + o.0[0], self.0[1] + o.0[1]])
    }
}

impl Mul<f32> for Uv {
    type Output = Uv;
    #[inline]
    fn mul(self, s: f32) -> Uv {
        Uv([self.0[0] * s, self.0[1] * s])
    }
}

impl Point for Uv {
    const ZERO: Self = Uv([0.0; 2]);
}

/// An edge `(a, b)`, `a < b`, and its sharpness (1 to 254 levels, 255 always).
pub(crate) type Sharp = (u32, u32, u8);

/// The sharpness the edge's children get: one level less, 255 staying 255.
pub(crate) fn child_sharpness(s: u8) -> u8 {
    if s == u8::MAX { s } else { s.saturating_sub(1) }
}

/// One level's topology read in canonical indices: node topology (a skeleton's or a store's)
/// and the level's numbering.
#[derive(Clone, Copy)]
pub(crate) struct Topo<'a> {
    pub cfg: NodeConfig,
    pub nodes: &'a [Arc<NodeTopo>],
    pub num: &'a Numbering,
}

impl Topo<'_> {
    /// Faces using canonical vertex `c` (store face ids, ascending).
    #[inline]
    pub fn ring(&self, c: u32) -> &[FaceId] {
        let v = self.num.id(c);
        let cfg = self.cfg;
        self.nodes[cfg.node_of(v) as usize].ring_of(cfg.local_of(v) as usize)
    }

    /// Store face `f`'s corners as canonical indices, in stored order.
    #[inline]
    pub fn face(&self, f: FaceId) -> [u32; 4] {
        self.stored(f)
            .map(|v| if v == NO { NO } else { self.num.canonical(v) })
    }

    /// Faces using store point `v`.
    #[inline]
    pub fn ring_at(&self, v: VertId) -> &[FaceId] {
        let cfg = self.cfg;
        self.nodes[cfg.node_of(v) as usize].ring_of(cfg.local_of(v) as usize)
    }

    /// Store face `f`'s corners as store ids.
    #[inline]
    pub fn stored(&self, f: FaceId) -> [u32; 4] {
        let cfg = self.cfg;
        self.nodes[cfg.face_node(f) as usize].faces[cfg.face_local(f) as usize]
    }
}

/// A small list on the stack, spilling to the heap past `N` items.
pub(crate) struct Buf<T: Copy + Default, const N: usize> {
    head: [T; N],
    len: usize,
    rest: Vec<T>,
}

impl<T: Copy + Default + Ord, const N: usize> Buf<T, N> {
    pub fn new() -> Self {
        Self {
            head: [T::default(); N],
            len: 0,
            rest: Vec::new(),
        }
    }
    pub fn push(&mut self, x: T) {
        if self.len < N {
            self.head[self.len] = x;
        } else {
            if self.rest.is_empty() {
                self.rest.extend_from_slice(&self.head);
            }
            self.rest.push(x);
        }
        self.len += 1;
    }
    pub fn as_mut(&mut self) -> &mut [T] {
        if self.len > N {
            &mut self.rest
        } else {
            &mut self.head[..self.len]
        }
    }
    pub fn sorted(&mut self) -> &[T] {
        let s = self.as_mut();
        s.sort_unstable();
        s
    }
}

/// Corners of `f` sorted ascending (`NO` last for a triangle): a face's canonical key.
#[inline]
pub(crate) fn key(f: [u32; 4]) -> [u32; 4] {
    let mut k = f;
    k.sort_unstable();
    k
}

/// The mean of a face's corners, summed in ascending canonical order.
#[inline]
fn centroid<T: Point>(sorted: [u32; 4], p: &impl Fn(u32) -> T) -> T {
    let n = arity(sorted);
    let sum = sorted[..n].iter().fold(T::ZERO, |s, &c| s + p(c));
    sum * (1.0 / n as f32)
}

/// True if `a` and `b` are consecutive corners of `f`.
#[inline]
pub(crate) fn has_edge(f: [u32; 4], a: u32, b: u32) -> bool {
    let n = arity(f);
    (0..n).any(|i| {
        let (x, y) = (f[i], f[(i + 1) % n]);
        (x == a && y == b) || (x == b && y == a)
    })
}

/// A face point: the mean of the corners (`sorted`, ascending).
pub(crate) fn face_point<T: Point>(sorted: [u32; 4], p: &impl Fn(u32) -> T) -> T {
    centroid(sorted, p)
}

/// One subdivision step's rules over a coarse level.
pub(crate) struct Rule<'a> {
    pub topo: Topo<'a>,
    /// The coarse level's sharp edges, sorted.
    pub sharp: &'a [Sharp],
    /// Catmull-Clark, or a flat step (midpoints and centers, old points kept).
    pub smooth: bool,
    /// Boundary corners (a point with one face) stay where they are.
    pub pin_corners: bool,
}

impl Rule<'_> {
    /// The sharpness of edge `a`-`b` at this level (0 when not listed).
    pub fn sharpness(&self, a: u32, b: u32) -> u8 {
        let (a, b) = (a.min(b), a.max(b));
        match self.sharp.binary_search_by(|e| (e.0, e.1).cmp(&(a, b))) {
            Ok(i) => self.sharp[i].2,
            Err(_) => 0,
        }
    }

    /// The edge point of `a`-`b`: the mean of its ends and its two face points, or the midpoint
    /// on a sharp, boundary or non-manifold edge and in a flat step.
    pub fn edge<T: Point>(&self, a: u32, b: u32, p: &impl Fn(u32) -> T) -> T {
        let (a, b) = (a.min(b), a.max(b));
        let ends = p(a) + p(b);
        if !self.smooth || self.sharpness(a, b) > 0 {
            return ends * 0.5;
        }
        let mut faces: Buf<[u32; 4], 4> = Buf::new();
        for &f in self.topo.ring(a) {
            let fc = self.topo.face(f);
            if has_edge(fc, a, b) {
                faces.push(key(fc));
            }
        }
        let faces = faces.sorted();
        if faces.len() != 2 {
            return ends * 0.5;
        }
        (ends + (centroid(faces[0], p) + centroid(faces[1], p))) * 0.25
    }

    /// The vertex point of `c`: smooth with fewer than two sharp edges, the crease rule with
    /// two, kept in place with more (and at a pinned boundary corner, and in a flat step).
    pub fn vertex<T: Point>(&self, c: u32, p: &impl Fn(u32) -> T) -> T {
        let v = p(c);
        if !self.smooth {
            return v;
        }
        let ring = self.topo.ring(c);
        // Edge neighbours, each once per face using the edge.
        let mut ends: Buf<u32, 32> = Buf::new();
        let mut faces: Buf<[u32; 4], 16> = Buf::new();
        for &f in ring {
            let fc = self.topo.face(f);
            let n = arity(fc);
            if let Some(j) = fc[..n].iter().position(|&u| u == c) {
                ends.push(fc[(j + n - 1) % n]);
                ends.push(fc[(j + 1) % n]);
            }
            faces.push(key(fc));
        }
        let ends = ends.sorted();
        let mut nb: Buf<u32, 16> = Buf::new();
        let mut sharp: [u32; 2] = [NO; 2];
        let mut nsharp = 0;
        let mut i = 0;
        while i < ends.len() {
            let u = ends[i];
            let uses = ends[i..].iter().take_while(|&&x| x == u).count();
            nb.push(u);
            if uses != 2 || self.sharpness(c, u) > 0 {
                if nsharp < 2 {
                    sharp[nsharp] = u;
                }
                nsharp += 1;
            }
            i += uses;
        }
        match nsharp {
            0 | 1 => {
                let n = nb.as_mut().len();
                if n == 0 {
                    return v;
                }
                let nbs = nb.as_mut().iter().fold(T::ZERO, |s, &u| s + p(u));
                let fs = (faces.sorted().iter()).fold(T::ZERO, |s, &f| s + centroid(f, p));
                let nf = n as f32;
                v * ((nf - 2.0) / nf) + (nbs + fs) * (1.0 / (nf * nf))
            }
            2 if !(self.pin_corners && ring.len() == 1) => {
                v * 0.75 + (p(sharp[0]) + p(sharp[1])) * 0.125
            }
            _ => v,
        }
    }
}

/// Where a fine point comes from in one step: its parent vertex, the ends of its parent edge,
/// or its parent face's corners (sorted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Parent {
    Vertex(u32),
    Edge(u32, u32),
    Face([u32; 4]),
}

impl Parent {
    /// The parent of fine point `v` of a level whose level below has `coarse` points, read from
    /// the fine faces around `v`: each child quad holds one corner below `coarse` (its parent
    /// corner), the face point across from it and two edge points.
    pub fn of(fine: &Topo, v: u32, coarse: u32) -> Parent {
        if v < coarse {
            return Parent::Vertex(v);
        }
        let ring = fine.ring(v);
        let mut corners: Buf<u32, 8> = Buf::new();
        let mut across = false;
        for (i, &f) in ring.iter().enumerate() {
            let fc = fine.face(f);
            let at = fc.iter().position(|&u| u == v).unwrap_or(0);
            let parent = (0..4).find(|&j| fc[j] < coarse).unwrap_or(0);
            if i == 0 {
                across = (at + 2) % 4 == parent;
            }
            corners.push(fc[parent]);
        }
        let c = corners.sorted();
        if across {
            let mut out = [NO; 4];
            out[..c.len().min(4)].copy_from_slice(&c[..c.len().min(4)]);
            Parent::Face(out)
        } else {
            let hi = c.last().copied().unwrap_or(NO);
            Parent::Edge(c.first().copied().unwrap_or(NO), hi)
        }
    }

    /// The point the rule puts it at.
    pub fn place(self, rule: &Rule, p: &impl Fn(u32) -> Vec3) -> Vec3 {
        match self {
            Parent::Vertex(c) => rule.vertex(c, p),
            Parent::Edge(a, b) => rule.edge(a, b, p),
            Parent::Face(f) => face_point(f, p),
        }
    }
}

/// Corners of `f` turned so the lowest canonical index comes first, winding kept.
#[inline]
fn lowest_first(f: [u32; 4]) -> [u32; 4] {
    let n = arity(f);
    let j = (0..n).min_by_key(|&j| f[j]).unwrap_or(0);
    let mut out = f;
    out[..n].rotate_left(j);
    out
}

/// The orthonormal frame `[tangent, bitangent, normal]` at fine point `v` on positions `q`:
/// the normal of the faces around `v` (area-weighted, summed in canonical order), and the
/// direction to the lowest-numbered edge neighbour whose projection off the normal is long
/// enough. `None` when the normal vanishes or no neighbour works.
pub(crate) fn frame(fine: &Topo, v: u32, q: &impl Fn(u32) -> Vec3) -> Option<[Vec3; 3]> {
    let mut faces: Buf<[u32; 4], 16> = Buf::new();
    let mut nb: Buf<u32, 32> = Buf::new();
    for &f in fine.ring(v) {
        let fc = fine.face(f);
        let n = arity(fc);
        if let Some(j) = fc[..n].iter().position(|&u| u == v) {
            nb.push(fc[(j + n - 1) % n]);
            nb.push(fc[(j + 1) % n]);
        }
        faces.push(lowest_first(fc));
    }
    let sum = (faces.sorted().iter()).fold(Vec3::ZERO, |s, &f| s + face_cross(f, q));
    let n = sum.normalized();
    if n.len2() == 0.0 || !n.x.is_finite() {
        return None;
    }
    let at = q(v);
    let nb = nb.sorted();
    for (i, &u) in nb.iter().enumerate() {
        if i > 0 && nb[i - 1] == u {
            continue;
        }
        let e = q(u) - at;
        let el = e.len2();
        let t = e - n * n.dot(e);
        if el > 0.0 && t.len2() > 1e-12 * el {
            let t = t.normalized();
            return Some([t, n.cross(t), n]);
        }
    }
    None
}

/// `x` in frame `r`.
#[inline]
pub(crate) fn into_frame(r: &[Vec3; 3], x: Vec3) -> Vec3 {
    Vec3::new(r[0].dot(x), r[1].dot(x), r[2].dot(x))
}

/// Frame coordinates `d` back in object space.
#[inline]
pub(crate) fn out_of_frame(r: &[Vec3; 3], d: Vec3) -> Vec3 {
    r[0] * d.x + r[1] * d.y + r[2] * d.z
}
