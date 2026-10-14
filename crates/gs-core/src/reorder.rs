//! Node maintenance after topology edits (decision 0021, section 7).
//!
//! [`MeshStore::maintain_nodes`] runs at the end of a refining stroke, inside its history step,
//! in ascending node order:
//!
//! 1. **Compaction.** Every node left with dead slots has its highest live slots moved into its
//!    holes and its dead tail dropped, so no dead slot outlives a stroke. The engine numbers
//!    parts by dense slots (decision 0014), which is why this is part of L1 (decision 0021, item
//!    11.3).
//! 2. **Merge.** A node the stroke changed that is left with fewer than a quarter of `fill`
//!    points moves into the neighbour it shares the most faces with, when both fit in `fill`
//!    points and `4 × fill` faces. The node left empty is reused by the next node cut.
//! 3. **Re-sort.** A node the stroke changed has its points re-sorted by Morton code within its
//!    box when the stroke took it past `fill` slots, or when its order decayed: the mean index
//!    gap along its edges is over 1.5 times what the sorted order gives. Faces keep their slots;
//!    every face naming a moved point is rewritten.

use std::sync::Arc;

use crate::build::gather;
use crate::columns::{Domain, NodeTopo, ring_head};
use crate::config::{FaceId, NodeConfig, NodeId};
use crate::math::{NO, Vec3, arity, bounds_of, morton};
use crate::store::MeshStore;
use crate::topo::{TopoEditor, cow};

/// A changed node merges away when left with fewer than `fill / SMALL` points.
const SMALL: u32 = 4;

impl MeshStore {
    /// Compact every node with dead slots, ascending (step 1 of the module docs). Returns those
    /// nodes. A node no edit left dead slots in is not touched.
    pub fn compact_slots(&mut self) -> Vec<NodeId> {
        let nodes = self.with_dead_slots();
        if !nodes.is_empty() {
            let mut ed = self.edit_topology();
            for &n in &nodes {
                ed.compact_node(n);
            }
        }
        nodes
    }

    /// Compact, merge and re-sort (see the module docs) the nodes whose topology changed since
    /// the marked stroke or else the open step began. Returns the nodes whose own columns this
    /// wrote, ascending.
    pub fn maintain_nodes(&mut self) -> Vec<NodeId> {
        let dead = self.with_dead_slots();
        let changed = |s: &MeshStore| -> Vec<NodeId> {
            (0..s.node_count() as NodeId)
                .filter(|&n| s.start_topology_changed(n))
                .collect()
        };
        let touched = changed(self);
        if dead.is_empty() && touched.is_empty() {
            return Vec::new();
        }
        let mut ed = self.edit_topology();
        for &n in &dead {
            ed.compact_node(n);
        }
        for &n in &touched {
            ed.merge_small(n);
        }
        // Merge targets changed too.
        let now = changed(&*ed.s);
        let orders = resorts(&*ed.s, &now);
        for (n, order) in orders {
            ed.renumber(n, &order);
        }
        ed.written()
    }

    /// Nodes with a dead vertex or face slot, ascending.
    fn with_dead_slots(&self) -> Vec<NodeId> {
        (self.cols.topo.iter().enumerate())
            .filter(|(_, t)| !t.vfree.is_empty() || !t.ffree.is_empty())
            .map(|(n, _)| n as NodeId)
            .collect()
    }
}

/// For each of `nodes` that needs re-sorting (step 3 of the module docs), the new order of its
/// slots: new slot `l` takes old slot `order[l]`.
fn resorts(s: &MeshStore, nodes: &[NodeId]) -> Vec<(NodeId, Vec<u32>)> {
    let (cfg, fill) = (s.cfg, s.cfg.fill() as usize);
    let found = gs_par::map(nodes.len(), |i| {
        let n = nodes[i];
        let (pos, t) = (&s.cols.pos[n as usize], &s.cols.topo[n as usize]);
        if pos.is_empty() {
            return None;
        }
        let order = sorted(pos);
        // A sorted node gives the same gap sorted again, so it never decays.
        if order.iter().enumerate().all(|(l, &o)| o as usize == l) {
            return None;
        }
        let grew = s.start_slots(n) <= fill && pos.len() > fill;
        let decayed = || {
            let (now, best) = gaps(cfg, n, t, &order);
            2 * now > 3 * best
        };
        (grew || decayed()).then_some((n, order))
    });
    found.into_iter().flatten().collect()
}

/// Slots of `pos` (all live) by Morton code within their box, ties by slot.
fn sorted(pos: &[Vec3]) -> Vec<u32> {
    let b = bounds_of(pos);
    let mut keys: Vec<(u64, u32)> = (pos.iter().enumerate())
        .map(|(l, &p)| (morton(p, b), l as u32))
        .collect();
    keys.sort_unstable();
    keys.into_iter().map(|k| k.1).collect()
}

/// Sums of index gaps along the edges of node `n`'s own faces whose ends are both its points:
/// with the slots as they are, and with new slot `l` taking old slot `order[l]`. Compared
/// between numberings of the same node, the sums compare their means.
fn gaps(cfg: NodeConfig, n: NodeId, t: &NodeTopo, order: &[u32]) -> (u64, u64) {
    let mut new = vec![0u32; order.len()];
    for (l, &o) in order.iter().enumerate() {
        new[o as usize] = l as u32;
    }
    let (mut now, mut best) = (0u64, 0u64);
    for f in t.faces.iter().filter(|f| f[0] != NO) {
        let k = arity(*f);
        for i in 0..k {
            let (a, b) = (f[i], f[(i + 1) % k]);
            if cfg.node_of(a) == n && cfg.node_of(b) == n {
                let (a, b) = (cfg.local_of(a), cfg.local_of(b));
                now += u64::from(a.abs_diff(b));
                best += u64::from(new[a as usize].abs_diff(new[b as usize]));
            }
        }
    }
    (now, best)
}

/// Pairs (from, to): the live slots at or past `live` that move, highest first, each into the
/// lowest hole left below `live`.
fn moves(len: usize, free: &[u32]) -> Vec<(u32, u32)> {
    let live = len - free.len();
    let mut dead = vec![false; len];
    for &l in free {
        dead[l as usize] = true;
    }
    let holes = (0..live).filter(|&l| dead[l]);
    let movers = (live..len).rev().filter(|&l| !dead[l]);
    movers
        .zip(holes)
        .map(|(m, h)| (m as u32, h as u32))
        .collect()
}

impl TopoEditor<'_> {
    /// Move node `n`'s highest live slots into its holes and drop its dead tail.
    fn compact_node(&mut self, n: NodeId) {
        let cfg = self.s.cfg;
        let t = self.node_topo(n);
        let (vlen, flen) = (t.ring.len(), t.faces.len());
        let vmoves = moves(vlen, &t.vfree);
        let fmoves = moves(flen, &t.ffree);
        let (vlive, flive) = (vlen - t.vfree.len(), flen - t.ffree.len());
        for (from, to) in vmoves {
            self.relocate_vert(cfg.vert(n, from), cfg.vert(n, to));
        }
        for (from, to) in fmoves {
            self.relocate_face(cfg.face(n, from), cfg.face(n, to));
        }
        self.truncate(n, vlive, flive);
    }

    /// Move node `n` (dense) into the neighbour sharing the most faces with it when it has
    /// fewer than `fill / 4` points and both fit (step 2 of the module docs).
    fn merge_small(&mut self, n: NodeId) {
        let cfg = self.s.cfg;
        let fill = cfg.fill() as usize;
        let (len, flen) = (self.node_topo(n).ring.len(), self.node_topo(n).faces.len());
        if len == 0 || len >= fill / SMALL as usize {
            return;
        }
        let Some(m) = self.most_shared(n) else {
            return;
        };
        let t = self.node_topo(m);
        if len + t.ring.len() > fill || flen + t.faces.len() > 4 * fill {
            return;
        }
        for l in 0..len as u32 {
            let w = self.alloc_vert(m);
            self.relocate_vert(cfg.vert(n, l), w);
        }
        for l in 0..flen as u32 {
            let g = self.alloc_face(m);
            self.relocate_face(cfg.face(n, l), g);
        }
        self.truncate(n, 0, 0);
    }

    /// The node other than `n` with the most faces using points of both, lowest id on ties.
    fn most_shared(&self, n: NodeId) -> Option<NodeId> {
        let cfg = self.s.cfg;
        let t = self.node_topo(n);
        let stored = (0..t.faces.len() as u32).map(|l| cfg.face(n, l));
        let around = (0..t.ring.len()).flat_map(|l| t.ring_of(l).iter().copied());
        let mut pairs: Vec<(NodeId, FaceId)> = Vec::new();
        for f in stored.chain(around) {
            let fc = self.face(f);
            let others = fc[..arity(fc)].iter().map(|&u| cfg.node_of(u));
            pairs.extend(others.filter(|&m| m != n).map(|m| (m, f)));
        }
        pairs.sort_unstable();
        pairs.dedup();
        let mut best: Option<(usize, NodeId)> = None;
        for run in pairs.chunk_by(|a, b| a.0 == b.0) {
            if best.is_none_or(|b| run.len() > b.0) {
                best = Some((run.len(), run[0].0));
            }
        }
        best.map(|b| b.1)
    }

    /// Renumber node `n`'s points (dense): new slot `l` takes old slot `order[l]`. Every face
    /// naming one is rewritten; faces keep their ids, so every ring keeps its faces and every
    /// normal its bits.
    fn renumber(&mut self, n: NodeId, order: &[u32]) {
        let (cfg, k) = (self.s.cfg, n as usize);
        let mut new = vec![0u32; order.len()];
        for (l, &o) in order.iter().enumerate() {
            new[o as usize] = l as u32;
        }
        let t = self.node_topo(n);
        let mut faces: Vec<FaceId> = (0..t.ring.len())
            .flat_map(|l| t.ring_of(l).iter().copied())
            .collect();
        faces.sort_unstable();
        faces.dedup();
        for f in faces {
            let was = self.face(f);
            let fc = was.map(|u| {
                if u != NO && cfg.node_of(u) == n {
                    cfg.vert(n, new[cfg.local_of(u) as usize])
                } else {
                    u
                }
            });
            // A face naming only points that keep their slots is left alone, so a node whose
            // faces do not change is not copied.
            if fc != was {
                self.set_face(f, fc);
            }
        }
        self.mark(n);
        let copies = &mut self.s.work.copies;
        let t = cow(&mut self.s.cols.topo[k], copies);
        let (mut ring, mut pool) = (Vec::with_capacity(order.len()), Vec::new());
        for &o in order {
            let r = t.ring_of(o as usize);
            ring.push(ring_head(pool.len(), r.len()));
            pool.extend_from_slice(r);
        }
        (t.ring, t.pool) = (ring, pool);
        let pick = |v: &[Vec3]| Arc::new(order.iter().map(|&o| v[o as usize]).collect());
        self.s.cols.pos[k] = pick(&self.s.cols.pos[k]);
        self.s.cols.nrm[k] = pick(&self.s.cols.nrm[k]);
        for (a, d) in self.s.defs.iter().enumerate() {
            let col = &mut self.s.cols.attrs[a][k];
            if let (Domain::Vertex, Some(v)) = (d.domain, col.as_ref()) {
                *col = Some(Arc::new(gather(v, order.iter().copied())));
            }
        }
        self.mirror_renumbered(n, &new);
    }

    /// Drop node `n`'s slots from `verts` and `faces` on, which are all dead, and empty its
    /// free lists and ring pool garbage. A node left with no slot holds no column at all.
    fn truncate(&mut self, n: NodeId, verts: usize, faces: usize) {
        let k = n as usize;
        self.mark(n);
        if verts == 0 && faces == 0 {
            let c = &mut self.s.cols;
            (c.pos[k], c.nrm[k], c.topo[k]) = Default::default();
            for a in &mut c.attrs {
                a[k] = None;
            }
            self.rebuild_tree();
            return;
        }
        let copies = &mut self.s.work.copies;
        let t = cow(&mut self.s.cols.topo[k], copies);
        t.ring.truncate(verts);
        t.faces.truncate(faces);
        t.vfree.clear();
        t.ffree.clear();
        t.compact();
        cow(&mut self.s.cols.pos[k], copies).truncate(verts);
        cow(&mut self.s.cols.nrm[k], copies).truncate(verts);
        for (a, d) in self.s.defs.iter().enumerate() {
            if let Some(v) = self.s.cols.attrs[a][k].as_mut() {
                let len = match d.domain {
                    Domain::Vertex => verts,
                    Domain::Face => faces,
                };
                if v.len() != len {
                    cow(v, copies).resize(len, d.kind);
                }
            }
        }
    }
}
