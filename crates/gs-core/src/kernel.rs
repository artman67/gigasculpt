//! The two-phase kernel framework (ARCHITECTURE 3.1 and 4.3).
//!
//! A kernel step (one dab) runs in three parallel passes over the nodes the brush touches:
//!
//! 1. The kernel reads a read-only [`SculptSurface`] and writes one position delta per vertex
//!    into scratch, node by node.
//! 2. Fused, per brushed node that moves: add the deltas to the node's own positions (through
//!    `Arc::make_mut`, so an open history step keeps the old ones), grow its bounds and
//!    recompute the normals of its interior vertices.
//! 3. Seam normals of the brushed nodes and of the shell (nodes whose halo touches the brush),
//!    read across nodes now that every position is final; then halos and a tree refit.
//!
//! A kernel whose deltas read only their own vertex ([`Kernel::own_only`]) runs phases 1 and 2
//! in the same task, node by node, so a node's columns are read once while they are in cache.
//!
//! No locks, no atomics, no float reductions across threads: every node is written by one task,
//! so the result does not depend on the thread count.

use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;

use crate::columns::{AttrId, ColumnSet, Columns};
use crate::config::{NodeConfig, NodeId, VertId};
use crate::math::{Aabb, Vec3};
use crate::normals;
use crate::par::{merge, select_mut};
use crate::store::{MeshStore, NodeQuery};
use crate::view::{AttrReader, NodeView, PosReader, TopoReader};
use crate::visibility::{Cover, Visible};

/// A deformation kernel: the only brush-specific step of a dab.
pub trait Kernel: Sync {
    /// Where the kernel acts, as spheres (one per symmetry pass of a brush dab, for example).
    /// It must leave every vertex outside all of them where it is.
    fn spheres(&self) -> Vec<(Vec3, f32)>;
    /// The directions along which each sphere reaches without end (Infinite Depth), one entry
    /// per sphere, or none: then the spheres are plain.
    fn free(&self) -> Vec<Free> {
        Vec::new()
    }
    /// Phase 1: one position delta per vertex slot of `node` into `out` (zeroed), reading
    /// only `surface`.
    fn deltas(&self, surface: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]);
    /// Which nodes the kernel can move a vertex of, so the step skips the rest. Must be true to
    /// the kernel: a node it would skip must get only zero deltas.
    fn reach(&self) -> Reach {
        Reach::Everything
    }
    /// Whether a vertex's delta reads, of the mesh's positions and normals, only the vertex's
    /// own (`node.pos[l]` and `node.nrm[l]`). Then each node runs both phases in one task, and
    /// the surface shows the other brushed nodes' positions and normals as empty.
    fn own_only(&self) -> bool {
        false
    }
    /// Whether phase 1 can run on part of a node ([`Kernel::part_deltas`]). Then a two-pass
    /// step splits its nodes into parts of [`PART`] slots, so it does not wait on its fullest
    /// node: a dab's nodes hold from a few to all of their slots in the brush.
    fn parts(&self) -> bool {
        false
    }
    /// Phase 1 for the slots `slots` of `node`: `out[i]` (zeroed) takes the delta of slot
    /// `slots.start + i`, the same as [`Kernel::deltas`] gives it. Called only when
    /// [`Kernel::parts`] says so; this default runs the whole node.
    fn part_deltas(
        &self,
        surface: &SculptSurface<'_>,
        node: &NodeView<'_>,
        slots: Range<u32>,
        out: &mut [Vec3],
    ) {
        let mut all = vec![Vec3::ZERO; node.pos.len()];
        self.deltas(surface, node, &mut all);
        out.copy_from_slice(&all[slots.start as usize..slots.end as usize]);
    }
}

/// The most slots of a part of a node in phase 1 ([`Kernel::parts`], [`MeshStore::run_steps`]):
/// a dab's nodes hold from a few to all of their slots in the brush, so phase 1 run node by
/// node would wait on the fullest. Any size gives the same bits; 512 balanced Smooth's rounds
/// best at 24 threads (decision 0036, section 2, amended).
const PART: u32 = 512;

/// A kernel whose dab runs as rounds of one step ([`MeshStore::run_steps`], decision 0036,
/// section 2, amended), each part of a node keeping what it gathered from round to round. Its
/// deltas must read no normals: those are recomputed only after the last round.
pub(crate) trait Steps: Sync {
    /// What a part of a node keeps from round to round.
    type Part: Send + Sync;
    /// As [`Kernel::spheres`], the same in every round.
    fn spheres(&self) -> Vec<(Vec3, f32)>;
    /// As [`Kernel::free`].
    fn free(&self) -> Vec<Free> {
        Vec::new()
    }
    /// As [`Kernel::reach`].
    fn reach(&self) -> Reach;
    /// The state of the part of `node` holding its slots `slots`, before its first round.
    fn part(&self, node: &NodeView<'_>, slots: Range<u32>) -> Self::Part;
    /// Phase 1 of a round for a part of `node`: the slots it moves, with their deltas, into
    /// `out`. A later round moves no slot the first did not.
    fn deltas(
        &self,
        surface: &SculptSurface<'_>,
        node: &NodeView<'_>,
        part: &mut Self::Part,
        out: &mut Vec<(u32, Vec3)>,
    );
}

/// A part of a node in [`MeshStore::run_steps`].
struct Part<P> {
    /// The node's index in the dab's nodes.
    node: usize,
    slots: Range<u32>,
    /// Built in the first round.
    kept: Option<P>,
    /// The slots and deltas of the round.
    out: Vec<(u32, Vec3)>,
    /// Whether the round moved a point of the part.
    moved: bool,
}

/// The directions along which an Infinite Depth reach runs without end (decision 0053,
/// sections 3 and 4): at most three, at right angles to each other. Distances from a center
/// are measured across them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Free {
    dirs: [Vec3; 3],
    n: usize,
}

impl Free {
    /// From unit directions at right angles to each other; past the third they are dropped.
    pub fn new(dirs: impl IntoIterator<Item = Vec3>) -> Free {
        let mut f = Free {
            dirs: [Vec3::ZERO; 3],
            n: 0,
        };
        for e in dirs.into_iter().take(3) {
            f.dirs[f.n] = e;
            f.n += 1;
        }
        f
    }

    /// `v` less its parts along the free directions: zero when all three are free.
    pub fn across(&self, v: Vec3) -> Vec3 {
        if self.n == 3 {
            return Vec3::ZERO;
        }
        (self.dirs[..self.n].iter()).fold(v, |v, &e| v - e * v.dot(e))
    }

    /// Whether box `b`'s bounding sphere comes within `r` of `c` across the free directions:
    /// never false for a box with a point in reach. Always false for an empty box.
    pub fn hits(&self, b: Aabb, c: Vec3, r: f32) -> bool {
        if b.lo.x > b.hi.x {
            return false;
        }
        let half = (b.hi - b.lo) * 0.5;
        let reach = r + half.len2().sqrt();
        self.across(b.lo + half - c).len2() <= reach * reach
    }
}

/// Which nodes a brush can change (decision 0015): the node skips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Every node.
    Everything,
    /// Nodes with a point that is not hidden.
    Visible,
    /// Nodes with a point that is neither hidden nor fully masked.
    Unmasked,
    /// Nodes with a face that is not hidden.
    Faces,
}

impl MeshStore {
    /// `nodes` (ascending) split into those `reach` can change and those it skips.
    pub(crate) fn reachable(&self, nodes: &[NodeId], reach: Reach) -> (Vec<NodeId>, Vec<NodeId>) {
        nodes.iter().partition(|&&n| {
            let st = self.node_state(n);
            match reach {
                Reach::Everything => true,
                Reach::Visible => st.points != Cover::All,
                Reach::Unmasked => st.points != Cover::All && st.mask != Cover::All,
                Reach::Faces => st.faces != Cover::All,
            }
        })
    }
}

/// The mesh as a kernel sees it in phase 1: read-only.
#[derive(Clone, Copy)]
pub struct SculptSurface<'a> {
    store: &'a MeshStore,
    /// Read once per step: the faces a neighbour walk may cross.
    visible: Visible<'a>,
}

impl<'a> SculptSurface<'a> {
    pub fn node(&self, n: NodeId) -> NodeView<'a> {
        self.store.node(n)
    }
    /// Current positions, across nodes.
    pub fn pos(&self) -> PosReader<'a> {
        self.store.pos()
    }
    /// Positions when the open history step began (the stroke-start snapshot).
    pub fn start_pos(&self) -> PosReader<'a> {
        self.store.start_pos()
    }
    /// Normals when the open history step began.
    pub fn start_nrm(&self) -> PosReader<'a> {
        self.store.start_nrm()
    }
    /// Whether node `n`'s faces or rings differ from the start's, so the start readers name
    /// other points there ([`MeshStore::start_topology_changed`]).
    pub fn start_topology_changed(&self, n: NodeId) -> bool {
        self.store.start_topology_changed(n)
    }
    pub fn topo(&self) -> TopoReader<'a> {
        self.store.topo()
    }
    /// The faces a neighbour walk may cross.
    pub fn visible(&self) -> Visible<'a> {
        self.visible
    }
    /// Whether vertex `v` is hidden (every face around it is).
    pub fn hidden(&self, v: VertId) -> bool {
        let cfg = self.store.config();
        self.node(cfg.node_of(v)).hidden(cfg.local_of(v) as usize)
    }
    /// Attribute `a` of every node.
    pub fn attr_reader(&self, a: AttrId) -> AttrReader<'a> {
        self.store.attr_reader(a)
    }
    pub fn config(&self) -> crate::config::NodeConfig {
        self.store.config()
    }
}

/// Buffers reused from dab to dab: the deltas of the kernels that run two passes (those that
/// read beyond their own vertex), and the next positions of a dab's rounds.
#[derive(Default)]
pub struct Scratch {
    deltas: Vec<Vec<Vec3>>,
    next: Vec<Vec<Vec3>>,
}

/// What a kernel step touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepReport {
    /// Nodes whose positions and normals were written.
    pub brushed: Vec<NodeId>,
    /// Nodes whose seam normals were rewritten.
    pub shell: Vec<NodeId>,
}

impl MeshStore {
    /// The mesh as kernels read it.
    pub(crate) fn surface(&self) -> SculptSurface<'_> {
        SculptSurface {
            store: self,
            visible: self.visible(),
        }
    }

    /// The nodes a kernel step works on: [`MeshStore::query_all`] over `spheres`, or, with
    /// free directions (one per sphere, Infinite Depth, decision 0053, section 4), each node
    /// whose halo `free` hits as the shell and whose own bounds it hits as brushed. Every
    /// node's box is tested: the tree's boxes would need a test of their own for a reach
    /// without end.
    fn gather(&self, spheres: &[(Vec3, f32)], free: &[Free]) -> NodeQuery {
        if free.is_empty() {
            return self.query_all(spheres);
        }
        let nodes = 0..self.node_count() as NodeId;
        let mut all = NodeQuery::default();
        for (&(c, r), e) in spheres.iter().zip(free) {
            let (brushed, shell): (Vec<NodeId>, Vec<NodeId>) = (nodes.clone())
                .filter(|&n| e.hits(self.halo(n), c, r))
                .partition(|&n| e.hits(self.bounds(n), c, r));
            all.brushed = merge(&all.brushed, &brushed);
            all.shell = merge(&all.shell, &shell);
            all.tested += nodes.len();
        }
        all.shell.retain(|n| all.brushed.binary_search(n).is_err());
        all
    }

    /// Run one kernel step (a dab). Writes positions and normals only. A node whose deltas are
    /// all zero is not written, so a step that moves nothing cannot leave an empty undo step
    /// behind.
    pub fn run_kernel<K: Kernel>(&mut self, k: &K, sc: &mut Scratch) -> StepReport {
        let q = self.gather(&k.spheres(), &k.free());
        self.work.boxes_tested += q.tested as u64;
        let (kset, skipped) = self.reachable(&q.brushed, k.reach());
        self.work.skipped += skipped.len() as u64;
        self.work.examined += (kset.iter())
            .map(|&n| self.cols.pos[n as usize].len() as u64)
            .sum::<u64>();
        let moved = if k.own_only() {
            self.fused(k, &kset)
        } else {
            self.two_pass(k, &kset, sc)
        };
        self.close(&q, &kset, &skipped, &moved)
    }

    /// One dab of `k` as `n` rounds (decision 0036, section 2, amended): one node query, then
    /// each round phase 1 over the parts of the nodes it found, every part keeping its state
    /// from round to round and writing its points' next positions beside the store's; after the
    /// last round each moved node's bounds and interior normals, then the seam normals, halos
    /// and marks, once. The same bits as `n` runs of [`MeshStore::run_kernel`] for a kernel
    /// that reads no normals: a round reads only the positions of the round before, and the
    /// rest comes from the same final positions. The rounds touch no node the first did not:
    /// only a node's own moves change its bounds and its halo's rim, and a point moves only
    /// inside the spheres, where the first query saw it.
    pub(crate) fn run_steps<K: Steps>(&mut self, k: &K, n: u32, sc: &mut Scratch) -> StepReport {
        let q = self.gather(&k.spheres(), &k.free());
        self.work.boxes_tested += q.tested as u64;
        let (kset, skipped) = self.reachable(&q.brushed, k.reach());
        self.work.skipped += skipped.len() as u64;
        let slots = (kset.iter())
            .map(|&n| self.cols.pos[n as usize].len() as u64)
            .sum::<u64>();
        // A part's kept state is built in its first round's task, on every thread.
        let mut parts = Vec::new();
        for (i, &m) in kset.iter().enumerate() {
            let len = self.cols.pos[m as usize].len() as u32;
            for a in (0..len).step_by(PART as usize) {
                parts.push(Part {
                    node: i,
                    slots: a..len.min(a.saturating_add(PART)),
                    kept: None,
                    out: Vec::new(),
                    moved: false,
                });
            }
        }
        // A node's next positions start as its positions: a round writes only the slots it
        // touches.
        sc.next.resize_with(sc.next.len().max(kset.len()), Vec::new);
        let next = &mut sc.next[..kset.len()];
        let pos = &self.cols.pos;
        gs_par::apply_each(next, |i, b| {
            b.clear();
            b.extend_from_slice(&pos[kset[i] as usize]);
        });
        let mut moved = vec![false; kset.len()];
        for r in 0..n {
            self.work.examined += slots;
            self.round(k, &kset, &mut parts, next);
            let mut now = vec![false; kset.len()];
            for p in &parts {
                now[p.node] |= p.moved;
            }
            // A node that moved takes its next positions, and keeps its last ones for the next
            // round to write over; they differ only in the slots its parts write every round.
            // A task each: the first swap copies the positions an open history step shares.
            let ids: Vec<NodeId> = (kset.iter().zip(&now))
                .filter(|e| *e.1)
                .map(|e| *e.0)
                .collect();
            let bufs = next.iter_mut().zip(&now).filter(|e| *e.1).map(|e| e.0);
            let mut swaps: Vec<_> = (select_mut(&mut self.cols.pos, &ids).into_iter())
                .zip(bufs)
                .collect();
            gs_par::apply_each(&mut swaps, |_, (p, b)| std::mem::swap(Arc::make_mut(p), *b));
            for (m, x) in moved.iter_mut().zip(now) {
                *m |= x;
            }
            if r == 0 {
                // A part the first round touched nothing in has nothing to move later.
                parts.retain(|p| !p.out.is_empty());
            }
        }
        self.settle(&kset, &moved);
        self.close(&q, &kset, &skipped, &moved)
    }

    /// One round of [`MeshStore::run_steps`] over `parts` (in node order and slot order): phase
    /// 1 for every part, which writes the next position of every slot it touches into `next`
    /// (per node of `nodes`), each slot by one part. One wait per round: no part reads `next`.
    fn round<K: Steps>(
        &self,
        k: &K,
        nodes: &[NodeId],
        parts: &mut [Part<K::Part>],
        next: &mut [Vec<Vec3>],
    ) {
        let surface = self.surface();
        let mut work = Vec::with_capacity(parts.len());
        let mut bufs = next.iter_mut();
        let (mut node, mut at, mut buf) = (usize::MAX, 0u32, <&mut [Vec3]>::default());
        for p in parts {
            while node != p.node {
                // Parts are in node order, every node of `nodes` with a buffer.
                (node, at) = (node.wrapping_add(1), 0);
                buf = bufs.next().map(Vec::as_mut_slice).unwrap_or_default();
            }
            let (_, rest) = std::mem::take(&mut buf).split_at_mut((p.slots.start - at) as usize);
            let (mine, rest) = rest.split_at_mut(p.slots.len());
            (at, buf) = (p.slots.end, rest);
            work.push((p, mine));
        }
        gs_par::apply_each(&mut work, |_, (p, mine)| {
            let Part {
                node,
                slots,
                kept,
                out,
                moved,
            } = &mut **p;
            let view = surface.node(nodes[*node]);
            let kept = kept.get_or_insert_with(|| k.part(&view, slots.clone()));
            out.clear();
            k.deltas(&surface, &view, kept, out);
            let a = slots.start as usize;
            *moved = false;
            // A zero delta writes the position as it is, so a point that does not move keeps its
            // bits (adding 0 turns -0 into +0; review 0003 m4).
            for &(l, d) in out.iter() {
                let x = view.pos[l as usize];
                mine[l as usize - a] = if d == Vec3::ZERO {
                    x
                } else {
                    *moved = true;
                    x + d
                };
            }
        });
    }

    /// After a dab's rounds: the bounds and interior normals of the nodes of `nodes` that
    /// `moved`, from their final positions.
    fn settle(&mut self, nodes: &[NodeId], moved: &[bool]) {
        let ids: Vec<NodeId> = (nodes.iter().zip(moved))
            .filter(|e| *e.1)
            .map(|e| *e.0)
            .collect();
        let cfg = self.cfg;
        let Columns { pos, nrm, topo, .. } = &mut self.cols;
        let pos = &*pos;
        let mut work: Vec<_> = (ids.iter().copied().zip(select_mut(nrm, &ids)))
            .map(|(n, nr)| (n, nr, Aabb::EMPTY))
            .collect();
        gs_par::apply_each(&mut work, |_, (n, nr, b)| {
            let (p, faces) = (&pos[*n as usize], &topo[*n as usize].faces);
            for x in p.iter() {
                b.grow(*x);
            }
            normals::interior(cfg, *n, p, Arc::make_mut(nr).as_mut_slice(), faces);
        });
        for (n, _, b) in work {
            self.der.bounds[n as usize] = b;
        }
    }

    /// The end of a kernel step whose reachable nodes `kset` (of query `q`, beside the
    /// `skipped`) `moved` or not: seam normals, halos and marks.
    fn close(
        &mut self,
        q: &NodeQuery,
        kset: &[NodeId],
        skipped: &[NodeId],
        moved: &[bool],
    ) -> StepReport {
        let (mut brushed, mut still) = (Vec::new(), Vec::new());
        for (&n, &m) in kset.iter().zip(moved) {
            if m {
                brushed.push(n);
            } else {
                still.push(n);
            }
        }
        if brushed.is_empty() {
            return StepReport::default();
        }
        // A node that did not move, skipped or still, can have a neighbour that did, which
        // turns its seam normals: it joins the shell.
        let shell = merge(&merge(&q.shell, skipped), &still);
        self.work.normals += (brushed.iter())
            .map(|&n| (self.cols.pos[n as usize].len() - self.der.seam[n as usize].len()) as u64)
            .sum::<u64>();
        let all = merge(&brushed, &shell);
        self.normals(&[], &all);
        // The brushed nodes' bounds came out of the apply pass; the shell's points did not
        // move, so only halos can change there.
        self.refresh_halos(&all, false);
        self.mark(&brushed, ColumnSet::POSITION | ColumnSet::NORMAL);
        self.mark(&shell, ColumnSet::NORMAL);
        StepReport { brushed, shell }
    }

    /// Phase 1 for every node of `nodes` into `sc`, by parts when the kernel can, then phase 2
    /// for those that move. Returns which moved.
    fn two_pass<K: Kernel>(&mut self, k: &K, nodes: &[NodeId], sc: &mut Scratch) -> Vec<bool> {
        sc.deltas
            .resize_with(sc.deltas.len().max(nodes.len()), Vec::new);
        let deltas = &mut sc.deltas[..nodes.len()];
        let surface = self.surface();
        let size = if k.parts() { PART as usize } else { usize::MAX };
        let mut parts = Vec::new();
        for (i, d) in deltas.iter_mut().enumerate() {
            d.resize(self.cols.pos[nodes[i] as usize].len(), Vec3::ZERO);
            let (mut rest, mut at) = (d.as_mut_slice(), 0);
            while !rest.is_empty() {
                let len = size.min(rest.len());
                let (mine, tail) = rest.split_at_mut(len);
                parts.push((i, at, mine));
                (rest, at) = (tail, at + len);
            }
        }
        gs_par::apply_each(&mut parts, |_, (i, at, out)| {
            let view = surface.node(nodes[*i]);
            out.fill(Vec3::ZERO);
            if size == usize::MAX {
                k.deltas(&surface, &view, out);
            } else {
                let a = *at as u32;
                k.part_deltas(&surface, &view, a..a + out.len() as u32, out);
            }
        });
        drop(parts);
        let moved: Vec<bool> = deltas.iter().map(|d| moves(d)).collect();
        let pick: Vec<usize> = (0..nodes.len()).filter(|&i| moved[i]).collect();
        let ids: Vec<NodeId> = pick.iter().map(|&i| nodes[i]).collect();
        let cfg = self.cfg;
        let Columns { pos, nrm, topo, .. } = &mut self.cols;
        let cols = select_mut(pos, &ids).into_iter().zip(select_mut(nrm, &ids));
        let mut work: Vec<_> = (ids.iter().copied().zip(cols))
            .map(|(n, cols)| (n, cols, Aabb::EMPTY))
            .collect();
        gs_par::apply_each(&mut work, |i, (n, (p, nr), b)| {
            *b = apply(cfg, *n, p, nr, &topo[*n as usize].faces, &deltas[pick[i]]);
        });
        for (n, _, b) in work {
            self.der.bounds[n as usize] = b;
        }
        moved
    }

    /// Phases 1 and 2 in one task per node, while the node is in cache (decision 0022, change
    /// 8), for a kernel that reads only its own vertex: the nodes' positions and normals are
    /// taken out of the columns while their tasks run. Returns which of `nodes` moved.
    fn fused<K: Kernel>(&mut self, k: &K, nodes: &[NodeId]) -> Vec<bool> {
        let cfg = self.cfg;
        let mut work: Vec<_> = (nodes.iter())
            .map(|&n| {
                let c = n as usize;
                let (p, nr) = (&mut self.cols.pos[c], &mut self.cols.nrm[c]);
                (n, std::mem::take(p), std::mem::take(nr), None)
            })
            .collect();
        let (surface, topo) = (self.surface(), &self.cols.topo);
        // A panicking kernel must not leave the columns empty: put them back first.
        let run = catch_unwind(AssertUnwindSafe(|| {
            gs_par::apply_each(&mut work, |_, (n, p, nr, b)| {
                let mut d = vec![Vec3::ZERO; p.len()];
                let view = NodeView {
                    pos: p.as_slice(),
                    nrm: nr.as_slice(),
                    ..surface.node(*n)
                };
                k.deltas(&surface, &view, &mut d);
                if moves(&d) {
                    *b = Some(apply(cfg, *n, p, nr, &topo[*n as usize].faces, &d));
                }
            });
        }));
        let mut moved = Vec::with_capacity(nodes.len());
        for (n, p, nr, b) in work {
            let c = n as usize;
            (self.cols.pos[c], self.cols.nrm[c]) = (p, nr);
            if let Some(b) = b {
                self.der.bounds[c] = b;
            }
            moved.push(b.is_some());
        }
        if let Err(panic) = run {
            resume_unwind(panic);
        }
        moved
    }
}

/// Whether a node's deltas move any point.
fn moves(d: &[Vec3]) -> bool {
    d.iter().any(|x| *x != Vec3::ZERO)
}

/// Phase 2 for node `n`: add its deltas `d` to positions `p`, then recompute its interior normals
/// `nr` from its `faces`. Returns the box around its positions.
fn apply(
    cfg: NodeConfig,
    n: NodeId,
    p: &mut Arc<Vec<Vec3>>,
    nr: &mut Arc<Vec<Vec3>>,
    faces: &[[u32; 4]],
    d: &[Vec3],
) -> Aabb {
    let p = Arc::make_mut(p);
    let mut b = Aabb::EMPTY;
    // A zero delta writes nothing, so a point that does not move keeps its bits (adding 0 turns
    // -0 into +0; review 0003 m4).
    for (x, d) in p.iter_mut().zip(d) {
        if *d != Vec3::ZERO {
            *x += *d;
        }
        b.grow(*x);
    }
    normals::interior(cfg, n, p, Arc::make_mut(nr).as_mut_slice(), faces);
    b
}
