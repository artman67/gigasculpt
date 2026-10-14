//! Edge flips and quad splits, Live Detail's edits that change connectivity but no position
//! (decision 0021, section 2, steps 5 and 6).

use crate::config::{FaceId, VertId};
use crate::math::NO;
use crate::topo::{Edge, TopoEditor, next};
use gs_protocol::quad_diagonal;

impl TopoEditor<'_> {
    /// The far corners of a two-triangle edge: `c` of the triangle running a then b, `d` of
    /// the other.
    pub(crate) fn wings(&self, e: &Edge) -> Option<(u32, u32)> {
        let g = e.g?;
        Some((next(self.face(e.f), e.b), next(self.face(g), e.a)))
    }

    /// Turn edge `e` (two triangles a-b-c and b-a-d) to join their far corners: the triangles
    /// become a-d-c and b-c-d, in the same face slots.
    pub(crate) fn flip(&mut self, e: Edge) {
        let (Some(g), Some((c, d))) = (e.g, self.wings(&e)) else {
            return;
        };
        let (a, b, f) = (e.a, e.b, e.f);
        self.set_face(f, [a, d, c, NO]);
        self.set_face(g, [b, c, d, NO]);
        self.remove_from_ring(a, g);
        self.remove_from_ring(b, f);
        self.add_to_ring(c, g);
        self.add_to_ring(d, f);
    }

    /// Split quad `f` into two triangles along diagonal `diag`, or without one along the
    /// diagonal it is drawn split along ([`quad_diagonal`], decision 0040 section 2). The second
    /// triangle takes a new face slot in `f`'s node and `f`'s attributes. Returns it, or `None`
    /// if the node has no face slot left or `diag` is not a diagonal of `f`.
    pub(crate) fn split_quad(&mut self, f: FaceId, diag: Option<[VertId; 2]>) -> Option<FaceId> {
        let q = self.face(f);
        let n = self.s.cfg.face_node(f);
        if q[3] == NO || q[0] == NO || self.room(n)[1] == 0 {
            return None;
        }
        let across_ac = match diag {
            Some([u, w]) if [u, w] == [q[0], q[2]] || [w, u] == [q[0], q[2]] => true,
            Some([u, w]) if [u, w] == [q[1], q[3]] || [w, u] == [q[1], q[3]] => false,
            Some(_) => return None,
            None => quad_diagonal(q.map(|v| self.pos(v).to_array())) == 0,
        };
        let (t1, t2, alone, both) = if across_ac {
            (
                [q[0], q[1], q[2], NO],
                [q[0], q[2], q[3], NO],
                q[3],
                [q[0], q[2]],
            )
        } else {
            (
                [q[0], q[1], q[3], NO],
                [q[1], q[2], q[3], NO],
                q[2],
                [q[1], q[3]],
            )
        };
        let h = self.alloc_face(n);
        self.copy_face_attrs(f, h);
        self.set_face(f, t1);
        self.set_face(h, t2);
        self.replace_in_ring(alone, f, h);
        for v in both {
            self.add_to_ring(v, h);
        }
        Some(h)
    }
}
