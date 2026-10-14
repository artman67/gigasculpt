//! Local topology edits on the same columns: the foundation of Live Detail (ARCHITECTURE 3.3,
//! decision 0021).
//!
//! Every setter writes through `Arc::make_mut`, so an open history step records it. Storage
//! stays node-contiguous: a new vertex takes a slot in the node of an existing endpoint, freed
//! slots are tombstoned (NaN position, empty ring, `[NO; 4]` face) and reused last in, first
//! out, and a node that runs out of slots is cut in two (`split.rs`), the policy decision 0001
//! kept. A refinement pass holds freed slots until it ends ([`TopoEditor::hold`]), so no slot is
//! reused within it.
//!
//! When the editor drops, only what the edits can have changed is brought up to date (decision
//! 0021, section 6): the bounds of the nodes whose own columns were written; their seam and
//! shared lists and the rims around them, judged again only at the vertices whose rings or
//! faces were rewritten (`MeshStore::relist`); the halos around them; the normals of the corners of every face written or moved;
//! the tree refit, or rebuilt when a node was added, emptied or reused. A node whose columns were not written is
//! never copied. Dirty marks go to the nodes written, those whose normals turned, and those
//! storing a face around a moved point, whose drawn faces moved.
//!
//! Every face is stored in a node owning one of its corners, as a store is built (with its
//! lowest corner): a seam walk then finds every corner of a node's faces, so halos, shells and
//! face brushes reach every face. An edit that changes a face's corners can leave it in a node
//! owning none of them (a collapse renaming the corner the node owned, a split or flip reusing
//! the face's slot for new corners); the editor's close moves such a face to the node of its
//! lowest corner with a free face slot, cutting that corner's node first when none has one.

use std::sync::Arc;

use crate::columns::{AttrValue, AttrVec, ColumnSet, Domain, NodeTopo};
use crate::config::{FaceId, NodeId, VertId};
use crate::derived::neighbours;
use crate::math::{NO, Vec3, arity};
use crate::normals::vertex_normal;
use crate::par::{merge, union};
use crate::store::MeshStore;
use crate::view::{PosReader, TopoReader};

/// Local topology edits. See the module docs.
pub struct TopoEditor<'a> {
    pub(crate) s: &'a mut MeshStore,
    /// Nodes whose own columns were written.
    written: Vec<bool>,
    /// Nodes owning a corner of a face written: their halos and normals can change, their
    /// columns need not.
    around: Vec<bool>,
    /// Vertices whose normal can have changed: every corner of a face written, and every vertex
    /// whose ring was rewritten.
    stale: Vec<VertId>,
    /// Vertices moved: the corners of every face around them have stale normals.
    moved: Vec<VertId>,
    /// Faces written into a node owning none of their corners: the close moves those still so.
    homeless: Vec<FaceId>,
    /// Slots freed while held: released to the free lists by [`TopoEditor::release`].
    held: Option<(Vec<VertId>, Vec<FaceId>)>,
    /// Node count when the editor opened.
    start_nodes: usize,
    /// A node was emptied or reused: the tree is rebuilt, not refit, though the count holds.
    rebuild: bool,
    pub(crate) node_splits: usize,
    /// The mirror map columns the store holds (`mirror.rs`): partners are rewritten wherever
    /// a point moves.
    pub(crate) maps: Vec<usize>,
}

impl MeshStore {
    /// Start local topology edits.
    pub fn edit_topology(&mut self) -> TopoEditor<'_> {
        let n = self.node_count();
        let maps = self.mirror_maps().into_iter().map(|m| m.1).collect();
        TopoEditor {
            s: self,
            written: vec![false; n],
            around: vec![false; n],
            stale: Vec::new(),
            moved: Vec::new(),
            homeless: Vec::new(),
            held: None,
            start_nodes: n,
            rebuild: false,
            node_splits: 0,
            maps,
        }
    }
}

/// The vertex after `v` in face `f` (a triangle or a quad).
pub(crate) fn next(f: [u32; 4], v: VertId) -> VertId {
    around(f, v, 1)
}

/// The vertex before `v` in face `f`.
pub(crate) fn prev(f: [u32; 4], v: VertId) -> VertId {
    around(f, v, arity(f) - 1)
}

/// The vertex `by` corners after `v` in face `f`.
fn around(f: [u32; 4], v: VertId, by: usize) -> VertId {
    let k = arity(f);
    let j = f[..k].iter().position(|&u| u == v).unwrap_or(0);
    f[(j + by) % k]
}

/// `Arc::make_mut`, counting the copy it makes when the column is shared (with an open history
/// step, a share or a stroke-start view).
pub(crate) fn cow<'a, T: Clone>(a: &'a mut Arc<T>, copies: &mut u64) -> &'a mut T {
    if Arc::get_mut(a).is_none() {
        *copies += 1;
    }
    Arc::make_mut(a)
}

/// One edge between two triangles, or a triangle and the open border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edge {
    pub a: VertId,
    pub b: VertId,
    /// The triangle running a then b.
    pub f: FaceId,
    /// The triangle running b then a; `None` on an open border.
    pub g: Option<FaceId>,
}

fn flag(v: &mut Vec<bool>, k: usize) {
    if k >= v.len() {
        v.resize(k + 1, false);
    }
    v[k] = true;
}

fn listed(v: &[bool]) -> Vec<NodeId> {
    (v.iter().enumerate())
        .filter(|x| *x.1)
        .map(|x| x.0 as NodeId)
        .collect()
}

impl TopoEditor<'_> {
    /// Nodes cut in two so far.
    pub fn node_splits(&self) -> usize {
        self.node_splits
    }

    /// Nodes whose own columns were written so far, ascending.
    pub(crate) fn written(&self) -> Vec<NodeId> {
        listed(&self.written)
    }

    /// Node `n`'s own columns are being written.
    pub(crate) fn mark(&mut self, n: NodeId) {
        let k = n as usize;
        assert!(k < self.s.node_count(), "node {n} does not exist");
        flag(&mut self.written, k);
        flag(&mut self.around, k);
    }
    pub(crate) fn node_topo(&self, n: NodeId) -> &NodeTopo {
        &self.s.cols.topo[n as usize]
    }
    fn split_id(&self, v: VertId) -> (usize, usize) {
        let cfg = self.s.cfg;
        (cfg.node_of(v) as usize, cfg.local_of(v) as usize)
    }
    pub(crate) fn pos(&self, v: VertId) -> Vec3 {
        let (n, l) = self.split_id(v);
        self.s.cols.pos[n][l]
    }
    pub(crate) fn face(&self, f: FaceId) -> [u32; 4] {
        let cfg = self.s.cfg;
        self.node_topo(cfg.face_node(f)).faces[cfg.face_local(f) as usize]
    }
    pub(crate) fn ring(&self, v: VertId) -> Vec<FaceId> {
        self.ring_ref(v).to_vec()
    }
    pub(crate) fn ring_ref(&self, v: VertId) -> &[FaceId] {
        let (n, l) = self.split_id(v);
        self.s.cols.topo[n].ring_of(l)
    }

    fn topo_mut(&mut self, n: usize) -> &mut NodeTopo {
        cow(&mut self.s.cols.topo[n], &mut self.s.work.copies)
    }

    pub(crate) fn set_pos(&mut self, v: VertId, p: Vec3) {
        let (n, l) = self.split_id(v);
        self.mark(n as u32);
        self.moved.push(v);
        cow(&mut self.s.cols.pos[n], &mut self.s.work.copies)[l] = p;
    }
    /// Rings are kept sorted by face id: that order makes normal sums canonical.
    pub(crate) fn set_ring(&mut self, v: VertId, r: &[FaceId]) {
        let (n, l) = self.split_id(v);
        self.mark(n as u32);
        self.stale.push(v);
        let mut r = r.to_vec();
        r.sort_unstable();
        self.topo_mut(n).set_ring(l, &r);
    }
    /// Write a face. The owners of its old and new corners are noted: their halos and the
    /// normals of those corners can change.
    pub(crate) fn set_face(&mut self, f: FaceId, x: [u32; 4]) {
        let cfg = self.s.cfg;
        let (n, l) = (cfg.face_node(f), cfg.face_local(f) as usize);
        for u in self.face(f).into_iter().chain(x).filter(|&u| u != NO) {
            flag(&mut self.around, cfg.node_of(u) as usize);
            self.stale.push(u);
        }
        self.mark(n);
        if x[0] != NO && x[..arity(x)].iter().all(|&u| cfg.node_of(u) != n) {
            self.homeless.push(f);
        }
        self.topo_mut(n as usize).faces[l] = x;
    }

    /// Value of attribute `a` at slot `slot` of node `n` (the kind's empty value when
    /// unallocated).
    pub(crate) fn attr(&self, a: usize, n: usize, slot: usize) -> AttrValue {
        match &self.s.cols.attrs[a][n] {
            Some(v) => v.get(slot),
            None => AttrValue::zero(self.s.defs[a].kind),
        }
    }
    /// Write attribute `a` at slot `slot` of node `n`, allocating the node's column unless the
    /// value is the kind's empty value. Writing the value already there writes nothing.
    pub(crate) fn set_attr(&mut self, a: usize, n: usize, slot: usize, x: AttrValue) {
        let d = &self.s.defs[a];
        let len = match d.domain {
            Domain::Vertex => self.s.cols.topo[n].ring.len(),
            Domain::Face => self.s.cols.topo[n].faces.len(),
        };
        let kind = d.kind;
        let col = &mut self.s.cols.attrs[a][n];
        let word = match x {
            AttrValue::F32(f) => f.to_bits(),
            AttrValue::U32(u) => u,
        };
        match col {
            None if x.is_zero(kind) => return,
            Some(v) if v.bits(slot) == word => return,
            _ => {}
        }
        let copies = &mut self.s.work.copies;
        let col = col.get_or_insert_with(|| Arc::new(AttrVec::zeros(kind, len)));
        cow(col, copies).set(slot, x);
        self.mark(n as u32);
    }
    /// Indices of the attributes on `domain`.
    pub(crate) fn attrs_on(&self, domain: Domain) -> Vec<usize> {
        (0..self.s.defs.len())
            .filter(|&a| self.s.defs[a].domain == domain)
            .collect()
    }

    /// An empty node: the lowest one holding no slot, else a new one appended.
    pub(crate) fn add_node(&mut self) -> NodeId {
        let topo = &self.s.cols.topo;
        let empty = (0..topo.len()).find(|&k| topo[k].ring.is_empty() && topo[k].faces.is_empty());
        if let Some(k) = empty {
            self.rebuild_tree();
            self.mark(k as NodeId);
            return k as NodeId;
        }
        let c = &mut self.s.cols;
        c.pos.push(Arc::default());
        c.nrm.push(Arc::default());
        c.topo.push(Arc::default());
        for a in &mut c.attrs {
            a.push(None);
        }
        let n = (c.len() - 1) as NodeId;
        self.mark(n);
        n
    }

    /// Rebuild the tree when the editor drops: a node was emptied or reused, which moves its
    /// box without changing the node count.
    pub(crate) fn rebuild_tree(&mut self) {
        self.rebuild = true;
    }

    /// Free vertex and face slots node `n` can still hand out.
    pub(crate) fn room(&self, n: NodeId) -> [usize; 2] {
        let (t, cfg) = (self.node_topo(n), self.s.cfg);
        [
            t.vfree.len() + cfg.capacity() as usize - t.ring.len(),
            t.ffree.len() + cfg.face_capacity() as usize - t.faces.len(),
        ]
    }

    /// Whether node `n` has `verts` free vertex slots and `faces` free face slots.
    pub(crate) fn has_room(&self, n: NodeId, verts: u32, faces: u32) -> bool {
        let [v, f] = self.room(n);
        v >= verts as usize && f >= faces as usize
    }

    /// A vertex slot in node `n` (which must have room), dead until written.
    pub(crate) fn alloc_vert(&mut self, n: NodeId) -> VertId {
        let k = n as usize;
        self.mark(n);
        let vertex_attrs = self.attrs_on(Domain::Vertex);
        let copies = &mut self.s.work.copies;
        let t = cow(&mut self.s.cols.topo[k], copies);
        let l = t.vfree.pop().unwrap_or_else(|| {
            t.ring.push(0);
            cow(&mut self.s.cols.pos[k], copies).push(Vec3::DEAD);
            cow(&mut self.s.cols.nrm[k], copies).push(Vec3::ZERO);
            for &a in &vertex_attrs {
                if let Some(v) = self.s.cols.attrs[a][k].as_mut() {
                    let v = cow(v, copies);
                    v.resize(v.len() + 1, self.s.defs[a].kind);
                }
            }
            (t.ring.len() - 1) as u32
        });
        assert!(
            l < self.s.cfg.capacity(),
            "node {n} has no free vertex slot"
        );
        self.s.cfg.vert(n, l)
    }

    /// A face slot in node `n` (which must have room), dead until written.
    pub(crate) fn alloc_face(&mut self, n: NodeId) -> FaceId {
        let k = n as usize;
        self.mark(n);
        let face_attrs = self.attrs_on(Domain::Face);
        let copies = &mut self.s.work.copies;
        let t = cow(&mut self.s.cols.topo[k], copies);
        let l = t.ffree.pop().unwrap_or_else(|| {
            t.faces.push([NO; 4]);
            for &a in &face_attrs {
                if let Some(v) = self.s.cols.attrs[a][k].as_mut() {
                    let v = cow(v, copies);
                    v.resize(v.len() + 1, self.s.defs[a].kind);
                }
            }
            (t.faces.len() - 1) as u32
        });
        assert!(
            l < self.s.cfg.face_capacity(),
            "node {n} has no free face slot"
        );
        self.s.cfg.face(n, l)
    }

    /// Hold freed slots until [`TopoEditor::release`]: within a refinement pass no slot is
    /// reused, so an id the pass has queued never comes to name something else.
    pub(crate) fn hold(&mut self) {
        self.held.get_or_insert_default();
    }

    /// Hand every held slot to its node's free list, in the order freed.
    pub(crate) fn release(&mut self) {
        let Some((verts, faces)) = self.held.take() else {
            return;
        };
        for v in verts {
            let (n, l) = self.split_id(v);
            self.topo_mut(n).vfree.push(l as u32);
        }
        for f in faces {
            let cfg = self.s.cfg;
            let (n, l) = (cfg.face_node(f) as usize, cfg.face_local(f));
            self.topo_mut(n).ffree.push(l);
        }
    }

    /// Tombstone vertex `v`: its attributes zeroed, its slot freed (or held).
    pub(crate) fn free_vert(&mut self, v: VertId) {
        self.set_pos(v, Vec3::DEAD);
        self.set_ring(v, &[]);
        let (n, l) = self.split_id(v);
        for a in self.attrs_on(Domain::Vertex) {
            self.set_attr(a, n, l, AttrValue::zero(self.s.defs[a].kind));
        }
        match &mut self.held {
            Some(h) => h.0.push(v),
            None => self.topo_mut(n).vfree.push(l as u32),
        }
    }

    /// Tombstone face `f`: its attributes zeroed, its slot freed (or held).
    pub(crate) fn free_face(&mut self, f: FaceId) {
        let cfg = self.s.cfg;
        self.set_face(f, [NO; 4]);
        let (n, l) = (cfg.face_node(f) as usize, cfg.face_local(f) as usize);
        for a in self.attrs_on(Domain::Face) {
            self.set_attr(a, n, l, AttrValue::zero(self.s.defs[a].kind));
        }
        match &mut self.held {
            Some(h) => h.1.push(f),
            None => self.topo_mut(n).ffree.push(l as u32),
        }
    }

    /// Whether `v` names a live vertex.
    pub fn is_live(&self, v: VertId) -> bool {
        let (n, l) = self.split_id(v);
        n < self.s.node_count() && l < self.s.cols.pos[n].len() && !self.s.cols.pos[n][l].is_dead()
    }

    /// Faces of edge a-b: those around `a` where `b` is the next or previous corner.
    pub(crate) fn edge_faces(&self, a: VertId, b: VertId) -> Vec<FaceId> {
        (self.ring_ref(a).iter().copied())
            .filter(|&f| {
                let fc = self.face(f);
                next(fc, a) == b || prev(fc, a) == b
            })
            .collect()
    }

    /// Edge a-b when one or two triangles (no quad) share it and both ends are live, with `a`
    /// and `b` swapped if needed so the first triangle runs a then b.
    pub(crate) fn edge(&self, a: VertId, b: VertId) -> Option<Edge> {
        if a == b || !self.is_live(a) || !self.is_live(b) {
            return None;
        }
        let fs = self.edge_faces(a, b);
        if fs.is_empty() || fs.len() > 2 || fs.iter().any(|&f| self.face(f)[3] != NO) {
            return None;
        }
        let forward = |f: FaceId| next(self.face(f), a) == b;
        match *fs.as_slice() {
            [f] if forward(f) => Some(Edge { a, b, f, g: None }),
            [f] => Some(Edge {
                a: b,
                b: a,
                f,
                g: None,
            }),
            [f, g] if forward(f) && !forward(g) => Some(Edge {
                a,
                b,
                f,
                g: Some(g),
            }),
            [f, g] if forward(g) && !forward(f) => Some(Edge {
                a,
                b,
                f: g,
                g: Some(f),
            }),
            _ => None,
        }
    }

    /// Split edge a-b at its midpoint into two triangles each side; attributes are blended.
    /// The new vertex and faces go in a's node, which is cut in two first if it is full.
    /// Returns the new vertex, or `None` if a-b is not an edge between two live triangles.
    pub fn split_edge(&mut self, a: VertId, b: VertId) -> Option<VertId> {
        self.edge(a, b).filter(|e| e.g.is_some())?;
        let (mut a, mut b) = (a, b);
        let cfg = self.s.cfg;
        if !self.has_room(cfg.node_of(a), 1, 2) {
            let moved = self.split_node(cfg.node_of(a));
            let rename = |v: VertId| {
                moved
                    .binary_search_by_key(&v, |m| m.0)
                    .map_or(v, |i| moved[i].1)
            };
            (a, b) = (rename(a), rename(b));
            if !self.has_room(cfg.node_of(a), 1, 2) {
                return None;
            }
        }
        let e = self.edge(a, b)?;
        let mid = (self.pos(a) + self.pos(b)) * 0.5;
        Some(self.split_at(e, mid, cfg.node_of(a)))
    }

    /// Split edge `e` with a new vertex at `p` in node `n` (which must have a vertex slot and
    /// two face slots): each triangle of the edge becomes two. The new vertex blends both ends'
    /// attributes; new faces copy their parent's.
    pub(crate) fn split_at(&mut self, e: Edge, p: Vec3, n: NodeId) -> VertId {
        let Edge { a, b, f, g } = e;
        let c = next(self.face(f), b);
        let m = self.alloc_vert(n);
        self.set_pos(m, p);
        let ((an, al), (bn, bl), (mn, ml)) = (self.split_id(a), self.split_id(b), self.split_id(m));
        for at in self.attrs_on(Domain::Vertex) {
            let vals = [self.attr(at, an, al), self.attr(at, bn, bl)];
            self.set_attr(at, mn, ml, AttrValue::blend(&self.s.defs[at], &vals));
        }
        let f3 = self.alloc_face(n);
        self.copy_face_attrs(f, f3);
        self.set_face(f, [a, m, c, NO]);
        self.set_face(f3, [m, b, c, NO]);
        self.replace_in_ring(b, f, f3);
        self.add_to_ring(c, f3);
        let Some(g) = g else {
            self.set_ring(m, &[f, f3]);
            return m;
        };
        let d = next(self.face(g), a);
        let f4 = self.alloc_face(n);
        self.copy_face_attrs(g, f4);
        self.set_face(g, [b, m, d, NO]);
        self.set_face(f4, [m, a, d, NO]);
        self.replace_in_ring(a, g, f4);
        self.add_to_ring(d, f4);
        self.set_ring(m, &[f, f3, g, f4]);
        m
    }

    pub(crate) fn replace_in_ring(&mut self, v: VertId, old: FaceId, new: FaceId) {
        let r: Vec<FaceId> = (self.ring_ref(v).iter())
            .map(|&f| if f == old { new } else { f })
            .collect();
        self.set_ring(v, &r);
    }
    pub(crate) fn add_to_ring(&mut self, v: VertId, f: FaceId) {
        let mut r = self.ring(v);
        r.push(f);
        self.set_ring(v, &r);
    }
    pub(crate) fn remove_from_ring(&mut self, v: VertId, f: FaceId) {
        let mut r = self.ring(v);
        r.retain(|&x| x != f);
        self.set_ring(v, &r);
    }

    pub(crate) fn copy_face_attrs(&mut self, from: FaceId, to: FaceId) {
        let cfg = self.s.cfg;
        let (fn_, fl) = (cfg.face_node(from) as usize, cfg.face_local(from) as usize);
        let (tn, tl) = (cfg.face_node(to) as usize, cfg.face_local(to) as usize);
        for a in self.attrs_on(Domain::Face) {
            self.set_attr(a, tn, tl, self.attr(a, fn_, fl));
        }
    }

    /// Copy point `v` into the dead slot `w`: position, attributes and ring, and rewrite every
    /// face and mirror partner naming `v` to name `w`. `v` is left for the caller to free or
    /// drop.
    pub(crate) fn relocate_vert(&mut self, v: VertId, w: VertId) {
        let (vn, vl) = self.split_id(v);
        let (wn, wl) = self.split_id(w);
        self.set_pos(w, self.pos(v));
        for a in self.attrs_on(Domain::Vertex) {
            self.set_attr(a, wn, wl, self.attr(a, vn, vl));
        }
        let ring = self.ring(v);
        for &f in &ring {
            let fc = self.face(f).map(|u| if u == v { w } else { u });
            self.set_face(f, fc);
        }
        self.set_ring(w, &ring);
        self.mirror_moved(v, w);
    }

    /// Copy face `f` into the dead slot `g`: corners and attributes, and rewrite the rings of
    /// its corners to name `g`. `f` is left for the caller to free or drop.
    pub(crate) fn relocate_face(&mut self, f: FaceId, g: FaceId) {
        let fc = self.face(f);
        self.copy_face_attrs(f, g);
        self.set_face(g, fc);
        for &u in &fc[..arity(fc)] {
            self.replace_in_ring(u, f, g);
        }
    }

    /// Move every face written into a node owning none of its corners to the node of its lowest
    /// corner with a free face slot, cutting the lowest corner's node first when none has one
    /// (see the module docs). A cut rewrites faces, which are checked in turn. Like
    /// [`Self::split_edge`], one cut is all it tries: a face still without room stays put, and
    /// the store checks in the tests would report it.
    fn rehome(&mut self) {
        let cfg = self.s.cfg;
        let home = |ed: &Self, fc: [u32; 4]| {
            let mut corners = fc[..arity(fc)].to_vec();
            corners.sort_unstable();
            let t = (corners.iter())
                .map(|&u| cfg.node_of(u))
                .find(|&t| ed.has_room(t, 0, 1));
            (corners[0], t)
        };
        while !self.homeless.is_empty() {
            let mut faces = std::mem::take(&mut self.homeless);
            faces.sort_unstable();
            faces.dedup();
            for f in faces {
                // Slots a compaction dropped no longer exist.
                let m = cfg.face_node(f);
                let slots = (self.s.cols.topo.get(m as usize)).map_or(0, |t| t.faces.len());
                if cfg.face_local(f) as usize >= slots {
                    continue;
                }
                let fc = self.face(f);
                if fc[0] == NO || fc[..arity(fc)].iter().any(|&u| cfg.node_of(u) == m) {
                    continue;
                }
                let t = match home(self, fc) {
                    (_, Some(t)) => Some(t),
                    (low, None) => {
                        self.split_node(cfg.node_of(low));
                        home(self, self.face(f)).1
                    }
                };
                if let Some(t) = t {
                    let g = self.alloc_face(t);
                    self.relocate_face(f, g);
                    self.free_face(f);
                }
            }
        }
    }

    /// Recompute the normal of every vertex of `stale` (the editor's, ascending) and of every
    /// corner of a face around a moved point, writing only nodes where a bit changed. Returns
    /// those nodes, and the nodes storing a face around a moved point, ascending. Each of those
    /// draws the point, so its box in the renderer (its own points and the corners of its faces)
    /// moved even when its columns and normals did not: like a stroke's shell, it is marked to
    /// be sent again.
    fn refresh_normals(&mut self, stale: &[VertId]) -> (Vec<NodeId>, Vec<NodeId>) {
        let cfg = self.s.cfg;
        let (mut near, mut drawn) = (Vec::new(), Vec::new());
        for &v in &std::mem::take(&mut self.moved) {
            near.push(v);
            if self.is_live(v) {
                for &f in self.ring_ref(v) {
                    let fc = self.face(f);
                    near.extend_from_slice(&fc[..arity(fc)]);
                    flag(&mut drawn, cfg.face_node(f) as usize);
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        let mut stale = union(stale, &near);
        // Slots a compaction dropped no longer exist.
        stale.retain(|&v| {
            let (n, l) = self.split_id(v);
            n < self.s.node_count() && l < self.s.cols.pos[n].len()
        });
        let s = &mut *self.s;
        let mut groups: Vec<(NodeId, &[VertId])> = Vec::new();
        for run in stale.chunk_by(|x, y| cfg.node_of(*x) == cfg.node_of(*y)) {
            groups.push((cfg.node_of(run[0]), run));
        }
        let (p, t) = (
            PosReader {
                cfg,
                cols: &s.cols.pos,
            },
            TopoReader {
                cfg,
                topo: &s.cols.topo,
            },
        );
        let fresh = gs_par::map(groups.len(), |i| {
            let (n, run) = groups[i];
            let old = &s.cols.nrm[n as usize];
            let vals: Vec<(usize, Vec3)> = (run.iter())
                .map(|&v| (cfg.local_of(v) as usize, vertex_normal(&t, &p, v)))
                .collect();
            let differs = vals.iter().any(|&(l, x)| old[l].bits() != x.bits());
            (n, differs.then_some(vals))
        });
        s.work.normals += stale.len() as u64;
        let mut written = Vec::new();
        for (n, vals) in fresh {
            let Some(vals) = vals else { continue };
            let nrm = cow(&mut s.cols.nrm[n as usize], &mut s.work.copies);
            for (l, x) in vals {
                nrm[l] = x;
            }
            written.push(n);
        }
        (written, listed(&drawn))
    }
}

impl Drop for TopoEditor<'_> {
    /// Bring everything derived from the written columns up to date (see the module docs).
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        self.release();
        self.rehome();
        let written = listed(&self.written);
        if written.is_empty() {
            return;
        }
        for &n in &written {
            let t = &mut self.s.cols.topo[n as usize];
            if t.mostly_garbage() {
                Arc::make_mut(t).compact();
            }
        }
        // Every vertex whose ring was rewritten or that is a corner of a face rewritten, sorted
        // once for both the normals and the lists.
        let mut stale = std::mem::take(&mut self.stale);
        stale.sort_unstable();
        stale.dedup();
        let (normals, drawn) = self.refresh_normals(&stale);
        let s = &mut *self.s;
        let grew = s.node_count() != self.start_nodes || self.rebuild;
        let around = listed(&self.around);
        s.refresh_bounds(&written, false);
        s.relist(&written, &around, &stale);
        let nb = neighbours(s.cfg, &s.der, &s.topo(), &written);
        let halos = merge(&merge(&written, &around), &nb);
        s.refresh_halos(&halos, grew);
        s.refresh_state(&written, ColumnSet::TOPOLOGY);
        let every = ColumnSet::POSITION | ColumnSet::NORMAL | ColumnSet::TOPOLOGY;
        s.mark(&written, every | s.attr_set());
        s.mark(&normals, ColumnSet::NORMAL);
        s.mark(&drawn, ColumnSet::NORMAL);
    }
}
