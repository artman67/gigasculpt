//! Per-node data derived from the columns: bounds, halo boxes, seam and shared vertex lists, the
//! tree. Recomputed for the nodes an edit touched; never part of undo.

use std::sync::{Arc, OnceLock};

use crate::columns::NodeTopo;
use crate::config::{NodeConfig, NodeId, VertId};
use crate::math::{Aabb, NO, Vec3, arity, bounds_of};
use crate::par::union;
use crate::tree::NodeTree;
use crate::view::{PosRead, PosReader, TopoRead, TopoReader};
use crate::visibility::VisState;

#[derive(Clone, Default)]
pub(crate) struct Derived {
    /// Box around the node's own live vertices.
    pub bounds: Vec<Aabb>,
    /// `bounds` grown by every vertex sharing a face with a seam vertex. A brush that misses
    /// this box cannot change any of the node's normals. The tree is built over these.
    pub halo: Vec<Aabb>,
    /// Owned seam vertices, local slots ascending (see `NodeView::seam`).
    pub seam: Vec<Vec<u32>>,
    /// Other nodes' vertices that this node's faces use, ascending. The seam seen from the
    /// other side: each seam vertex is owned by one node and listed as shared by its neighbours.
    pub shared: Vec<Vec<VertId>>,
    /// Each node's [`rim_of`], or `None` once a topology change here or in a node it shares a
    /// face with may have changed it; [`MeshStore::refresh_halos`] rebuilds it when needed. The
    /// topology editor's close keeps it instead, judging again only the vertices it touched
    /// ([`MeshStore::relist`]).
    ///
    /// [`MeshStore::refresh_halos`]: crate::store::MeshStore::refresh_halos
    /// [`MeshStore::relist`]: crate::store::MeshStore::relist
    pub rim: Vec<Option<Vec<VertId>>>,
    pub tree: NodeTree,
    /// Hidden points and node summaries (decision 0015).
    pub vis: VisState,
    /// Each node's [`Runs`], dropped with its halo by [`MeshStore::refresh_halos`].
    ///
    /// [`MeshStore::refresh_halos`]: crate::store::MeshStore::refresh_halos
    pub runs: Vec<Runs>,
}

/// Slots in a run: the face slots, or vertex slots, one of a node's [`Runs`] boxes covers.
pub(crate) const RUN: usize = 64;

/// Boxes around runs of [`RUN`] slots of one node (decision 0001, amended 2026-10-11), so a
/// ray cast or nearest search that reaches the node tests only the runs near it: for its face
/// slots, every corner of the run's live faces, hidden or not; for its vertex slots, the run's
/// live vertices. Each list is built by the first search that needs it and dropped when the
/// node's halo is recomputed, which every edit that can move a corner of the node's faces does,
/// so an edit pays nothing for them until the next search.
#[derive(Clone, Default)]
pub(crate) struct Runs {
    faces: OnceLock<Arc<[Aabb]>>,
    verts: OnceLock<Arc<[Aabb]>>,
}

impl Runs {
    /// The box of each run of one node's `faces`, whose corners `pos` reads.
    pub(crate) fn faces(&self, faces: &[[u32; 4]], pos: &impl PosRead) -> &[Aabb] {
        self.faces.get_or_init(|| {
            (faces.chunks(RUN))
                .map(|run| {
                    let mut b = Aabb::EMPTY;
                    for f in run.iter().filter(|f| f[0] != NO) {
                        for &v in &f[..arity(*f)] {
                            b.grow(pos.get(v));
                        }
                    }
                    b
                })
                .collect()
        })
    }

    /// The box of each run of one node's vertex slots `pos`: empty for a run of dead slots.
    pub(crate) fn verts(&self, pos: &[Vec3]) -> &[Aabb] {
        self.verts
            .get_or_init(|| pos.chunks(RUN).map(bounds_of).collect())
    }
}

/// Owned vertices of node `n` with a face stored elsewhere or a face using a foreign vertex.
pub(crate) fn seam_of(cfg: NodeConfig, n: NodeId, own: &NodeTopo, t: &TopoReader) -> Vec<u32> {
    (0..own.ring.len())
        .filter(|&l| on_seam(cfg, n, own, t, l))
        .map(|l| l as u32)
        .collect()
}

/// Whether local slot `l` of node `n` is on its seam: a face around it is stored elsewhere or
/// uses a foreign vertex. It depends on the slot's ring and the faces in it, nothing else.
fn on_seam(cfg: NodeConfig, n: NodeId, own: &NodeTopo, t: &TopoReader, l: usize) -> bool {
    own.ring_of(l).iter().any(|&f| {
        let fc = t.face(f);
        cfg.face_node(f) != n || fc[..arity(fc)].iter().any(|&u| cfg.node_of(u) != n)
    })
}

/// Node `n`'s seam list after a topology edit that rewrote the rings of, or faces around, only
/// its local slots `touched` (ascending): `old` with those slots judged again. A slot past the
/// node's end (dropped by a compaction) is on no list.
pub(crate) fn seam_after(
    cfg: NodeConfig,
    n: NodeId,
    own: &NodeTopo,
    t: &TopoReader,
    old: &[u32],
    touched: &[u32],
) -> Vec<u32> {
    let fresh: Vec<u32> = (touched.iter().copied())
        .filter(|&l| (l as usize) < own.ring.len() && on_seam(cfg, n, own, t, l as usize))
        .collect();
    rejudged(old, touched, &fresh)
}

/// A sorted list after its members among `touched` (ascending) are judged again: `old` without
/// them, merged with those that still belong, `fresh` (ascending, all in `touched`).
pub(crate) fn rejudged(old: &[u32], touched: &[u32], fresh: &[u32]) -> Vec<u32> {
    let kept: Vec<u32> = (old.iter().copied())
        .filter(|x| touched.binary_search(x).is_err())
        .collect();
    union(&kept, fresh)
}

/// `(node, vertex)` pairs, ascending.
pub(crate) type Pairs = Vec<(NodeId, VertId)>;

/// Where each vertex of `stale` (ascending) that still has a slot belongs after a topology
/// edit, as `(node, vertex)` pairs ascending: in the shared list of every other node storing a
/// face around it, and in the rim of every other node owning a corner of one. A vertex of a
/// node's face makes a seam vertex of each of its other corners, so a node's rim is every
/// foreign vertex sharing a face with one of its own: both memberships depend only on the
/// vertex's ring and the corners of the faces in it.
pub(crate) fn memberships(cfg: NodeConfig, t: &TopoReader, stale: &[VertId]) -> (Pairs, Pairs) {
    let (mut shared, mut rim) = (Vec::new(), Vec::new());
    for &u in stale {
        let n = cfg.node_of(u);
        let slots = t.topo.get(n as usize).map_or(0, |x| x.ring.len());
        if cfg.local_of(u) as usize >= slots {
            continue;
        }
        for &f in t.ring(u) {
            if cfg.face_node(f) != n {
                shared.push((cfg.face_node(f), u));
            }
            let fc = t.face(f);
            for &c in &fc[..arity(fc)] {
                if cfg.node_of(c) != n {
                    rim.push((cfg.node_of(c), u));
                }
            }
        }
    }
    for v in [&mut shared, &mut rim] {
        v.sort_unstable();
        v.dedup();
    }
    (shared, rim)
}

/// The vertices of `pairs` (ascending, from [`memberships`]) listed for node `n`, ascending.
pub(crate) fn listed_for(pairs: &[(NodeId, VertId)], n: NodeId) -> Vec<VertId> {
    let a = pairs.partition_point(|p| p.0 < n);
    let b = pairs.partition_point(|p| p.0 <= n);
    pairs[a..b].iter().map(|p| p.1).collect()
}

/// Foreign vertices used by node `n`'s faces.
pub(crate) fn shared_of(cfg: NodeConfig, n: NodeId, own: &NodeTopo) -> Vec<VertId> {
    let mut v: Vec<VertId> = own
        .faces
        .iter()
        .flat_map(|fc| fc[..arity(*fc)].iter().copied())
        .filter(|&u| u != crate::math::NO && cfg.node_of(u) != n)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Every node's seam and shared lists from its topology alone: a detail level's skeleton read
/// from a file, before its store exists.
pub(crate) fn lists_of(
    cfg: NodeConfig,
    topo: &[std::sync::Arc<NodeTopo>],
) -> (Vec<Vec<u32>>, Vec<Vec<VertId>>) {
    let t = TopoReader { cfg, topo };
    let lists = gs_par::map(topo.len(), |k| {
        let n = k as NodeId;
        (seam_of(cfg, n, &topo[k], &t), shared_of(cfg, n, &topo[k]))
    });
    lists.into_iter().unzip()
}

/// Other nodes' vertices on a face around a seam vertex of node `n`, ascending: with `n`'s own
/// vertices, every corner a change to `n`'s normals can come from.
pub(crate) fn rim_of(cfg: NodeConfig, n: NodeId, seam: &[u32], t: &TopoReader) -> Vec<VertId> {
    let mut v = Vec::new();
    for &l in seam {
        for &f in t.ring(cfg.vert(n, l)) {
            let fc = t.face(f);
            v.extend((fc[..arity(fc)].iter()).filter(|&&u| cfg.node_of(u) != n));
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// A node's halo: its bounds `own` grown by the positions of its `rim`.
pub(crate) fn halo_of(own: Aabb, rim: &[VertId], p: &PosReader) -> Aabb {
    let mut h = own;
    for &u in rim {
        h.grow(p.get(u));
    }
    h
}

/// Nodes owning a vertex that shares a face with a seam vertex of `nodes`, or a corner of a
/// face `nodes` store, minus `nodes`: the nodes whose seam normals and halos can change when
/// `nodes` move or their faces change. (A face is stored in a node owning one of its corners, so
/// the seam walk finds the corners of stored faces too; the shared list also covers a face the
/// topology editor's close found no room to move.) A node's rim, when it is current, already
/// lists the corners its seam walk would find, ascending, so its owners come without walking or
/// sorting.
pub(crate) fn neighbours(
    cfg: NodeConfig,
    der: &Derived,
    t: &TopoReader,
    nodes: &[NodeId],
) -> Vec<NodeId> {
    let per = gs_par::map(nodes.len(), |i| {
        let n = nodes[i];
        let mut out: Vec<NodeId> = Vec::new();
        let mut add = |u: VertId| {
            let m = cfg.node_of(u);
            if out.last() != Some(&m) {
                out.push(m);
            }
        };
        der.shared[n as usize].iter().for_each(|&u| add(u));
        if let Some(rim) = &der.rim[n as usize] {
            for &u in rim {
                add(u);
            }
        } else {
            for &l in &der.seam[n as usize] {
                for &f in t.ring(cfg.vert(n, l)) {
                    let fc = t.face(f);
                    fc[..arity(fc)].iter().for_each(|&u| add(u));
                }
            }
        }
        out
    });
    let mut v: Vec<NodeId> = per.concat();
    v.sort_unstable();
    v.dedup();
    v.retain(|x| nodes.binary_search(x).is_err());
    v
}
