//! [`MeshStore`]: the sculpt mesh, layout B of decision 0001. Every node owns its columns as
//! `Arc`s; reads go through [`NodeView`] and the cross-node readers; writes go through the
//! kernel step, the write guard, the topology editor or a whole-mesh replacement, all of which
//! write with `Arc::make_mut` only.

use std::sync::Arc;

use crate::build::{Flat, Placed, build};
use crate::columns::{AttrDef, AttrId, ColumnSet, Columns, MAX_ATTRS};
use crate::config::{MeshError, NodeConfig, NodeId, VertId};
use crate::derived::{
    Derived, Runs, halo_of, listed_for, memberships, neighbours, rejudged, rim_of, seam_after,
    seam_of, shared_of,
};
use crate::history::History;
use crate::math::{Aabb, Vec3, bounds_of};
use crate::normals;
use crate::par::{merge, select_mut};
use crate::tree::NodeTree;
use crate::view::{AttrReader, NodeView, PosReader, TopoReader};

/// Cumulative work counters. Tests hold optimizations to limits with these ("a small query
/// tests few boxes"), so switching an optimization off cannot pass unnoticed (ARCHITECTURE 10.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Tree boxes tested by node queries.
    pub boxes_tested: u64,
    /// Full tree builds. A refit does not count.
    pub tree_builds: u64,
    /// Vertex normals recomputed.
    pub normals: u64,
    /// Vertex slots handed to kernels in phase 1.
    pub examined: u64,
    /// Nodes a brush reached but skipped, because it could change none of their points or
    /// faces (decision 0015).
    pub skipped: u64,
    /// Global Morton sorts of the whole mesh (imports and re-sorts). A detail-level switch or
    /// a subdivide never adds one (decision 0014).
    pub sorts: u64,
    /// Node columns the topology editor copied because history or a share still held them
    /// (decision 0021, section 6): a column is copied at most once per step, and only when the
    /// node's own data changes.
    pub copies: u64,
    /// Values read to find a fresh face group id: one node summary each, never a face
    /// (decision 0026).
    pub group_reads: u64,
}

/// Every node's seam and shared lists (see `Derived`).
pub(crate) type Lists = (Vec<Vec<u32>>, Vec<Vec<VertId>>);

/// The nodes a sphere touches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeQuery {
    /// Nodes whose own bounds touch the sphere: only these can own a vertex inside it.
    pub brushed: Vec<NodeId>,
    /// Other nodes whose halo touches it: only their seam normals can change.
    pub shell: Vec<NodeId>,
    /// Tree boxes tested.
    pub tested: usize,
}

/// The sculpt mesh.
pub struct MeshStore {
    pub(crate) cfg: NodeConfig,
    pub(crate) defs: Vec<AttrDef>,
    pub(crate) cols: Columns,
    pub(crate) der: Derived,
    pub(crate) dirty: Vec<ColumnSet>,
    pub(crate) hist: History,
    pub(crate) work: Work,
}

impl MeshStore {
    /// Cut a flat mesh into nodes and compute everything derived. Also returns where every
    /// vertex and face of `flat` went.
    pub(crate) fn from_flat(
        flat: &Flat,
        cfg: NodeConfig,
        defs: Vec<AttrDef>,
    ) -> Result<(MeshStore, Placed), MeshError> {
        let (cols, placed) = build(flat, cfg, &defs)?;
        let mut s = MeshStore {
            cfg,
            defs,
            cols,
            der: Derived::default(),
            dirty: Vec::new(),
            hist: History::default(),
            work: Work {
                sorts: 1,
                ..Work::default()
            },
        };
        s.refresh_all();
        s.drop_broken_mirrors();
        Ok((s, placed))
    }

    /// A copy of the mesh as it is now that shares every node column with this one (decision
    /// 0010). It costs a reference-count bump per column and a copy of the small per-node
    /// derived data; writing on either side copies only the nodes written. The copy starts
    /// with no history and no open step, and nothing marked dirty.
    #[must_use]
    pub fn share(&self) -> MeshStore {
        MeshStore {
            cfg: self.cfg,
            defs: self.defs.clone(),
            cols: self.cols.clone(),
            der: self.der.clone(),
            dirty: Vec::new(),
            hist: History::default(),
            work: self.work,
        }
    }

    /// True if both stores hold the very same columns: the same allocation for every column of
    /// every node, so the same mesh bit for bit. Two shares of one store are, until one writes.
    pub fn same_columns(&self, other: &MeshStore) -> bool {
        self.alike(other) && self.cols.same(&other.cols)
    }

    /// True if both stores hold the same mesh stored the same way: the same attribute columns,
    /// and every node's positions, faces and attribute values equal bit for bit (normals follow
    /// from those). Stores cut differently compare unequal even if they export the same mesh.
    pub fn same_content(&self, other: &MeshStore) -> bool {
        self.alike(other) && self.cols.equal(&other.cols, &self.defs)
    }

    /// The same node size and attribute columns.
    fn alike(&self, other: &MeshStore) -> bool {
        self.cfg == other.cfg && self.defs == other.defs
    }

    /// A store over ready node columns (a detail level's, decision 0014), with derived data,
    /// normals and the tree computed. `lists` are the seam and shared lists when the level's
    /// skeleton already knows them, so only boxes and normals are worked out.
    pub(crate) fn from_columns(
        cfg: NodeConfig,
        defs: Vec<AttrDef>,
        cols: Columns,
        lists: Option<Lists>,
    ) -> MeshStore {
        let mut s = MeshStore {
            cfg,
            defs,
            cols,
            der: Derived::default(),
            dirty: Vec::new(),
            hist: History::default(),
            work: Work::default(),
        };
        // A level is cut from another; no partner id carries over.
        for (axis_col, d) in s.cols.attrs.iter_mut().zip(&s.defs) {
            if crate::mirror::axis_of(d).is_some() {
                axis_col.fill(None);
            }
        }
        let all: Vec<NodeId> = (0..s.node_count() as u32).collect();
        match lists {
            Some((seam, shared)) => {
                s.der.seam = seam;
                s.der.shared = shared;
                s.refresh_with(&all, false, true);
            }
            None => s.refresh(&all, true),
        }
        s.refresh_state(&all, ColumnSet::TOPOLOGY);
        s.normals(&all, &[]);
        let every = ColumnSet::POSITION | ColumnSet::NORMAL | ColumnSet::TOPOLOGY;
        s.mark(&all, every | s.attr_set());
        s
    }

    /// The seam and shared lists of every node: what a level's skeleton keeps beside its
    /// topology columns.
    pub(crate) fn lists(&self) -> Lists {
        (self.der.seam.clone(), self.der.shared.clone())
    }

    /// Derived data and normals of every node from scratch, every node dirty.
    pub(crate) fn refresh_all(&mut self) {
        let all: Vec<NodeId> = (0..self.node_count() as u32).collect();
        self.refresh(&all, true);
        self.refresh_state(&all, ColumnSet::TOPOLOGY);
        self.normals(&all, &[]);
        let every = ColumnSet::POSITION | ColumnSet::NORMAL | ColumnSet::TOPOLOGY;
        self.mark(&all, every | self.attr_set());
    }

    pub fn config(&self) -> NodeConfig {
        self.cfg
    }
    pub fn node_count(&self) -> usize {
        self.cols.len()
    }
    /// Live vertices.
    pub fn vertex_count(&self) -> usize {
        self.cols
            .topo
            .iter()
            .map(|t| t.ring.len() - t.vfree.len())
            .sum()
    }
    /// Live faces.
    pub fn face_count(&self) -> usize {
        let live = |t: &Arc<crate::columns::NodeTopo>| t.faces.len() - t.ffree.len();
        self.cols.topo.iter().map(live).sum()
    }

    /// One node, read-only.
    pub fn node(&self, n: NodeId) -> NodeView<'_> {
        let k = n as usize;
        NodeView {
            id: n,
            base: self.cfg.vert(n, 0),
            first_face: self.cfg.face(n, 0),
            pos: &self.cols.pos[k],
            nrm: &self.cols.nrm[k],
            faces: &self.cols.topo[k].faces,
            seam: &self.der.seam[k],
            bounds: self.der.bounds[k],
            shared: &self.der.shared[k],
            topo: &self.cols.topo[k],
            attrs: &self.cols.attrs,
            hidden: self.der.vis.hidden.get(k).and_then(Option::as_ref),
        }
    }
    /// Current positions, across nodes.
    pub fn pos(&self) -> PosReader<'_> {
        PosReader {
            cfg: self.cfg,
            cols: &self.cols.pos,
        }
    }
    /// Current normals, across nodes.
    pub fn nrm(&self) -> PosReader<'_> {
        PosReader {
            cfg: self.cfg,
            cols: &self.cols.nrm,
        }
    }
    /// Faces and rings, across nodes.
    pub fn topo(&self) -> TopoReader<'_> {
        TopoReader {
            cfg: self.cfg,
            topo: &self.cols.topo,
        }
    }
    /// Box around node `n`'s own live vertices.
    pub fn bounds(&self, n: NodeId) -> Aabb {
        self.der.bounds[n as usize]
    }
    /// Node `n`'s bounds grown by every vertex that shares a face with one of its seam vertices.
    pub fn halo(&self, n: NodeId) -> Aabb {
        self.der.halo[n as usize]
    }
    /// Other nodes' vertices that node `n`'s faces use, ascending.
    pub fn shared(&self, n: NodeId) -> &[VertId] {
        &self.der.shared[n as usize]
    }
    pub fn work(&self) -> Work {
        self.work
    }

    /// Attribute `a` of every node, read across nodes.
    pub fn attr_reader(&self, a: AttrId) -> AttrReader<'_> {
        AttrReader {
            cfg: self.cfg,
            domain: self.defs[a.index()].domain,
            cols: &self.cols.attrs[a.index()],
        }
    }

    /// Registered attribute columns.
    pub fn attrs(&self) -> &[AttrDef] {
        &self.defs
    }
    pub fn attr_id(&self, name: &str) -> Option<AttrId> {
        self.defs
            .iter()
            .position(|d| d.name == name)
            .map(|i| AttrId(i as u8))
    }
    /// Whether `def` can be written here: its id if it is registered, `None` if a write would
    /// register it, or why not (the name is taken by another definition, or every column is
    /// used).
    pub fn attr_fits(&self, def: &AttrDef) -> Result<Option<AttrId>, MeshError> {
        match self.attr_id(&def.name) {
            Some(id) if self.defs[id.index()] == *def => Ok(Some(id)),
            Some(_) => Err(MeshError::Input(format!(
                "attribute {} exists with another type",
                def.name
            ))),
            None if self.defs.len() == MAX_ATTRS => Err(MeshError::Input(format!(
                "more than {MAX_ATTRS} attributes"
            ))),
            None => Ok(None),
        }
    }
    /// Register an attribute column, unallocated on every node. Registering the same definition
    /// again returns its id; the same name with another definition is an error. Edits register
    /// a column only when they write a value into it ([`MeshStore::put_column`]).
    pub fn add_attr(&mut self, def: AttrDef) -> Result<AttrId, MeshError> {
        if let Some(id) = self.attr_fits(&def)? {
            return Ok(id);
        }
        self.defs.push(def);
        self.cols.attrs.push(vec![None; self.node_count()]);
        Ok(AttrId((self.defs.len() - 1) as u8))
    }
    /// The column set of every registered attribute.
    pub(crate) fn attr_set(&self) -> ColumnSet {
        (0..self.defs.len()).fold(ColumnSet::EMPTY, |s, a| {
            s | ColumnSet::attr(AttrId(a as u8))
        })
    }

    /// Nodes the sphere (`c`, `r`) touches, from the tree.
    pub fn query(&self, c: Vec3, r: f32) -> NodeQuery {
        let (hits, tested) = self.der.tree.query(c, r);
        let (brushed, shell) = hits
            .into_iter()
            .partition(|&n| self.der.bounds[n as usize].hits_sphere(c, r));
        NodeQuery {
            brushed,
            shell,
            tested,
        }
    }

    /// Nodes any of the spheres touches: the union of [`MeshStore::query`] over them, with a
    /// node brushed by one sphere never listed in the shell.
    pub fn query_all(&self, spheres: &[(Vec3, f32)]) -> NodeQuery {
        let mut all = NodeQuery::default();
        for &(c, r) in spheres {
            let q = self.query(c, r);
            all.brushed = merge(&all.brushed, &q.brushed);
            all.shell = merge(&all.shell, &q.shell);
            all.tested += q.tested;
        }
        all.shell.retain(|n| all.brushed.binary_search(n).is_err());
        all
    }

    /// Every node with dirty columns, ascending, and clear them all. The renderer calls this to
    /// learn which GPU ranges to upload.
    pub fn take_dirty(&mut self) -> Vec<(NodeId, ColumnSet)> {
        let out = (self.dirty.iter().enumerate())
            .filter(|(_, s)| !s.is_empty())
            .map(|(n, s)| (n as u32, *s))
            .collect();
        self.dirty.clear();
        out
    }
    pub(crate) fn mark(&mut self, nodes: &[NodeId], cols: ColumnSet) {
        self.dirty.resize(self.node_count(), ColumnSet::EMPTY);
        for &n in nodes {
            self.dirty[n as usize] |= cols;
        }
    }

    /// Recompute bounds and halos of `nodes` (sorted), and with `topo` also their seam and
    /// shared lists and the whole tree; without it the tree is refit.
    pub(crate) fn refresh(&mut self, nodes: &[NodeId], topo: bool) {
        self.refresh_with(nodes, topo, topo);
    }

    /// [`MeshStore::refresh`] with the seam and shared lists recomputed only with `lists`, and
    /// the tree built with `build` (else refit).
    fn refresh_with(&mut self, nodes: &[NodeId], lists: bool, build: bool) {
        self.refresh_bounds(nodes, lists);
        self.refresh_halos(nodes, build);
    }

    /// Recompute the bounds of `nodes` (sorted), and with `lists` their seam and shared lists.
    /// Halos and the tree are left for [`MeshStore::refresh_halos`].
    pub(crate) fn refresh_bounds(&mut self, nodes: &[NodeId], lists: bool) {
        let nn = self.node_count();
        let der = &mut self.der;
        der.bounds.resize(nn, Aabb::EMPTY);
        der.halo.resize(nn, Aabb::EMPTY);
        der.seam.resize(nn, Vec::new());
        der.shared.resize(nn, Vec::new());
        der.rim.resize(nn, None);
        let (cfg, cols) = (self.cfg, &self.cols);
        let t = TopoReader {
            cfg,
            topo: &cols.topo,
        };
        let fresh = gs_par::map(nodes.len(), |i| {
            let k = nodes[i] as usize;
            let lists = lists.then(|| {
                let own = &cols.topo[k];
                (
                    seam_of(cfg, nodes[i], own, &t),
                    shared_of(cfg, nodes[i], own),
                )
            });
            (bounds_of(&cols.pos[k]), lists)
        });
        for (&n, (b, lists)) in nodes.iter().zip(fresh) {
            der.bounds[n as usize] = b;
            if let Some((seam, shared)) = lists {
                der.seam[n as usize] = seam;
                // A rim reads the faces around the node's seam, and those of a node whose
                // vertices this node's faces use.
                der.rim[n as usize] = None;
                for &u in &shared {
                    der.rim[cfg.node_of(u) as usize] = None;
                }
                der.shared[n as usize] = shared;
            }
        }
    }

    /// The seam and shared lists of `written` and the rims of `around` (both sorted, `around`
    /// holding every node owning a corner of a face rewritten) after a topology edit, judging
    /// again only the vertices of `stale` (sorted): every vertex whose ring was rewritten or
    /// that is a corner, before or after, of a face rewritten. Every other entry stands (see
    /// [`memberships`]); a rim not yet built stays so. Run after [`MeshStore::refresh_bounds`].
    pub(crate) fn relist(&mut self, written: &[NodeId], around: &[NodeId], stale: &[VertId]) {
        let (cfg, cols, der) = (self.cfg, &self.cols, &mut self.der);
        let t = TopoReader {
            cfg,
            topo: &cols.topo,
        };
        let (shared, rim) = memberships(cfg, &t, stale);
        let lists = gs_par::map(written.len(), |i| {
            let (n, k) = (written[i], written[i] as usize);
            let a = stale.partition_point(|&u| cfg.node_of(u) < n);
            let b = stale.partition_point(|&u| cfg.node_of(u) <= n);
            let slots: Vec<u32> = stale[a..b].iter().map(|&u| cfg.local_of(u)).collect();
            (
                seam_after(cfg, n, &cols.topo[k], &t, &der.seam[k], &slots),
                rejudged(&der.shared[k], stale, &listed_for(&shared, n)),
            )
        });
        for (&n, (seam, sh)) in written.iter().zip(lists) {
            (der.seam[n as usize], der.shared[n as usize]) = (seam, sh);
        }
        let rims = gs_par::map(around.len(), |i| {
            let m = around[i];
            let old = der.rim[m as usize].as_deref();
            old.map(|old| rejudged(old, stale, &listed_for(&rim, m)))
        });
        for (&m, r) in around.iter().zip(rims) {
            if r.is_some() {
                der.rim[m as usize] = r;
            }
        }
    }

    /// Recompute the halos of `nodes` (sorted) from their bounds and rims, rebuilding a rim a
    /// topology change dropped, and drop their run boxes; then build the tree with `build`,
    /// else refit it.
    pub(crate) fn refresh_halos(&mut self, nodes: &[NodeId], build: bool) {
        let (cfg, cols, der) = (self.cfg, &self.cols, &mut self.der);
        let t = TopoReader {
            cfg,
            topo: &cols.topo,
        };
        let p = PosReader {
            cfg,
            cols: &cols.pos,
        };
        let stale: Vec<NodeId> = (nodes.iter().copied())
            .filter(|&n| der.rim[n as usize].is_none())
            .collect();
        let rims = gs_par::map(stale.len(), |i| {
            rim_of(cfg, stale[i], &der.seam[stale[i] as usize], &t)
        });
        for (&n, r) in stale.iter().zip(rims) {
            der.rim[n as usize] = Some(r);
        }
        let halos = gs_par::map(nodes.len(), |i| {
            let k = nodes[i] as usize;
            halo_of(der.bounds[k], der.rim[k].as_deref().unwrap_or(&[]), &p)
        });
        for (&n, h) in nodes.iter().zip(halos) {
            der.halo[n as usize] = h;
        }
        // A node's run boxes go with its halo: the next search rebuilds them.
        der.runs.resize_with(der.halo.len(), Runs::default);
        for &n in nodes {
            der.runs[n as usize] = Runs::default();
        }
        if build {
            der.tree = NodeTree::build(&der.halo);
            self.work.tree_builds += 1;
        } else {
            der.tree.refit(&der.halo);
        }
    }

    /// Recompute every normal of `full` and the seam normals of `seam_only` (sorted, disjoint).
    /// A node of `seam_only` is written only where a normal turns, so an open step holds it
    /// only then.
    pub(crate) fn normals(&mut self, full: &[NodeId], seam_only: &[NodeId]) {
        let all = merge(full, seam_only);
        let (cfg, der) = (self.cfg, &self.der);
        let Columns { pos, nrm, topo, .. } = &mut self.cols;
        let (p, t) = (PosReader { cfg, cols: pos }, TopoReader { cfg, topo });
        let mut work: Vec<(NodeId, &mut Arc<Vec<Vec3>>)> =
            all.iter().copied().zip(select_mut(nrm, &all)).collect();
        gs_par::apply_each(&mut work, |_, (n, nrm)| {
            let k = *n as usize;
            // An inactive top level holds no normals until it is active again (decision 0014,
            // section 13): nothing to update.
            if nrm.is_empty() && !pos[k].is_empty() {
                return;
            }
            let seam = &der.seam[k];
            if full.binary_search(n).is_ok() {
                let nrm = Arc::make_mut(nrm);
                normals::interior(cfg, *n, &pos[k], nrm, &topo[k].faces);
                normals::seam(cfg, *n, nrm, seam, &t, &p);
            } else {
                normals::seam_turned(cfg, *n, nrm, seam, &t, &p);
            }
        });
        let count = |ns: &[NodeId], f: &dyn Fn(usize) -> usize| -> u64 {
            ns.iter().map(|&n| f(n as usize) as u64).sum()
        };
        self.work.normals += count(full, &|k| self.cols.pos[k].len())
            + count(seam_only, &|k| self.der.seam[k].len());
    }

    /// After the positions of `moved` (sorted) changed: refresh their bounds, every halo and
    /// normal that can depend on them, refit the tree and mark them dirty.
    pub(crate) fn after_moves(&mut self, moved: &[NodeId]) {
        let nb = neighbours(self.cfg, &self.der, &self.topo(), moved);
        self.refresh(&merge(moved, &nb), false);
        self.normals(moved, &nb);
        self.mark(moved, ColumnSet::POSITION | ColumnSet::NORMAL);
        self.mark(&nb, ColumnSet::NORMAL);
    }
}
