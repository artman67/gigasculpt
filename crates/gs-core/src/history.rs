//! Undo history by copy-on-write (decision 0001, layout B).
//!
//! Opening a step clones every node's column `Arc`s: a reference-count bump each. Every write
//! goes through `Arc::make_mut`, which copies a node's column the first time it is written while
//! the step still holds it. Closing the step keeps only the old `Arc`s whose pointer changed.
//! Undo and redo swap those pointers back and forth; nothing is copied and nothing recomputed
//! except bounds. The open step's positions double as the stroke-start snapshot.
//!
//! This is the hook the engine's write guards call: [`MeshStore::begin_step`] before a command
//! writes, then [`MeshStore::end_step`], or [`MeshStore::cancel_step`] to roll back on error.
//!
//! Every closed step has an id, unique in the process, so whoever keeps a list of steps (the
//! engine's history) can name the one it means: to count what it holds, to drop it, or to move
//! it from a share of the store back onto the store ([`MeshStore::adopt`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::columns::{AttrId, AttrVec, ColumnSet, Columns, NodeTopo};
use crate::config::{MeshError, NodeId};
use crate::derived::neighbours;
use crate::math::Vec3;
use crate::par::merge;
use crate::store::MeshStore;
use crate::view::PosReader;

/// The id of a closed history step. Unique among every step the process closes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StepId(u64);

impl StepId {
    fn next() -> StepId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        StepId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One allocation a store's columns or history hold: its address, which tells copies that
/// share it apart from copies that do not, and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub at: usize,
    pub bytes: usize,
}

/// A column value history can hold: compared by pointer, sized by content.
trait Held {
    fn same(&self, o: &Self) -> bool;
    fn bytes(&self) -> usize;
    /// Where the value's allocation is, if it holds one.
    fn address(&self) -> Option<usize>;
    /// The allocation, if the value holds one.
    fn allocation(&self) -> Option<Allocation> {
        let at = self.address()?;
        Some(Allocation {
            at,
            bytes: self.bytes(),
        })
    }
}

impl Held for Arc<Vec<Vec3>> {
    fn same(&self, o: &Self) -> bool {
        Arc::ptr_eq(self, o)
    }
    fn bytes(&self) -> usize {
        self.len() * size_of::<Vec3>()
    }
    fn address(&self) -> Option<usize> {
        Some(Arc::as_ptr(self).addr())
    }
}

impl Held for Arc<NodeTopo> {
    fn same(&self, o: &Self) -> bool {
        Arc::ptr_eq(self, o)
    }
    fn bytes(&self) -> usize {
        let t = &**self;
        (t.faces.len() * 4 + t.ring.len() + t.pool.len() + t.vfree.len() + t.ffree.len()) * 4
    }
    fn address(&self) -> Option<usize> {
        Some(Arc::as_ptr(self).addr())
    }
}

impl Held for Option<Arc<AttrVec>> {
    fn same(&self, o: &Self) -> bool {
        match (self, o) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        }
    }
    fn bytes(&self) -> usize {
        self.as_ref().map_or(0, |a| a.bytes())
    }
    fn address(&self) -> Option<usize> {
        Some(Arc::as_ptr(self.as_ref()?).addr())
    }
}

/// The allocation of one node's topology column (a level's skeleton holds them too).
pub(crate) fn topo_allocation(t: &Arc<NodeTopo>) -> Allocation {
    Allocation {
        at: Arc::as_ptr(t).addr(),
        bytes: t.bytes(),
    }
}

/// Every allocation of every column in `c`.
fn columns_allocations(c: &Columns, f: &mut impl FnMut(Allocation)) {
    let (pos, nrm) = (c.pos.iter(), c.nrm.iter());
    pos.chain(nrm)
        .filter_map(Held::allocation)
        .for_each(&mut *f);
    c.topo.iter().filter_map(Held::allocation).for_each(&mut *f);
    let attrs = c.attrs.iter().flatten();
    attrs.filter_map(Held::allocation).for_each(f);
}

/// Every allocation a diff holds.
fn diff_allocations<C: Held>(d: &Diff<C>, f: &mut impl FnMut(Allocation)) {
    d.iter().filter_map(|(_, c)| c.allocation()).for_each(f);
}

/// Append `(node, cols)` for every node in `d`.
fn note<C>(out: &mut Vec<(NodeId, ColumnSet)>, d: &[(u32, C)], cols: ColumnSet) {
    out.extend(d.iter().map(|e| (e.0, cols)));
}

/// `(node, old value)` for every node whose entry changed.
type Diff<C> = Vec<(u32, C)>;

fn diff<C: Held>(before: Vec<C>, now: &[C]) -> Diff<C> {
    (before.into_iter().zip(now).enumerate())
        .filter(|(_, (b, n))| !b.same(n))
        .map(|(k, (b, _))| (k as u32, b))
        .collect()
}

fn swap<C>(now: &mut [C], saved: &mut Diff<C>) {
    for (k, c) in saved {
        std::mem::swap(&mut now[*k as usize], c);
    }
}

fn bytes<C: Held>(d: &Diff<C>) -> usize {
    d.iter().map(|(_, c)| c.bytes()).sum()
}

/// Bytes of every column of every node in `c`.
fn columns_bytes(c: &Columns) -> usize {
    let col = |v: &[Arc<Vec<Vec3>>]| v.iter().map(Held::bytes).sum::<usize>();
    let attrs: usize = c.attrs.iter().flatten().map(Held::bytes).sum();
    col(&c.pos) + col(&c.nrm) + c.topo.iter().map(Held::bytes).sum::<usize>() + attrs
}

/// One undo step: the changed columns as they are on the other side of the step.
pub(crate) struct Step {
    /// Set when the step closes.
    id: StepId,
    /// Node count on the other side.
    other_len: usize,
    pos: Diff<Arc<Vec<Vec3>>>,
    nrm: Diff<Arc<Vec<Vec3>>>,
    topo: Diff<Arc<NodeTopo>>,
    attrs: Vec<Diff<Option<Arc<AttrVec>>>>,
    /// Nodes past the smaller side's count, while they are out of the mesh.
    tail: Option<Columns>,
}

impl Step {
    fn new(mut before: Columns, now: &Columns) -> Step {
        let other_len = before.len();
        let tail = (other_len > now.len()).then(|| before.split_off(now.len()));
        let blank = vec![None; before.len()];
        before.attrs.resize(now.attrs.len(), blank);
        Step {
            id: StepId::default(),
            other_len,
            pos: diff(before.pos, &now.pos),
            nrm: diff(before.nrm, &now.nrm),
            topo: diff(before.topo, &now.topo),
            attrs: (before.attrs.into_iter().zip(&now.attrs))
                .map(|(b, n)| diff(b, n))
                .collect(),
            tail,
        }
    }

    fn bytes(&self) -> usize {
        let tail = self.tail.as_ref().map_or(0, columns_bytes);
        let attrs: usize = self.attrs.iter().map(bytes).sum();
        bytes(&self.pos) + bytes(&self.nrm) + bytes(&self.topo) + attrs + tail
    }

    fn allocations(&self, f: &mut impl FnMut(Allocation)) {
        diff_allocations(&self.pos, f);
        diff_allocations(&self.nrm, f);
        diff_allocations(&self.topo, f);
        for a in &self.attrs {
            diff_allocations(a, f);
        }
        if let Some(tail) = &self.tail {
            columns_allocations(tail, f);
        }
    }

    /// The columns on the other side of this step from `now`, leaving both as they are.
    fn other_side(&self, now: &Columns) -> Columns {
        fn put<C: Clone>(cols: &mut [C], d: &Diff<C>) {
            for (k, c) in d {
                cols[*k as usize] = c.clone();
            }
        }
        let mut c = now.clone();
        match self.other_len.cmp(&c.len()) {
            std::cmp::Ordering::Less => drop(c.split_off(self.other_len)),
            std::cmp::Ordering::Greater => c.append(self.tail.clone().unwrap_or_default()),
            std::cmp::Ordering::Equal => {}
        }
        put(&mut c.pos, &self.pos);
        put(&mut c.nrm, &self.nrm);
        put(&mut c.topo, &self.topo);
        for (a, d) in c.attrs.iter_mut().zip(&self.attrs) {
            put(a, d);
        }
        c
    }

    fn summary(&self, now_len: usize) -> StepSummary {
        StepSummary {
            id: self.id,
            nodes: step_nodes(self, now_len),
            bytes: self.bytes(),
            topology: !self.topo.is_empty() || self.other_len != now_len,
        }
    }

    /// Swap this step's columns into `cols`. Returns the changed nodes that exist afterwards,
    /// each with the columns that changed, and whether topology changed.
    fn flip(&mut self, cols: &mut Columns) -> (Vec<(NodeId, ColumnSet)>, bool) {
        let cur = cols.len();
        match self.other_len.cmp(&cur) {
            std::cmp::Ordering::Less => self.tail = Some(cols.split_off(self.other_len)),
            std::cmp::Ordering::Greater => cols.append(self.tail.take().unwrap_or_default()),
            std::cmp::Ordering::Equal => {}
        }
        swap(&mut cols.pos, &mut self.pos);
        swap(&mut cols.nrm, &mut self.nrm);
        swap(&mut cols.topo, &mut self.topo);
        for (a, d) in cols.attrs.iter_mut().zip(&mut self.attrs) {
            swap(a, d);
        }
        let mut changed: Vec<(NodeId, ColumnSet)> = Vec::new();
        note(&mut changed, &self.pos, ColumnSet::POSITION);
        note(&mut changed, &self.nrm, ColumnSet::NORMAL);
        note(&mut changed, &self.topo, ColumnSet::TOPOLOGY);
        for (a, d) in self.attrs.iter().enumerate() {
            note(&mut changed, d, ColumnSet::attr(AttrId(a as u8)));
        }
        let every = ColumnSet::POSITION | ColumnSet::NORMAL | ColumnSet::TOPOLOGY;
        changed.extend((cur..self.other_len).map(|k| (k as u32, every)));
        let topo = !self.topo.is_empty() || cur != self.other_len;
        self.other_len = cur;
        (changed, topo)
    }
}

/// Undo and redo stacks, the open step's snapshot, and the start of a stroke drawn inside it.
#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    open: Option<Columns>,
    /// The columns when a stroke began inside a step its caller had already opened (a batch
    /// that edits before it strokes). The stroke reverts to it and reads it as its start.
    mark: Option<Columns>,
}

/// What closing a step kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepSummary {
    /// The step's id (meaningless for a step that changed nothing, which is not kept).
    pub id: StepId,
    /// Nodes with at least one changed column.
    pub nodes: usize,
    /// Bytes the step holds.
    pub bytes: usize,
    /// Whether any node's faces or rings changed, or nodes were added or removed.
    pub topology: bool,
}

impl MeshStore {
    /// Open a history step. Every write until [`MeshStore::end_step`] becomes one undo step.
    ///
    /// # Panics
    /// If a step is already open.
    pub fn begin_step(&mut self) {
        assert!(self.hist.open.is_none(), "a history step is already open");
        self.hist.open = Some(self.cols.clone());
    }

    /// Close the open step. A step that changed nothing is dropped and leaves redo alone.
    ///
    /// # Panics
    /// If no step is open.
    pub fn end_step(&mut self) -> StepSummary {
        let before = self.hist.open.take();
        let Some(before) = before else {
            panic!("no history step is open");
        };
        self.hist.mark = None;
        let mut step = Step::new(before, &self.cols);
        step.id = StepId::next();
        let summary = step.summary(self.node_count());
        if summary.nodes > 0 {
            self.hist.undo.push(step);
            self.hist.redo.clear();
        }
        summary
    }

    /// Takes the edit `copy` made as this store's next undo step, exactly as if the edit had run
    /// here: `copy` is a [`MeshStore::share`] of this store that closed one step (a job's copy,
    /// decision 0010). Its columns become this store's, its step goes on top of this store's
    /// undo stack, and the redo stack is cleared, as [`MeshStore::end_step`] does.
    ///
    /// Refuses, changing nothing, unless this store is still exactly the state the copy started
    /// from (the step's before side holds this store's very columns), neither store has a step
    /// open, and the copy holds just that one step and kept this store's attribute columns.
    pub fn adopt(&mut self, copy: MeshStore) -> Result<StepSummary, MeshError> {
        if !self.is_base_of(&copy) {
            return Err(MeshError::Input(
                "cannot adopt the copy: this store changed since it was taken, or it is not a \
                 share edited in one closed step"
                    .into(),
            ));
        }
        let MeshStore {
            defs,
            cols,
            der,
            dirty,
            mut hist,
            work,
            ..
        } = copy;
        let Some(step) = hist.undo.pop() else {
            return Err(MeshError::Input(
                "cannot adopt the copy: it holds no step".into(),
            ));
        };
        let summary = step.summary(cols.len());
        self.defs = defs;
        self.cols = cols;
        self.der = der;
        self.work = work;
        self.dirty
            .resize(self.dirty.len().max(dirty.len()), ColumnSet::EMPTY);
        for (mine, theirs) in self.dirty.iter_mut().zip(dirty) {
            *mine |= theirs;
        }
        self.hist.undo.push(step);
        self.hist.redo.clear();
        Ok(summary)
    }

    /// True if [`MeshStore::adopt`] would take `copy`: it holds one closed step and no open one,
    /// kept this store's node size and attribute columns, and its step's before side holds
    /// this store's very columns (the same allocation everywhere; attribute columns the copy
    /// added unallocated), and this store has no step open.
    pub fn is_base_of(&self, copy: &MeshStore) -> bool {
        let h = &copy.hist;
        let [step] = &h.undo[..] else {
            return false;
        };
        if !h.redo.is_empty() || h.open.is_some() || self.hist.open.is_some() {
            return false;
        }
        let before = step.other_side(&copy.cols);
        let added = before
            .attrs
            .get(self.cols.attrs.len()..)
            .unwrap_or_default();
        let mut mine = before.clone();
        mine.attrs.truncate(self.cols.attrs.len());
        copy.cfg == self.cfg
            && copy.defs.get(..self.defs.len()) == Some(&self.defs[..])
            && self.cols.same(&mine)
            && added.iter().flatten().all(Option::is_none)
    }

    /// Drops every step that could be redone. The engine calls this when its own list of steps
    /// drops them, so no redo step stays behind that nothing can reach.
    pub fn clear_redo(&mut self) {
        self.hist.redo.clear();
    }

    /// Drops every undo and redo step: they name nodes by a numbering an edit outside an open
    /// step has replaced, so none of them could be undone or redone (see
    /// [`MeshStore::resort`]).
    pub(crate) fn forget_history(&mut self) {
        self.hist.undo.clear();
        self.hist.redo.clear();
    }

    /// Drops the oldest undo step if it is step `id`, for a history over its budget. Returns
    /// whether it did.
    pub fn drop_step(&mut self, id: StepId) -> bool {
        let oldest = self.hist.undo.first().is_some_and(|s| s.id == id);
        if oldest {
            self.hist.undo.remove(0);
        }
        oldest
    }

    /// Calls `f` with every allocation the current columns hold.
    pub fn column_allocations(&self, mut f: impl FnMut(Allocation)) {
        columns_allocations(&self.cols, &mut f);
    }

    /// Calls `f` with every allocation the columns held when the open step began: what is
    /// committed, before the edit in progress (the current columns when no step is open).
    pub fn start_allocations(&self, mut f: impl FnMut(Allocation)) {
        columns_allocations(self.hist.open.as_ref().unwrap_or(&self.cols), &mut f);
    }

    /// Calls `f` with every allocation each closed undo or redo step holds, and the step's id.
    pub fn step_allocations(&self, mut f: impl FnMut(StepId, Allocation)) {
        for step in self.hist.undo.iter().chain(&self.hist.redo) {
            step.allocations(&mut |a| f(step.id, a));
        }
    }

    /// Roll the open step back: the mesh returns to exactly what it was at `begin_step`.
    ///
    /// # Panics
    /// If no step is open.
    pub fn cancel_step(&mut self) {
        self.hist.mark = None;
        self.revert_step();
        self.hist.open = None;
    }

    /// Put every column back exactly as it was when the open step began, or when the stroke
    /// marked inside it began, and keep the step open. Drag and anchored strokes call this
    /// before each update, so their result depends only on the stroke-start snapshot and the
    /// latest pointer position.
    ///
    /// # Panics
    /// If no step is open.
    pub fn revert_step(&mut self) {
        assert!(self.hist.open.is_some(), "no history step is open");
        let mut step = Step::new(self.snapshot().clone(), &self.cols);
        self.apply_flip(&mut step);
    }

    /// Mark the start of a stroke inside the open step: until [`MeshStore::clear_mark`],
    /// [`MeshStore::revert_step`] goes back to here and the start readers read here.
    pub(crate) fn set_mark(&mut self) {
        self.hist.mark = Some(self.cols.clone());
    }

    pub(crate) fn clear_mark(&mut self) {
        self.hist.mark = None;
    }

    /// Undo the last step. `None` when there is nothing to undo.
    ///
    /// # Panics
    /// While a step is open.
    pub fn undo(&mut self) -> Option<Vec<NodeId>> {
        self.travel(true)
    }

    /// Redo the last undone step. `None` when there is nothing to redo.
    ///
    /// # Panics
    /// While a step is open.
    pub fn redo(&mut self) -> Option<Vec<NodeId>> {
        self.travel(false)
    }

    /// Move one step back (`undo`) or forward: flip the top step of one stack onto the other.
    fn travel(&mut self, back: bool) -> Option<Vec<NodeId>> {
        assert!(
            self.hist.open.is_none(),
            "undo or redo while a history step is open"
        );
        let h = &mut self.hist;
        let mut s = if back { h.undo.pop() } else { h.redo.pop() }?;
        let nodes = self.apply_flip(&mut s);
        let h = &mut self.hist;
        if back { &mut h.redo } else { &mut h.undo }.push(s);
        Some(nodes)
    }

    /// Flip a step and refresh what is derived. Returns the changed nodes, ascending.
    fn apply_flip(&mut self, s: &mut Step) -> Vec<NodeId> {
        let before = self.node_count();
        let (changed, topo) = s.flip(&mut self.cols);
        let mut nodes: Vec<NodeId> = changed.iter().map(|c| c.0).collect();
        nodes.sort_unstable();
        nodes.dedup();
        // After a topology change, halos around the changed nodes can change even where no
        // column of theirs did. A move alone changes the normals of every node whose halo it
        // can change, so those nodes are in the step already.
        self.refresh_bounds(&nodes, topo);
        let nb = if topo {
            neighbours(self.cfg, &self.der, &self.topo(), &nodes)
        } else {
            Vec::new()
        };
        self.refresh_halos(&merge(&nodes, &nb), self.node_count() != before);
        let shape = if topo {
            ColumnSet::TOPOLOGY
        } else {
            ColumnSet::EMPTY
        };
        let cols = (changed.iter()).fold(shape, |s, c| s | c.1);
        self.refresh_state(&nodes, cols);
        self.dirty.resize(self.node_count(), ColumnSet::EMPTY);
        for (n, cols) in changed {
            self.dirty[n as usize] |= cols;
        }
        nodes
    }

    pub fn undo_len(&self) -> usize {
        self.hist.undo.len()
    }
    pub fn redo_len(&self) -> usize {
        self.hist.redo.len()
    }
    pub fn step_open(&self) -> bool {
        self.hist.open.is_some()
    }

    /// Bytes held only by history: the columns of every undo and redo step.
    pub fn history_bytes(&self) -> usize {
        self.hist
            .undo
            .iter()
            .chain(&self.hist.redo)
            .map(Step::bytes)
            .sum()
    }

    /// Bytes the mesh's own columns hold (derived data and history not included).
    pub fn mesh_bytes(&self) -> usize {
        columns_bytes(&self.cols)
    }

    /// Drop the oldest undo steps until history holds at most `max` bytes, then redo steps
    /// (furthest first) if that is not enough.
    pub fn trim_history(&mut self, max: usize) {
        while self.history_bytes() > max && !self.hist.undo.is_empty() {
            self.hist.undo.remove(0);
        }
        while self.history_bytes() > max && !self.hist.redo.is_empty() {
            self.hist.redo.remove(0);
        }
    }

    /// The columns as they were when the marked stroke or else the open step began, or the
    /// current ones.
    fn snapshot(&self) -> &Columns {
        let start = self.hist.mark.as_ref().or(self.hist.open.as_ref());
        start.unwrap_or(&self.cols)
    }

    /// Node `n`'s values of attribute `a` as they were when the marked stroke or else the open
    /// step began (the current ones when no step is open); `None` where the node held none
    /// then, which reads as the empty value (zero, white for a color).
    pub fn start_attr(&self, a: AttrId, n: NodeId) -> Option<&AttrVec> {
        let col = self.snapshot().attrs.get(a.index())?;
        col.get(n as usize)?.as_deref()
    }

    /// Positions as they were when the marked stroke or else the open step began (the
    /// stroke-start snapshot), or the current positions when no step is open.
    pub fn start_pos(&self) -> PosReader<'_> {
        PosReader {
            cfg: self.cfg,
            cols: &self.snapshot().pos,
        }
    }

    /// Whether node `n`'s faces or rings differ from the start the start readers read (a node
    /// added since counts). The start readers index by current ids, which in such a node name
    /// other points than they did at the start.
    pub fn start_topology_changed(&self, n: NodeId) -> bool {
        let start = &self.snapshot().topo;
        let now = &self.cols.topo[n as usize];
        start.get(n as usize).is_none_or(|t| !Arc::ptr_eq(t, now))
    }

    /// Node `n`'s vertex slots when the marked stroke or else the open step began (0 for a node
    /// added since).
    pub(crate) fn start_slots(&self, n: NodeId) -> usize {
        (self.snapshot().topo.get(n as usize)).map_or(0, |t| t.ring.len())
    }

    /// Normals as they were when the marked stroke or else the open step began, or the current
    /// ones when no step is open.
    pub fn start_nrm(&self) -> PosReader<'_> {
        PosReader {
            cfg: self.cfg,
            cols: &self.snapshot().nrm,
        }
    }
}

/// Distinct nodes a step changed, added or removed; the mesh now has `now_len` nodes.
fn step_nodes(s: &Step, now_len: usize) -> usize {
    let mut v: Vec<u32> = (s.pos.iter().map(|e| e.0))
        .chain(s.nrm.iter().map(|e| e.0))
        .chain(s.topo.iter().map(|e| e.0))
        .chain(s.attrs.iter().flatten().map(|e| e.0))
        .collect();
    v.sort_unstable();
    v.dedup();
    v.len() + s.other_len.abs_diff(now_len)
}
