//! The one door through which nodes leave the core for a mirror of the mesh, such as the
//! renderer's GPU copy (decision 0012). It names no GPU type: a sink gets the same read views
//! kernels use.
//!
//! Whoever keeps the mirror reads the dirty bits ([`MeshStore::take_dirty`]) and feeds the
//! nodes they name, whatever changed them: strokes, undo, redo, job commits, write guards; or
//! every node, for a full copy.

use std::fmt::Debug;

use crate::config::NodeId;
use crate::store::MeshStore;
use crate::view::{NodeView, PosReader};

/// Receives nodes, each with its current positions and normals.
pub trait NodeSink: Send + Sync + Debug {
    /// `node` as it is now; `pos` reads other nodes' positions (the seam corners of its faces).
    /// Called from `gs-par` tasks, so several nodes may arrive at once on different threads,
    /// each at most once per call.
    fn node(&self, node: &NodeView<'_>, pos: &PosReader<'_>);
}

impl MeshStore {
    /// Hands `nodes` to `sink` as they are now, one task per node.
    pub fn feed(&self, nodes: &[NodeId], sink: &dyn NodeSink) {
        let pos = self.pos();
        let mut ids = nodes.to_vec();
        gs_par::apply_each(&mut ids, |_, n| sink.node(&self.node(*n), &pos));
    }
}
