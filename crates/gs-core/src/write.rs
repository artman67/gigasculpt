//! The write guard the engine's commands use for positions and attributes. It hands out one
//! node at a time, in parallel, and only the columns declared up front. Every write goes through
//! `Arc::make_mut`, so an open history step keeps the old data. When the guard drops, normals,
//! bounds, halos, the tree and dirty bits are brought up to date.

use std::sync::Arc;

use crate::columns::{AttrDef, AttrId, AttrVec, ColumnSet, Columns, Domain};
use crate::config::NodeId;
use crate::math::Vec3;
use crate::par::select_mut;
use crate::store::MeshStore;

/// Write access to declared columns of a set of nodes. See [`MeshStore::write`].
pub struct Writer<'a> {
    store: &'a mut MeshStore,
    nodes: Vec<NodeId>,
    cols: ColumnSet,
}

/// One node's declared columns, writable.
pub struct NodeMut<'a> {
    id: NodeId,
    cols: ColumnSet,
    pos: &'a mut Arc<Vec<Vec3>>,
    /// Per registered attribute: the node's column if declared.
    attrs: Vec<Option<&'a mut Option<Arc<AttrVec>>>>,
    defs: &'a [AttrDef],
    verts: usize,
    faces: usize,
}

impl NodeMut<'_> {
    pub fn id(&self) -> NodeId {
        self.id
    }
    /// The node's positions, slot by slot (dead slots are NaN).
    ///
    /// # Panics
    /// If positions were not declared.
    pub fn positions(&mut self) -> &mut [Vec3] {
        assert!(
            self.cols.contains(ColumnSet::POSITION),
            "positions not declared"
        );
        Arc::make_mut(self.pos).as_mut_slice()
    }
    /// The node's values of attribute `a`, allocated as zeros on first use.
    ///
    /// # Panics
    /// If the attribute was not declared.
    pub fn attr(&mut self, a: AttrId) -> &mut AttrVec {
        let d = &self.defs[a.index()];
        let len = match d.domain {
            Domain::Vertex => self.verts,
            Domain::Face => self.faces,
        };
        let Some(col) = self.attrs[a.index()].as_deref_mut() else {
            panic!("attribute {} not declared", d.name);
        };
        Arc::make_mut(col.get_or_insert_with(|| Arc::new(AttrVec::zeros(d.kind, len))))
    }
    /// Drop the node's values of attribute `a`: it reads as its empty value (zero, white for a
    /// color) and holds no memory.
    ///
    /// # Panics
    /// If the attribute was not declared.
    pub fn clear_attr(&mut self, a: AttrId) {
        let Some(col) = self.attrs[a.index()].as_deref_mut() else {
            panic!("attribute {} not declared", self.defs[a.index()].name);
        };
        *col = None;
    }
}

impl MeshStore {
    /// A write guard over `nodes` (ascending, distinct) for the columns in `cols`: positions
    /// and attributes. Normals follow positions on their own, and topology changes go through
    /// the topology editor, so neither can be declared here.
    ///
    /// # Panics
    /// If `cols` declares normals or topology.
    pub fn write(&mut self, nodes: &[NodeId], cols: ColumnSet) -> Writer<'_> {
        assert!(
            !cols.contains(ColumnSet::NORMAL) && !cols.contains(ColumnSet::TOPOLOGY),
            "normals and topology are not writable through a write guard"
        );
        Writer {
            store: self,
            nodes: nodes.to_vec(),
            cols,
        }
    }
}

impl Writer<'_> {
    /// Call `f` on every node, in parallel, one task per node.
    pub fn each(&mut self, f: impl Fn(&mut NodeMut<'_>) + Sync) {
        let (nodes, cols) = (&self.nodes, self.cols);
        let s = &mut *self.store;
        let Columns {
            pos, topo, attrs, ..
        } = &mut s.cols;
        let mut per_attr: Vec<_> = (attrs.iter_mut().enumerate())
            .map(|(a, col)| {
                let declared = cols.contains(ColumnSet::attr(AttrId(a as u8)));
                let picked = if declared {
                    select_mut(col, nodes)
                } else {
                    Vec::new()
                };
                picked.into_iter()
            })
            .collect();
        let mut muts: Vec<NodeMut<'_>> = (nodes.iter().zip(select_mut(pos, nodes)))
            .map(|(&n, p)| NodeMut {
                id: n,
                cols,
                pos: p,
                attrs: per_attr.iter_mut().map(Iterator::next).collect(),
                defs: &s.defs,
                verts: topo[n as usize].ring.len(),
                faces: topo[n as usize].faces.len(),
            })
            .collect();
        gs_par::apply_each(&mut muts, |_, m| f(m));
    }
}

/// A node's new values of one attribute column, for [`MeshStore::put_attr`].
#[derive(Clone, Debug, PartialEq)]
pub enum NewValues {
    /// Leave the node's column as it is.
    Same,
    /// Replace it with these values.
    Values(AttrVec),
    /// Release it: it reads as its empty value (zero, white for a color) and holds no memory.
    Zeros,
}

impl MeshStore {
    /// Write each node's new values of attribute `col` (`fresh[i]` for `nodes[i]`) through one
    /// write guard over the nodes that change. Returns those nodes.
    pub fn put_attr(
        &mut self,
        col: AttrId,
        nodes: &[NodeId],
        fresh: Vec<NewValues>,
    ) -> Vec<NodeId> {
        let (ids, vals): (Vec<NodeId>, Vec<NewValues>) = (nodes.iter().zip(fresh))
            .filter(|(_, v)| *v != NewValues::Same)
            .map(|(&n, v)| (n, v))
            .unzip();
        self.write(&ids, ColumnSet::attr(col)).each(|m| {
            let Ok(i) = ids.binary_search(&m.id()) else {
                return;
            };
            match &vals[i] {
                NewValues::Values(v) => *m.attr(col) = v.clone(),
                NewValues::Zeros => m.clear_attr(col),
                NewValues::Same => {}
            }
        });
        ids
    }

    /// As [`MeshStore::put_attr`], into the column `def` names, registering it only when some
    /// node gets values: a write that leaves every node as it was adds no column, so an edit
    /// that changes nothing leaves the store exactly as it was, its column list included
    /// (review 0003 M3). `fresh` is worked out against the column as it is, a missing column
    /// reading as zeros. Writes nothing if `def` cannot be written ([`MeshStore::attr_fits`]).
    pub fn put_column(
        &mut self,
        def: AttrDef,
        nodes: &[NodeId],
        fresh: Vec<NewValues>,
    ) -> Vec<NodeId> {
        let registered = matches!(self.attr_fits(&def), Ok(Some(_)));
        let values = fresh.iter().any(|v| matches!(v, NewValues::Values(_)));
        if !(registered || values) {
            return Vec::new();
        }
        let Ok(col) = self.add_attr(def) else {
            return Vec::new();
        };
        self.put_attr(col, nodes, fresh)
    }
}

impl Drop for Writer<'_> {
    /// Bring everything derived up to date. Skipped while panicking: the engine then cancels
    /// the open step, which restores the columns and refreshes what they changed.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let nodes = std::mem::take(&mut self.nodes);
        if self.cols.contains(ColumnSet::POSITION) {
            self.store.after_moves(&nodes);
        }
        self.store.refresh_state(&nodes, self.cols);
        self.store.mark(&nodes, self.cols);
    }
}
