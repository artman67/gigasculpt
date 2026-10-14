//! How a part's vertices and faces are numbered outside its mesh store.
//!
//! A `MeshStore` sorts the vertices along a Morton curve when it is built, so its own order
//! (live vertex slots node by node, the order `MeshStore::to_plain` exports) is not the order a
//! caller gave. Parts made from plain mesh data (`part.create`, `file.import`) keep the
//! caller's numbering, faces and corner order included: `part.get` gives back exactly what went
//! in, and tests can compare a part with a transformed copy vertex by vertex. Parts the engine
//! generates (primitives) are numbered in the store's own order, which costs nothing to keep.
//! A part with detail levels is numbered by its active level's numbering (decision 0014).
//!
//! The numbering itself is gs-core's (`gs_core::Numbering`), one home for imported parts and
//! for detail levels. It holds while the store's topology is unchanged. A stroke that changes
//! it (Live Detail) switches the part to the store's own order, and undo switches it back
//! (decision 0021, item 11.4).

use std::sync::Arc;

use gs_core::{MeshError, MeshStore, NodeConfig, NodeId, Placement};
use gs_protocol::PlainMesh;

use crate::mesh::corners;

/// Which numbering a new part gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexOrder {
    /// The order of the plain mesh it was made from.
    AsGiven,
    /// The mesh store's spatial order. For meshes the engine generates.
    Spatial,
}

/// A part's numbering.
#[derive(Clone)]
pub(crate) enum Order {
    /// The store's own order.
    Spatial,
    /// A numbering of its own: the order the part was made with, or its active detail level's.
    Given(Arc<gs_core::Numbering>),
}

impl Order {
    /// The numbering as a scene file stores it (see `gs_core::Numbering::placement`), `None`
    /// for the store's own order.
    pub fn given(&self) -> Option<Placement> {
        match self {
            Order::Spatial => None,
            Order::Given(g) => Some(g.placement()),
        }
    }

    /// A given numbering read from a scene file, checked against its store: permutations of
    /// the store's vertices and faces, a first corner each face has, and no dead slot (a
    /// numbering counts slots, so it assumes none, see `gs_core::Numbering`).
    pub fn from_given(
        store: &MeshStore,
        verts: Vec<u32>,
        faces: Vec<u32>,
        first: Vec<u8>,
    ) -> Result<Order, String> {
        let nodes = || (0..store.node_count() as NodeId).map(|n| store.node(n));
        let slots: usize = nodes().map(|n| n.pos.len()).sum();
        let face_slots: usize = nodes().map(|n| n.faces.len()).sum();
        if slots != store.vertex_count() || face_slots != store.face_count() {
            return Err("a given numbering on a part with dead slots".into());
        }
        let corners: Vec<u8> = nodes()
            .flat_map(|n| n.faces.iter())
            .map(|f| corners(f).len() as u8)
            .collect();
        let permutation = |v: &[u32], n: usize| {
            let mut seen = vec![false; n];
            v.len() == n
                && v.iter()
                    .all(|&i| (i as usize) < n && !std::mem::replace(&mut seen[i as usize], true))
        };
        if !permutation(&verts, slots) || !permutation(&faces, corners.len()) {
            return Err("the numbering is not a renumbering of the part".into());
        }
        let fits = (faces.iter().zip(&first)).all(|(&f, &c)| c < corners[f as usize]);
        if first.len() != faces.len() || !fits {
            return Err("a face's first corner is not one of its corners".into());
        }
        let placed = Placement {
            vertices: verts,
            faces,
            first_corner: first,
        };
        let given = gs_core::Numbering::from_placement(store, placed);
        Ok(Order::Given(Arc::new(given)))
    }

    /// Bytes a given numbering holds; the store's own order holds none.
    pub fn bytes(&self) -> usize {
        self.allocation().map_or(0, |a| a.bytes)
    }

    /// A given numbering's allocation, for the history's memory count.
    pub fn allocation(&self) -> Option<gs_core::Allocation> {
        match self {
            Order::Spatial => None,
            Order::Given(g) => Some(gs_core::Allocation {
                at: Arc::as_ptr(g) as usize,
                bytes: g.bytes(),
            }),
        }
    }

    /// True if both number the vertices and faces the same way.
    pub fn same(&self, other: &Order) -> bool {
        match (self, other) {
            (Order::Spatial, Order::Spatial) => true,
            (Order::Given(a), Order::Given(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
    }

    /// The numbering as gs-core's, the store's own order made explicit.
    pub fn numbering(&self, store: &MeshStore) -> Arc<gs_core::Numbering> {
        match self {
            Order::Spatial => Arc::new(gs_core::Numbering::store_order(store)),
            Order::Given(given) => Arc::clone(given),
        }
    }
}

/// Builds a store from `mesh`, with the numbering `order` asks for. gs-core says where every
/// vertex and face went, so nothing is exported to find out. In the store's own order the UV
/// corners turn with the faces the build turned, so each stays on its corner.
pub(crate) fn import(mesh: PlainMesh, order: VertexOrder) -> Result<(MeshStore, Order), MeshError> {
    let (mut store, placed) = MeshStore::import(mesh, NodeConfig::default())?;
    if order == VertexOrder::Spatial {
        store.store_corners(&crate::level::uv_corners(), &placed);
        return Ok((store, Order::Spatial));
    }
    let given = gs_core::Numbering::from_placement(&store, placed);
    Ok((store, Order::Given(Arc::new(given))))
}

/// Sorts a part's store again and numbers it in the store's own order (`mesh.reorder`). The UV
/// corners turn with the faces: from a given numbering's corners to the stored ones, then with
/// the turn the re-sort gave each face, so each stays on its corner.
pub(crate) fn resort_spatial(store: &mut MeshStore, order: &Order) -> Result<(), MeshError> {
    let corners = crate::level::uv_corners();
    if let Order::Given(given) = order {
        store.store_corners(&corners, &given.placement());
    }
    let placed = store.resort()?;
    store.store_corners(&corners, &placed);
    Ok(())
}

/// The part as plain mesh data, in its numbering.
pub(crate) fn export(store: &MeshStore, order: &Order) -> PlainMesh {
    let sorted = store.to_plain();
    match order {
        Order::Spatial => sorted,
        Order::Given(given) => given.apply(sorted),
    }
}

/// Each vertex slot's index in a part's numbering.
pub enum Numbering {
    /// The store's own order: the dense index of each node's first slot.
    Store(Vec<u32>),
    Given(Arc<gs_core::Numbering>),
}

impl Numbering {
    pub(crate) fn of(store: &MeshStore, order: &Order) -> Self {
        match order {
            Order::Spatial => {
                let mut next = 0;
                let starts = (0..store.node_count() as NodeId)
                    .map(|n| {
                        let start = next;
                        next += store.node(n).pos.len() as u32;
                        start
                    })
                    .collect();
                Self::Store(starts)
            }
            Order::Given(given) => Self::Given(Arc::clone(given)),
        }
    }

    /// The index of slot `local` of node `node`.
    pub fn index(&self, node: NodeId, local: u32) -> u32 {
        match self {
            Self::Store(starts) => starts[node as usize] + local,
            Self::Given(given) => given.index(node, local),
        }
    }
}
