//! Step 5 of a pass (decision 0021, section 2): split the long edges, longest first, a
//! longer side of an edge's triangles before the edge.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::config::{FaceId, NodeId, VertId};
use crate::math::{NO, Vec3, leans};
use crate::mirror;
use crate::topo::Edge;

use super::Pass;
use super::pair::shared_face;
use crate::live::LiveReport;

/// After a split, a neighbour edge this many times longer joins the queue.
const NEIGHBOUR_SPLIT: f32 = 1.2;

/// The split queue, longest first: a key with its ends reversed (so ties go to lower ids), and
/// whether the edge splits to make way for a shorter one beside it even if it is not long.
type Queue = BinaryHeap<(u32, Reverse<VertId>, Reverse<VertId>, bool)>;

/// Tries of the split step (an edge from the queue, or a side tried before the edge that asked)
/// per edge it started with or operation the pass ran, past which the pass stops as if its
/// budget ran out, and a debug build fails. A split queues at most 16 edges and every side
/// tried first is longer than the edge that asked or a crossing side that asks for none, so a
/// working pass stays far below this; it only ends a pass that has stopped making progress.
const MOST_TRIES: u64 = 64;

/// What trying to split an edge came to.
enum Tried {
    /// It split, with its image.
    Split,
    /// It did not split: gone, short, waiting, no room, or out of budget.
    Not,
    /// This side of its triangles must split first.
    First(VertId, VertId),
}

impl Pass<'_, '_> {
    pub(super) fn split_all(&mut self, edges: &[(VertId, VertId)], out: &mut LiveReport) {
        let mut queue: Queue = BinaryHeap::new();
        for &(a, b) in edges {
            if self.ed.is_live(a) && self.ed.is_live(b) && self.long_pair(a, b) {
                self.push(&mut queue, a, b, false);
            }
        }
        let mut tries = 0u64;
        while let Some((_, Reverse(a), Reverse(b), conform)) = queue.pop() {
            // A side that must split first is tried at once, and a side of it before that
            // (longest-edge propagation, Rivara 1984); the edges that asked go back to the queue,
            // except that the one nearest a side that did not split waits.
            let mut asked = Vec::new();
            let mut next = Some((a, b, conform));
            while let Some((a, b, conform)) = next.take() {
                tries += 1;
                let most = MOST_TRIES * (edges.len() as u64 + u64::from(self.ops) + 1);
                debug_assert!(tries <= most, "the split step stopped making progress");
                self.hit |= tries > most;
                if self.hit {
                    return;
                }
                match self.try_split(a, b, conform, &mut queue, out) {
                    Tried::First(u, w) => {
                        asked.push((a, b, conform));
                        next = Some((u, w, true));
                    }
                    Tried::Split => {}
                    Tried::Not if self.hit => {}
                    Tried::Not => {
                        if let Some((a, b, _)) = asked.pop() {
                            self.wait(a, b, out);
                        }
                    }
                }
            }
            for (a, b, conform) in asked {
                self.push(&mut queue, a, b, conform);
            }
        }
    }

    /// Try to split edge a-b with its image (with `conform`, to make way for a shorter edge
    /// beside it even if it is not long), first splitting the quads it borders.
    fn try_split(
        &mut self,
        a: VertId,
        b: VertId,
        conform: bool,
        queue: &mut Queue,
        out: &mut LiveReport,
    ) -> Tried {
        // With a map, the edge's image splits with it; an edge that is its own image splits
        // alone, at a point on the plane.
        let image = self.mirrored(a, b);
        let Some(fs) = self.splittable(a, b) else {
            return Tried::Not;
        };
        let fs2 = match image.map(|(c, d)| self.splittable(c, d)) {
            Some(None) => return Tried::Not,
            Some(Some(fs2)) => fs2,
            None => Vec::new(),
        };
        if !(self.long_pair(a, b) || conform && self.free_pair(a, b)) {
            return Tried::Not;
        }
        if fs.iter().chain(&fs2).any(|f| self.waiting.contains(f)) {
            out.waits += 1;
            return Tried::Not;
        }
        let mut ok = true;
        let quads: Vec<FaceId> = (fs.into_iter())
            .filter(|&f| self.ed.face(f)[3] != NO)
            .collect();
        for f in quads {
            // A quad's image may have been one of them, split already with it.
            if ok && self.ed.face(f)[3] != NO {
                ok = self.split_quad(f, out, queue);
            }
        }
        if self.hit {
            return Tried::Not;
        }
        let Some(e) = self.ed.edge(a, b).filter(|_| ok) else {
            return Tried::Not;
        };
        let e2 = match image.map(|(c, d)| self.ed.edge(c, d)) {
            Some(None) => return Tried::Not,
            Some(e2) => e2,
            None => None,
        };
        self.split_pair(e, e2, queue, out)
    }

    /// Edge a-b and its image wait for a later dab, and so does every edge of their faces.
    fn wait(&mut self, a: VertId, b: VertId, out: &mut LiveReport) {
        out.waits += 1;
        for (u, w) in [Some((a, b)), self.mirrored(a, b)].into_iter().flatten() {
            let fs = self.ed.edge_faces(u, w);
            self.waiting.extend(fs);
        }
    }

    /// Split edge `e` and its image `e2`, both still splittable and their quads split, unless
    /// a side must split first (returned, for a conforming split) or there is no room.
    fn split_pair(
        &mut self,
        e: Edge,
        e2: Option<Edge>,
        queue: &mut Queue,
        out: &mut LiveReport,
    ) -> Tried {
        // A triangle straddling the plane holds both edges: its crossing side splits first,
        // whatever its length, which leaves them in triangles of their own. Otherwise a longer
        // side of the edge's triangles does.
        let first = match e2.and_then(|e2| shared_face(&e, &e2)) {
            Some(t) => {
                let Some(side) = self.crossing(t) else {
                    out.waits += 1;
                    return Tried::Not;
                };
                Some(side)
            }
            None => self.longer_side(&e),
        };
        if let Some((u, w)) = first {
            let image_ok = self
                .mirrored(u, w)
                .is_none_or(|(x, y)| self.can_conform(x, y));
            if self.can_conform(u, w) && image_ok {
                return Tried::First(u, w);
            }
            self.wait(e.a, e.b, out);
            return Tried::Not;
        }
        let Some((n, n2)) = self.rooms(&e, e2.as_ref()) else {
            self.wait(e.a, e.b, out);
            return Tried::Not;
        };
        // Both new points are placed from the mesh as it was, before either split; an edge
        // that is its own image puts its point on the plane.
        let mut p = self.midpoint(&e);
        if let Some((axis, _)) = self.mirror.filter(|_| e2.is_none()) {
            p = mirror::onto(p, axis);
        }
        let p2 = e2.map(|e2| self.midpoint(&e2));
        if !self.spend(1 + u32::from(e2.is_some())) {
            return Tried::Not;
        }
        let (m, mut next) = self.split(e, p, n);
        out.splits += 1;
        out.added += 1;
        if let Some((_, col)) = self.mirror {
            let mut m2 = m;
            if let (Some(e2), Some(p2)) = (e2, p2) {
                let (x, more) = self.split(e2, p2, n2);
                (m2, next) = (x, [next, more].concat());
                out.splits += 1;
                out.added += 1;
            }
            self.ed.set_partner(col, m, m2);
            self.ed.set_partner(col, m2, m);
        }
        for (u, w) in next {
            self.push(queue, u, w, false);
        }
        Tried::Split
    }

    /// The faces of edge a-b when it may split: both ends live, one or two faces, none hidden
    /// and none without area. Splitting a face without area only makes more, as its new point
    /// lies on the same line (review 0006, S3); the collapse step removes those with two
    /// corners at one point.
    fn splittable(&self, a: VertId, b: VertId) -> Option<Vec<FaceId>> {
        if !(self.ed.is_live(a) && self.ed.is_live(b)) {
            return None;
        }
        let fs = self.ed.edge_faces(a, b);
        let ok =
            !fs.is_empty() && fs.len() <= 2 && !fs.iter().any(|&f| self.hidden(f) || self.flat(f));
        ok.then_some(fs)
    }

    /// Split `e` at `p` with the new point in node `n`. Returns the new point and the edges to
    /// queue: the long edges it made and the neighbour edges much longer than `e` was. They
    /// are queued once its partner is set, which their keys read.
    fn split(&mut self, e: Edge, p: Vec3, n: NodeId) -> (VertId, Vec<(VertId, VertId)>) {
        let wings = self.ed.wings(&e);
        let c = self.ed.far_corner(e.f, e.a, e.b);
        let len2 = self.len2(e.a, e.b);
        let (a, b) = (e.a, e.b);
        let m = self.ed.split_at(e, p, n);
        self.changed.push(m);
        self.made.push(m);
        let mut next = Vec::new();
        let far = [Some(c), wings.map(|x| x.1)];
        for u in [Some(a), Some(b)].into_iter().chain(far).flatten() {
            if self.is_long(m, u) && self.near(m, u) {
                next.push((m, u));
            }
        }
        let longer = NEIGHBOUR_SPLIT * NEIGHBOUR_SPLIT * len2;
        for x in far.into_iter().flatten() {
            for u in [a, b] {
                if self.len2(x, u) > longer && self.is_long(x, u) && self.near(x, u) {
                    next.push((x, u));
                }
            }
        }
        (m, next)
    }

    fn push(&self, queue: &mut Queue, a: VertId, b: VertId, conform: bool) {
        let (k, lo, hi) = self.key(a, b);
        queue.push((k, Reverse(lo), Reverse(hi), conform));
    }

    /// The longest side of `e`'s triangles when it is longer than `e` (by the length in its
    /// key; a side only as long is not, so on a tie `e` splits itself). Splitting `e` first
    /// would cut a thin triangle against it, so that side splits first (longest-edge bisection,
    /// Rivara 1984). An edge crossing the plane looks at no triangle straddling it: the pair
    /// there asks it to split first, so it never asks them.
    fn longer_side(&self, e: &Edge) -> Option<(VertId, VertId)> {
        let own = self.key(e.a, e.b);
        let crosses = self.mirror.is_some() && self.twin(e.a) == e.b;
        let straddles = |f: FaceId| {
            let c = self.ed.far_corner(f, e.a, e.b);
            self.twin(c) == c
        };
        let faces = [Some(e.f), e.g].into_iter().flatten();
        let sides = (faces.filter(|&f| !(crosses && straddles(f)))).flat_map(|f| {
            let fc = self.ed.face(f);
            [(fc[0], fc[1]), (fc[1], fc[2]), (fc[2], fc[0])]
        });
        let longest = sides.map(|(u, w)| self.key(u, w)).max()?;
        (longest.0 > own.0).then_some((longest.1, longest.2))
    }

    /// Whether edge u-w may split to make way: its faces visible and not waiting, and not both
    /// ends fully masked. It may lie outside the pass spheres: the chain of ever longer edges
    /// ends where it meets an edge longest in both its triangles, and it only adds points.
    fn can_conform(&self, u: VertId, w: VertId) -> bool {
        if self.free(u, w) <= 0.0 {
            return false;
        }
        let fs = self.ed.edge_faces(u, w);
        !fs.is_empty()
            && fs.len() <= 2
            && (fs.iter()).all(|f| !self.hidden(*f) && !self.waiting.contains(f))
    }

    /// Split every quad of `faces` no edge split reached, as a whole-part pass does so its
    /// faces come out triangles; flips look at their corners. A new diagonal that is long
    /// waits for the next pass, whose region holds it.
    pub(super) fn split_quads(&mut self, faces: &[FaceId], out: &mut LiveReport) {
        let mut queue: Queue = BinaryHeap::new();
        for &f in faces {
            let q = self.ed.face(f);
            if self.hit {
                return;
            }
            if q[0] != NO && q[3] != NO && self.split_quad(f, out, &mut queue) {
                self.changed.extend(q);
            }
        }
    }

    /// Split quad `f` before an edge of it splits, with its mirror image along the image of
    /// the same diagonal; a quad that is its own image splits alone when it has a corner on the
    /// plane, and never otherwise. The new diagonals join the queue when long. Returns false if
    /// it could not (budget, room, or no diagonal that keeps the map).
    fn split_quad(&mut self, f: FaceId, out: &mut LiveReport, queue: &mut Queue) -> bool {
        let image = match self.mirror.map(|_| self.mirror_face(f)) {
            Some(None) => return false,
            Some(Some(g)) if g == f && !self.splits_alone(f) => return false,
            Some(Some(g)) if g != f => Some(g),
            _ => None,
        };
        let cfg = self.ed.s.cfg;
        let n = cfg.face_node(f);
        let room = match image.map(|g| cfg.face_node(g)) {
            Some(m) if m == n => self.ed.room(n)[1] >= 2,
            Some(m) => self.ed.room(n)[1] >= 1 && self.ed.room(m)[1] >= 1,
            None => self.ed.room(n)[1] >= 1,
        };
        if !room {
            out.waits += 1;
            return false;
        }
        if !self.spend(1 + u32::from(image.is_some())) {
            return false;
        }
        let Some([u, w]) = self.quad_split(f, None, queue) else {
            return false;
        };
        out.quad_splits += 1;
        if let Some(g) = image {
            let diag = [self.twin(u), self.twin(w)];
            if self.quad_split(g, Some(diag), queue).is_none() {
                return false;
            }
            out.quad_splits += 1;
        }
        true
    }

    /// Split quad `f` (along `diag` if given) and queue the new diagonal when long. Returns it.
    fn quad_split(
        &mut self,
        f: FaceId,
        diag: Option<[VertId; 2]>,
        queue: &mut Queue,
    ) -> Option<[VertId; 2]> {
        let h = self.ed.split_quad(f, diag)?;
        let (t1, t2) = (self.ed.face(f), self.ed.face(h));
        let shared: Vec<VertId> = (t1[..3].iter().copied())
            .filter(|u| t2[..3].contains(u))
            .collect();
        let [u, w] = shared[..] else { return None };
        if self.is_long(u, w) && self.near(u, w) {
            self.push(queue, u, w, false);
        }
        Some([u, w])
    }

    /// Where a split of edge `e` (far corners c and `d`) puts its new point: the Hermite
    /// midpoint `(a + b) / 2 + (t_a - t_b) / 8`, `t` the edge projected onto each end's
    /// tangent plane, or the plain midpoint when that is off or the curved point would lean one
    /// of the four new triangles away from its parent (next to a thin triangle).
    fn midpoint(&self, e: &Edge) -> Vec3 {
        let d = self.ed.wings(e).map(|x| x.1);
        let (a, b) = (e.a, e.b);
        let (pa, pb) = (self.ed.pos(a), self.ed.pos(b));
        let mid = (pa + pb) * 0.5;
        if !self.ld.smooth_split {
            return mid;
        }
        let v = pb - pa;
        let tangent = |n: Vec3| v - n * n.dot(v);
        let (ta, tb) = (
            tangent(self.ed.normal_now(a)),
            tangent(self.ed.normal_now(b)),
        );
        let p = mid + (ta - tb) * 0.125;
        let tri = |x: Vec3, y: Vec3, z: Vec3| (y - x).cross(z - x);
        let c = self.ed.pos(self.ed.far_corner(e.f, a, b));
        let parent = tri(pa, pb, c);
        let mut keeps = !leans(parent, tri(pa, p, c)) && !leans(parent, tri(p, pb, c));
        if let Some(d) = d.map(|d| self.ed.pos(d)) {
            let parent = tri(pb, pa, d);
            keeps = keeps && !leans(parent, tri(pb, p, d)) && !leans(parent, tri(p, pa, d));
        }
        if keeps { p } else { mid }
    }
}
