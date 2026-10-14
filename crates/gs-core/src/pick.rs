//! Ray casts and nearest points: the surface a screen-space stroke hits, and what `pick.ray`
//! and `pick.nearest` report (ARCHITECTURE 7.5). The one home of ray casting.
//!
//! Every search walks the node tree nearest first and stops at the first node that cannot hold
//! anything nearer than the best found, then does the same inside each node over the boxes of
//! its runs of 64 slots (decision 0001, amended 2026-10-11), so its cost follows what lies near
//! the ray or point, not the size of the mesh or of a node. Ties go to the lowest id, so
//! answers never depend on search order.

use gs_protocol::HIDE;

use crate::columns::AttrValue;
use crate::config::{FaceId, NodeId, VertId};
use crate::derived::RUN;
use crate::math::{Aabb, NO, Vec3, face_triangles};
use crate::store::MeshStore;
use crate::view::PosRead;

/// Where a ray met the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Distance from the ray's origin.
    pub t: f32,
    pub pos: Vec3,
    pub face: FaceId,
    /// Unit normal of the triangle hit, by its winding (outward on a closed mesh wound
    /// consistently).
    pub normal: Vec3,
}

/// What a ray cast found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RayCast {
    pub hit: Option<Hit>,
    /// Faces tested: the work counter that keeps a cast to the nodes along the ray.
    pub faces_tested: usize,
}

/// What a nearest-vertex search found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nearest {
    /// The vertex and its distance.
    pub found: Option<(VertId, f32)>,
    /// Vertex slots measured: the work counter that keeps a search to the nodes near it.
    pub tested: usize,
}

impl MeshStore {
    /// The first visible face along the ray from `origin` along `dir`: the nearest hit at or
    /// past the origin, the lowest face id on a tie. Faces are hit from either side; hidden
    /// faces are passed through; a quad counts as the two triangles it is drawn as
    /// ([`face_triangles`](crate::math::face_triangles)).
    pub fn ray_cast(&self, origin: Vec3, dir: Vec3) -> RayCast {
        let mut out = RayCast::default();
        let d = dir.normalized();
        if !(finite(origin) && d.len2() > 0.0) {
            return out;
        }
        let (mut nodes, _) = self.der.tree.ray(origin, d);
        nodes.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let (pos, hide) = (self.pos(), self.attr_id(HIDE));
        for (n, enter) in nodes {
            // A node's faces lie inside its box, so none past here can be nearer.
            if out.hit.is_some_and(|h| enter > h.t) {
                break;
            }
            let node = self.node(n);
            let hidden = hide.and_then(|h| node.attr(h));
            // The same inside the node, run by run, each box grown by what rounding can move a
            // hit, so a run is passed over only when none of its faces can be the answer.
            let pad = slack(origin, self.halo(n));
            let boxes = self.der.runs[n as usize].faces(node.faces, &pos);
            let mut runs: Vec<(f32, usize)> = (boxes.iter().enumerate())
                .filter_map(|(r, b)| grown(*b, pad).ray_entry(origin, d).map(|e| (e, r)))
                .collect();
            runs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (enter, r) in runs {
                if out.hit.is_some_and(|h| enter > h.t) {
                    break;
                }
                let first = r * RUN;
                let run = &node.faces[first..(first + RUN).min(node.faces.len())];
                for (j, f) in (first..).zip(run) {
                    if f[0] == NO || hidden.is_some_and(|h| h.get(j) != AttrValue::U32(0)) {
                        continue;
                    }
                    out.faces_tested += 1;
                    let face = node.first_face + j as u32;
                    let Some((t, normal)) = face_hit(*f, &pos, origin, d) else {
                        continue;
                    };
                    if out
                        .hit
                        .is_none_or(|h| (t.total_cmp(&h.t).then(face.cmp(&h.face))).is_lt())
                    {
                        let pos = origin + d * t;
                        out.hit = Some(Hit {
                            t,
                            pos,
                            face,
                            normal,
                        });
                    }
                }
            }
        }
        out
    }

    /// The live vertex nearest `p`, and its distance; the lowest id on a tie.
    pub fn nearest_vertex(&self, p: Vec3) -> Nearest {
        let gap = move |b: Aabb| (b.lo - p).max(p - b.hi).max(Vec3::ZERO);
        self.nearest_by(move |b| wide_len2(gap(b)), |q| wide_len2(q - p))
    }

    /// The live vertex nearest the ray from `origin` along `dir`, and its distance from the
    /// ray (from the origin, for a vertex behind it); the lowest id on a tie. A ray that misses
    /// the surface says how far off it passed with this.
    pub fn nearest_to_ray(&self, origin: Vec3, dir: Vec3) -> Nearest {
        let d = dir.normalized();
        let gap2 = move |q: Vec3| {
            let s = q - origin;
            wide_len2(s - d * s.dot(d).max(0.0))
        };
        // No point of a box is nearer than its bounding sphere, less what rounding can take
        // off a distance measured in f32 (a few parts in 2^21 of the lengths it is made of).
        let bound = move |b: Aabb| {
            let r = 0.5 * wide_len2(b.hi - b.lo).sqrt();
            let c = (b.lo + b.hi) * 0.5;
            let slack = (wide_len2(c - origin).sqrt() + wide_len2(c).sqrt() + r) / 16384.0;
            let gap = (gap2(c).sqrt() - r - slack).max(0.0);
            gap * gap
        };
        self.nearest_by(bound, gap2)
    }

    /// The live vertex with the least `dist2`, searching nodes, then the runs of each node's
    /// vertex slots, in order of `bound`, a lower bound of `dist2` over a box. Both are in f64:
    /// a point mapped into the space of a part scaled far down (by 1e-24, say) lies so far out
    /// that its squared distance overflows f32 (review 0008, m3).
    fn nearest_by(&self, bound: impl Fn(Aabb) -> f64, dist2: impl Fn(Vec3) -> f64) -> Nearest {
        let nodes: Vec<Aabb> = (0..self.node_count() as NodeId)
            .map(|n| self.bounds(n))
            .collect();
        let mut out = Nearest::default();
        let mut best: Option<(f64, VertId)> = None;
        for (lo, n) in nearest_first(&nodes, &bound) {
            if best.is_some_and(|b| lo > b.0) {
                break;
            }
            let node = self.node(n as NodeId);
            for (lo, r) in nearest_first(self.der.runs[n].verts(node.pos), &bound) {
                if best.is_some_and(|b| lo > b.0) {
                    break;
                }
                let first = r * RUN;
                let run = &node.pos[first..(first + RUN).min(node.pos.len())];
                out.tested += run.len();
                for (v, &q) in (node.base + first as VertId..).zip(run) {
                    let d2 = dist2(q);
                    if !d2.is_nan()
                        && best.is_none_or(|b| (d2.total_cmp(&b.0).then(v.cmp(&b.1))).is_lt())
                    {
                        best = Some((d2, v));
                    }
                }
            }
        }
        // The distance itself fits f32: only its square may not.
        out.found = best.map(|(d2, v)| (v, d2.sqrt() as f32));
        out
    }
}

/// The boxes holding a point, by index, with their `bound`, least first (the lowest index on a
/// tie): an empty box, or one whose bound is not finite (a query that is), is left out.
fn nearest_first(boxes: &[Aabb], bound: impl Fn(Aabb) -> f64) -> Vec<(f64, usize)> {
    let mut v: Vec<(f64, usize)> = (boxes.iter().enumerate())
        .filter(|(_, b)| b.lo.x <= b.hi.x)
        .map(|(i, b)| (bound(*b), i))
        .filter(|x| x.0.is_finite())
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    v
}

/// How far rounding can move a ray's hit on a face inside box `b`, for a ray from `o`: 2^-14 of
/// the distance from `o` to the box's farthest corner. That is far beyond the error of
/// [`triangle_hit`] on a face that is not a sliver, for a ray more than a few degrees off it.
fn slack(o: Vec3, b: Aabb) -> f32 {
    let far = (b.lo - o).max(o - b.lo).max((b.hi - o).max(o - b.hi));
    far.len2().sqrt() / 16384.0
}

/// `b` grown by `pad` on every side; an empty box stays empty.
fn grown(b: Aabb, pad: f32) -> Aabb {
    if b.lo.x > b.hi.x {
        return Aabb::EMPTY;
    }
    let p = Vec3::new(pad, pad, pad);
    Aabb {
        lo: b.lo - p,
        hi: b.hi + p,
    }
}

/// The squared length of `v`, summed in f64.
fn wide_len2(v: Vec3) -> f64 {
    v.to_array()
        .iter()
        .map(|&c| f64::from(c) * f64::from(c))
        .sum()
}

fn finite(p: Vec3) -> bool {
    p.to_array().iter().all(|x| x.is_finite())
}

/// The nearest hit of a unit ray on face `f`, at or past its origin, with the normal of the
/// triangle hit.
fn face_hit(f: [u32; 4], pos: &impl PosRead, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
    let p = f.map(|v| if v == NO { Vec3::ZERO } else { pos.get(v) });
    // The nearer; the first triangle on a tie.
    face_triangles(f, |k| p[k].to_array())
        .iter()
        .filter_map(|t| triangle_hit(o, d, t.map(|k| p[k])))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Where the ray from `o` along unit `d` meets triangle `a b c`, from either side, at or past
/// `o`: its distance and the triangle's unit normal by its winding (Moller-Trumbore, in `f32`).
/// The test every cast runs on each triangle it reaches.
pub fn triangle_hit(o: Vec3, d: Vec3, [a, b, c]: [Vec3; 3]) -> Option<(f32, Vec3)> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let q = s.cross(e1);
    let (u, v, t) = (s.dot(p) * inv, d.dot(q) * inv, e2.dot(q) * inv);
    let inside = u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t >= 0.0;
    inside.then(|| (t, e1.cross(e2).normalized()))
}
