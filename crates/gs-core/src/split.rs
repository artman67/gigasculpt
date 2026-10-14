//! Node splitting, the overflow policy decision 0001 kept for Live Detail: when a node has no
//! room left, cut it in two along the longest axis of its vertices. Spilling into a neighbour
//! instead let nodes overlap and made the edited region 8.8 times slower.

use crate::config::{NodeId, VertId};
use crate::math::{Aabb, NO, arity};
use crate::topo::TopoEditor;

impl TopoEditor<'_> {
    /// Cut node `x` in two. The upper half of its live vertices along their longest axis move
    /// to a new node (renumbered: every face using them is rewritten), then so does every face
    /// of `x` left with no corner in `x`. The mesh itself does not change.
    ///
    /// Returns `(old id, new id)` of every moved vertex, ascending by old id.
    pub fn split_node(&mut self, x: NodeId) -> Vec<(VertId, VertId)> {
        let cfg = self.s.cfg;
        let live: Vec<VertId> = (0..self.node_topo(x).ring.len() as u32)
            .map(|l| cfg.vert(x, l))
            .filter(|&v| !self.pos(v).is_dead())
            .collect();
        let b = live.iter().fold(Aabb::EMPTY, |mut b, &v| {
            b.grow(self.pos(v));
            b
        });
        let e = (b.hi - b.lo).to_array();
        let axis = (0..3).max_by(|&i, &j| e[i].total_cmp(&e[j])).unwrap_or(0);
        let mut order: Vec<(f32, VertId)> = live
            .iter()
            .map(|&v| (self.pos(v).to_array()[axis], v))
            .collect();
        order.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.cmp(&q.1)));
        let y = self.add_node();
        self.node_splits += 1;
        let mut moved: Vec<(VertId, VertId)> = (order[order.len() / 2..].iter())
            .map(|&(_, v)| {
                let w = self.alloc_vert(y);
                self.relocate_vert(v, w);
                self.free_vert(v);
                (v, w)
            })
            .collect();
        moved.sort_unstable();
        let faces = self.node_topo(x).faces.clone();
        for (l, fc) in faces.iter().enumerate() {
            let corners = &fc[..arity(*fc)];
            if fc[0] != NO && corners.iter().all(|&u| cfg.node_of(u) != x) {
                let (f, g) = (cfg.face(x, l as u32), self.alloc_face(y));
                self.relocate_face(f, g);
                self.free_face(f);
            }
        }
        moved
    }
}
