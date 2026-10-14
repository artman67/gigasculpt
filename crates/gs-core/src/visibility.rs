//! What is hidden (decision 0015): the state derived from the `hide` and `mask` columns, the
//! faces a neighbour walk may cross, and the edits that hide and show faces.
//!
//! Hidden is stored once, as one bit per face. Derived from it, per node: which vertex slots are
//! hidden (a vertex is hidden when its ring is not empty and every face in it is hidden), and a
//! summary of the node: hidden faces, hidden points and the mask, each none, some or all. The
//! summary lets brushes skip whole nodes. None of it is undone: whatever changes a column
//! refreshes it (the write guard, the topology editor, undo and redo, full rebuilds).

use std::sync::Arc;

use crate::bits::Bits;
use crate::columns::{AttrDef, AttrId, AttrVec, ColumnSet, NodeTopo};
use crate::config::{FaceId, NodeConfig, NodeId};
use crate::math::{NO, Vec3};
use crate::par::merge;
use crate::select::FaceSet;
use crate::store::MeshStore;
use crate::write::NewValues;

/// None, some or all of a node's elements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cover {
    #[default]
    None,
    Some,
    All,
}

impl Cover {
    /// `count` of `of` elements.
    fn of(count: usize, of: usize) -> Cover {
        match count {
            0 => Cover::None,
            c if c == of => Cover::All,
            _ => Cover::Some,
        }
    }
}

/// A node's summary (decision 0015): hidden faces among its live faces, hidden points among its
/// live points, and its mask among its live points (`None` all free, `All` all fully masked).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeState {
    pub faces: Cover,
    pub points: Cover,
    pub mask: Cover,
}

/// The derived state of every node.
#[derive(Clone, Default)]
pub(crate) struct VisState {
    /// Hidden vertex slots per node; `None` where none is hidden.
    pub hidden: Vec<Option<Bits>>,
    pub state: Vec<NodeState>,
    /// Some face of the mesh is hidden.
    pub any: bool,
    /// The largest face group id among each node's live faces, hidden ones included (0 when
    /// none), so a fresh id is one read per node (decision 0026).
    pub groups: Vec<u32>,
}

/// The faces a walk may cross: the visible ones.
#[derive(Clone, Copy)]
pub struct Visible<'a> {
    cfg: NodeConfig,
    /// Every node's hide column, or `None` when no face is hidden.
    hide: Option<&'a [Option<Arc<AttrVec>>]>,
}

impl Visible<'_> {
    /// Whether face `f` is visible.
    #[inline]
    pub fn face(&self, f: FaceId) -> bool {
        let Some(cols) = self.hide else {
            return true;
        };
        match cols[self.cfg.face_node(f) as usize].as_deref() {
            Some(AttrVec::Bits(b)) => !b.get(self.cfg.face_local(f) as usize),
            _ => true,
        }
    }
}

/// Whether a mask value holds its vertex completely: at or above 1, or not a number (which the
/// influence stage treats as fully masked).
fn full(m: f32) -> bool {
    m >= 1.0 || m.is_nan()
}

impl MeshStore {
    /// The hide column, if registered.
    pub(crate) fn hide_id(&self) -> Option<AttrId> {
        self.attr_id(&AttrDef::hide().name)
    }

    /// The faces a neighbour walk may cross.
    pub fn visible(&self) -> Visible<'_> {
        let hide = (self.der.vis.any)
            .then(|| self.hide_id().map(|h| &self.cols.attrs[h.index()][..]))
            .flatten();
        Visible {
            cfg: self.cfg,
            hide,
        }
    }

    /// Node `n`'s summary.
    pub fn node_state(&self, n: NodeId) -> NodeState {
        self.der.vis.state[n as usize]
    }

    /// Whether any face is hidden.
    pub fn any_hidden(&self) -> bool {
        self.der.vis.any
    }

    /// Bring the derived state of `nodes` (ascending) up to date after `cols` changed in them:
    /// for hide or topology, their face summaries and the hidden-point bits of every vertex that
    /// uses one of their faces; for the mask or topology, their mask summaries; for face groups
    /// or topology, their largest group ids.
    pub(crate) fn refresh_state(&mut self, nodes: &[NodeId], cols: ColumnSet) {
        let nn = self.node_count();
        self.der.vis.hidden.resize(nn, None);
        self.der.vis.state.resize(nn, NodeState::default());
        self.der.vis.groups.resize(nn, 0);
        let topo = cols.contains(ColumnSet::TOPOLOGY);
        let hide = self.hide_id();
        let mask = self.attr_id(&AttrDef::mask().name);
        let group = self.attr_id(&AttrDef::face_group().name);
        let has = |a: Option<AttrId>| a.is_some_and(|a| cols.contains(ColumnSet::attr(a)));
        if topo || has(hide) {
            self.refresh_hidden(nodes, hide);
        }
        if topo || has(group) {
            let cols = &self.cols;
            let largest = gs_par::map(nodes.len(), |i| {
                let k = nodes[i] as usize;
                let col = group.and_then(|g| cols.attrs[g.index()][k].as_deref());
                crate::groups::largest_in(&cols.topo[k], col)
            });
            for (&n, g) in nodes.iter().zip(largest) {
                self.der.vis.groups[n as usize] = g;
            }
        }
        if topo || has(mask) {
            let cols = &self.cols;
            let covers = gs_par::map(nodes.len(), |i| {
                let k = nodes[i] as usize;
                mask_cover(
                    &cols.pos[k],
                    mask.and_then(|m| cols.attrs[m.index()][k].as_deref()),
                )
            });
            for (&n, c) in nodes.iter().zip(covers) {
                self.der.vis.state[n as usize].mask = c;
            }
        }
    }

    /// Face summaries of `nodes`, then hidden-point bits of their nodes and of the nodes owning
    /// the other vertices of their faces.
    fn refresh_hidden(&mut self, nodes: &[NodeId], hide: Option<AttrId>) {
        let col = |k: usize| hide.and_then(|h| self.cols.attrs[h.index()][k].as_deref());
        let faces = gs_par::map(nodes.len(), |i| {
            let k = nodes[i] as usize;
            face_cover(&self.cols.topo[k], col(k))
        });
        for (&n, c) in nodes.iter().zip(faces) {
            self.der.vis.state[n as usize].faces = c;
        }
        self.der.vis.any = self.der.vis.state.iter().any(|s| s.faces != Cover::None);
        let mut around: Vec<NodeId> = (nodes.iter())
            .flat_map(|&n| self.der.shared[n as usize].iter())
            .map(|&v| self.cfg.node_of(v))
            .collect();
        around.sort_unstable();
        around.dedup();
        let all = merge(nodes, &around);
        let visible = self.visible();
        let points = gs_par::map(all.len(), |i| {
            let k = all[i] as usize;
            hidden_points(&self.cols.topo[k], &self.cols.pos[k], visible)
        });
        for (&n, (bits, cover)) in all.iter().zip(points) {
            self.der.vis.hidden[n as usize] = bits;
            self.der.vis.state[n as usize].points = cover;
        }
    }

    /// Change which faces are hidden, as one write. A node left with no hidden face releases its
    /// column. Returns the nodes written.
    ///
    /// A part with no hide column reads as nothing hidden, exactly as one whose column is empty,
    /// and gets the column with its first hidden face ([`MeshStore::put_column`]). So every
    /// edit, Invert included, gives the same result on the same faces whether or not the
    /// column was ever made: Invert on a fully visible part hides every face (review 0003 M3
    /// and M6, decision 0015).
    pub fn edit_visibility(&mut self, edit: VisibilityEdit<'_>) -> Vec<NodeId> {
        let h = self.hide_id();
        let nodes: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let fresh = gs_par::map(nodes.len(), |k| {
            let node = self.node(nodes[k]);
            let old = match h.and_then(|h| node.attr(h)) {
                Some(AttrVec::Bits(b)) => b.clone(),
                _ => Bits::zeros(node.faces.len()),
            };
            let live = || -> Bits { node.faces.iter().map(|f| f[0] != NO).collect() };
            let mut new = old.clone();
            match edit {
                VisibilityEdit::Hide(set) => set.node(nodes[k]).into_iter().for_each(|p| new.or(p)),
                VisibilityEdit::Show(set) => {
                    set.node(nodes[k]).into_iter().for_each(|p| new.and_not(p));
                }
                VisibilityEdit::Only(set) => {
                    new = live();
                    set.node(nodes[k]).into_iter().for_each(|p| new.and_not(p));
                }
                VisibilityEdit::Invert => {
                    new = live();
                    new.and_not(&old);
                }
                VisibilityEdit::ShowAll => new = Bits::zeros(old.len()),
            }
            match (new == old, new.any()) {
                (true, _) => NewValues::Same,
                (false, true) => NewValues::Values(AttrVec::Bits(new)),
                (false, false) => NewValues::Zeros,
            }
        });
        self.put_column(AttrDef::hide(), &nodes, fresh)
    }
}

/// How [`MeshStore::edit_visibility`] changes the hidden faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VisibilityEdit<'a> {
    /// Hide these faces.
    Hide(&'a FaceSet),
    /// Show these faces.
    Show(&'a FaceSet),
    /// Show these faces and hide every other face.
    Only(&'a FaceSet),
    /// Swap hidden and visible.
    Invert,
    /// Show every face.
    ShowAll,
}

/// Hidden faces among a node's live faces. Dead face slots never hold a set bit.
fn face_cover(topo: &NodeTopo, col: Option<&AttrVec>) -> Cover {
    let live = topo.faces.len() - topo.ffree.len();
    match col {
        Some(AttrVec::Bits(b)) => Cover::of(b.count(), live),
        _ => Cover::None,
    }
}

/// A node's hidden vertex slots (`None` when none is) and the cover of hidden points among its
/// live points.
fn hidden_points(topo: &NodeTopo, pos: &[Vec3], visible: Visible<'_>) -> (Option<Bits>, Cover) {
    if visible.hide.is_none() {
        return (None, Cover::None);
    }
    let bits: Bits = (0..topo.ring.len())
        .map(|l| {
            let ring = topo.ring_of(l);
            !ring.is_empty() && ring.iter().all(|&f| !visible.face(f))
        })
        .collect();
    let live = pos.iter().filter(|p| !p.is_dead()).count();
    let cover = Cover::of(bits.count(), live);
    (bits.any().then_some(bits), cover)
}

/// The mask among a node's live points.
fn mask_cover(pos: &[Vec3], col: Option<&AttrVec>) -> Cover {
    let Some(AttrVec::F32(m)) = col else {
        return Cover::None;
    };
    let live = || {
        (m.iter().zip(pos))
            .filter(|(_, p)| !p.is_dead())
            .map(|(x, _)| *x)
    };
    if live().all(|x| x <= 0.0) {
        Cover::None
    } else if live().all(full) {
        Cover::All
    } else {
        Cover::Some
    }
}
