//! Read access: per-node slices (the hot path) and readers that cross node boundaries (seams).
//! Kernels, normals and tests are written against these, never against the storage itself.

use std::sync::Arc;

use crate::bits::Bits;
use crate::columns::{AttrId, AttrValue, AttrVec, Domain, NodeTopo};
use crate::config::{FaceId, NodeConfig, NodeId, VertId};
use crate::math::{Aabb, Vec3};

/// A vertex position by id, from any node.
pub trait PosRead: Sync {
    fn get(&self, v: VertId) -> Vec3;
}

/// Faces by id, and each vertex's ring: every face that uses it, sorted by face id. The sort
/// makes normal sums canonical, so the per-node fast path and the seam path agree bit for bit.
pub trait TopoRead: Sync {
    /// `[a, b, c, NO]` for a triangle.
    fn face(&self, f: FaceId) -> [u32; 4];
    fn ring(&self, v: VertId) -> &[FaceId];
}

/// The vertices sharing an edge with `v` across the faces `keep` accepts, ascending, each
/// marked 1 when only one of those faces uses that edge (a boundary edge) and 2 otherwise.
/// Walks pass the visible faces ([`crate::visibility::Visible`]), so the border of what is
/// visible is a boundary (decision 0015). Fills `out`, which is cleared first.
pub fn edge_neighbours<T: TopoRead>(
    t: &T,
    v: VertId,
    keep: impl Fn(FaceId) -> bool,
    out: &mut Vec<(VertId, u8)>,
) {
    out.clear();
    for &f in t.ring(v).iter().filter(|&&f| keep(f)) {
        let fc = t.face(f);
        let k = crate::math::arity(fc);
        if let Some(j) = fc[..k].iter().position(|&u| u == v) {
            let before = if j == 0 { k - 1 } else { j - 1 };
            let after = if j + 1 == k { 0 } else { j + 1 };
            add_neighbour(out, fc[before]);
            add_neighbour(out, fc[after]);
        }
    }
}

/// Puts `u` in its place in `out`, which stays ascending: marked 1 the first time, 2 when it
/// is already there (a second face shares the edge). A ring gives about a dozen candidates, so
/// placing each costs less than sorting them all and then removing the repeats.
#[inline]
fn add_neighbour(out: &mut Vec<(VertId, u8)>, u: VertId) {
    // From the end: a ring's faces are in id order, so its candidates tend to come ascending.
    let mut i = out.len();
    while i > 0 && out[i - 1].0 > u {
        i -= 1;
    }
    if i > 0 && out[i - 1].0 == u {
        out[i - 1].1 = 2;
        return;
    }
    out.push((u, 1));
    let s = out.as_mut_slice();
    let mut j = s.len() - 1;
    while j > i {
        s[j] = s[j - 1];
        j -= 1;
    }
    s[i] = (u, 1);
}

/// Positions of every node.
#[derive(Clone, Copy)]
pub struct PosReader<'a> {
    pub(crate) cfg: NodeConfig,
    pub(crate) cols: &'a [Arc<Vec<Vec3>>],
}

impl PosRead for PosReader<'_> {
    #[inline]
    fn get(&self, v: VertId) -> Vec3 {
        self.cols[self.cfg.node_of(v) as usize][self.cfg.local_of(v) as usize]
    }
}

/// Faces and rings of every node.
#[derive(Clone, Copy)]
pub struct TopoReader<'a> {
    pub(crate) cfg: NodeConfig,
    pub(crate) topo: &'a [Arc<NodeTopo>],
}

impl TopoRead for TopoReader<'_> {
    #[inline]
    fn face(&self, f: FaceId) -> [u32; 4] {
        self.topo[self.cfg.face_node(f) as usize].faces[self.cfg.face_local(f) as usize]
    }
    #[inline]
    fn ring(&self, v: VertId) -> &[FaceId] {
        self.topo[self.cfg.node_of(v) as usize].ring_of(self.cfg.local_of(v) as usize)
    }
}

/// One attribute column of every node, read by vertex or face id across nodes. Elements of a
/// node that never wrote the column read as zero.
#[derive(Clone, Copy)]
pub struct AttrReader<'a> {
    pub(crate) cfg: NodeConfig,
    pub(crate) domain: Domain,
    pub(crate) cols: &'a [Option<Arc<AttrVec>>],
}

impl AttrReader<'_> {
    /// The value of vertex or face `id` (by the column's domain); `None` where its node never
    /// wrote the column.
    #[inline]
    pub fn get(&self, id: u32) -> Option<AttrValue> {
        let (n, l) = match self.domain {
            Domain::Vertex => (self.cfg.node_of(id), self.cfg.local_of(id)),
            Domain::Face => (self.cfg.face_node(id), self.cfg.face_local(id)),
        };
        Some(self.cols[n as usize].as_deref()?.get(l as usize))
    }
    /// A scalar column's value (0 where unwritten).
    #[inline]
    pub fn scalar(&self, id: u32) -> f32 {
        match self.get(id) {
            Some(AttrValue::F32(x)) => x,
            _ => 0.0,
        }
    }
    /// A label or flag column's value (0 where unwritten).
    #[inline]
    pub fn label(&self, id: u32) -> u32 {
        match self.get(id) {
            Some(AttrValue::U32(x)) => x,
            _ => 0,
        }
    }
}

/// One node, read-only. Slot `i` of every vertex column is vertex `base + i`; a dead slot has
/// position [`Vec3::DEAD`]. Face slot `j` is face `first_face + j`; a dead slot is `[NO; 4]`.
#[derive(Clone, Copy)]
pub struct NodeView<'a> {
    pub id: NodeId,
    pub base: VertId,
    pub first_face: FaceId,
    pub pos: &'a [Vec3],
    pub nrm: &'a [Vec3],
    pub faces: &'a [[u32; 4]],
    /// Owned vertices (local slots, ascending) that share a face with another node's vertex,
    /// or use a face stored in another node.
    pub seam: &'a [u32],
    /// Box around the node's own live vertices.
    pub bounds: Aabb,
    /// Other nodes' vertices that this node's faces use, ascending: with `bounds`, every
    /// corner of the node's faces.
    pub shared: &'a [VertId],
    pub(crate) topo: &'a NodeTopo,
    pub(crate) attrs: &'a [Vec<Option<Arc<AttrVec>>>],
    /// Hidden vertex slots, `None` when none is.
    pub(crate) hidden: Option<&'a Bits>,
}

impl<'a> NodeView<'a> {
    /// Faces using local vertex slot `l`, ascending.
    pub fn ring(&self, l: usize) -> &'a [FaceId] {
        self.topo.ring_of(l)
    }
    /// This node's values of an attribute, or `None` if never written (all zeros).
    pub fn attr(&self, a: AttrId) -> Option<&'a AttrVec> {
        self.attrs[a.index()][self.id as usize].as_deref()
    }
    /// Whether local vertex slot `l` is hidden: every face around it is (decision 0015).
    #[inline]
    pub fn hidden(&self, l: usize) -> bool {
        self.hidden.is_some_and(|b| b.get(l))
    }
}
