//! The renderer's side of gs-core's node sink (decision 0012): the nodes an update or a full
//! upload feeds are packed for the GPU as the core hands them over, in parallel.
//!
//! An update knows its nodes and their vertex ranges before it feeds them, so it opens one
//! write view per node onto the vertex pool first and each node is packed straight into its
//! view: the packed bytes are written once, into the memory the GPU copies from. A full upload
//! has no pool yet, so it packs each node into a buffer of its own.
//!
//! [`pack`] is the one way a node's data becomes GPU data, for updates and full uploads alike,
//! so a mesh brought up to date and a fresh upload hold the same bytes.
//!
//! Normal cones are measured only for meshes drawn front faces only, the one way of drawing
//! that culls by facing, and for full uploads: they are the only reason packing reads a node's
//! faces, and the window draws both sides.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use bytemuck::Zeroable;
use gs_core::{NO, NodeSink, NodeView, PosRead, PosReader};
use gs_protocol::{arity, quad_diagonal, quad_triangles};

use crate::cull::{Cone, ConeBuilder};
use crate::math::{V3, add};
use crate::mesh::{Bounds, GpuVertex};

/// Vertex slots packed at a time before they are copied out: 256, 4 KiB, which stay in the
/// first-level cache.
const BLOCK: usize = 256;

/// One node as the GPU draws it.
pub(crate) struct Staged {
    /// Its vertex slots, in order (dead slots keep their NaN position; no face uses them); none
    /// when they were packed straight into a write view.
    pub vertices: Vec<GpuVertex>,
    /// The box around its own live vertices and every corner its live faces use, seam corners
    /// included. (A live vertex no face of the node uses only makes it safely larger.)
    pub bounds: Bounds,
    /// Its faces' normal cone; `None` when not measured, because the mesh was not drawn front
    /// faces only.
    pub cone: Option<Cone>,
}

/// A GPU mesh's packed nodes not taken yet.
#[derive(Default)]
pub(crate) struct Staging {
    staged: Mutex<BTreeMap<u32, Staged>>,
    /// The write view each node of the next feed is packed into; a node without one is packed
    /// into a buffer of its own. Set before a feed, never during one.
    views: BTreeMap<u32, Mutex<wgpu::QueueWriteBufferView>>,
    /// Measure normal cones. Set before a feed, never during one.
    cones: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl fmt::Debug for Staging {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Staging")
            .field("staged", &lock(&self.staged).len())
            .field("views", &self.views.len())
            .finish_non_exhaustive()
    }
}

impl NodeSink for Staging {
    fn node(&self, node: &NodeView<'_>, pos: &PosReader<'_>) {
        let cones = self.cones.load(Ordering::Relaxed);
        let mut vertices = Vec::new();
        let (bounds, cone) = if let Some(view) = self.views.get(&node.id) {
            let mut view = lock(view);
            pack(node, pos, cones, |at, block| {
                let bytes: &[u8] = bytemuck::cast_slice(block);
                let at = at * size_of::<GpuVertex>();
                view.slice(at..at + bytes.len()).copy_from_slice(bytes);
            })
        } else {
            vertices.reserve_exact(node.pos.len());
            pack(node, pos, cones, |_, block| {
                vertices.extend_from_slice(block);
            })
        };
        let staged = Staged {
            vertices,
            bounds,
            cone,
        };
        lock(&self.staged).insert(node.id, staged);
    }
}

impl Staging {
    /// Whether nodes packed from now on get their normal cone measured.
    pub fn measure_cones(&self, on: bool) {
        self.cones.store(on, Ordering::Relaxed);
    }

    /// The write views the next feed's nodes are packed into, by node.
    pub fn pack_into(&mut self, views: BTreeMap<u32, wgpu::QueueWriteBufferView>) {
        self.views = views.into_iter().map(|(n, v)| (n, Mutex::new(v))).collect();
    }

    /// Every staged node, ascending, leaving none; the write views are dropped, which sends
    /// them.
    pub fn take(&mut self) -> BTreeMap<u32, Staged> {
        self.views.clear();
        std::mem::take(&mut *lock(&self.staged))
    }
}

/// Packs node `node`: its vertex slots, positions and 16-bit octahedral normals, handed to
/// `put` [`BLOCK`] slots at a time with the first slot's index; its box, gs-core's box around
/// its own live vertices grown by the other nodes' vertices its faces use (read through
/// `pos`), so it holds every corner of its faces; and, with `cones`, their normal cone around
/// the sum of the node's vertex normals, splitting quads as the index pool does. Without cones
/// the faces are not read at all.
pub(crate) fn pack(
    node: &NodeView<'_>,
    pos: &PosReader<'_>,
    cones: bool,
    mut put: impl FnMut(usize, &[GpuVertex]),
) -> (Bounds, Option<Cone>) {
    let mut bounds = Bounds {
        min: node.bounds.lo.to_array(),
        max: node.bounds.hi.to_array(),
    };
    for &v in node.shared {
        bounds.include(pos.get(v).to_array());
    }
    let cone = cones.then(|| cone(node, pos));
    let mut block = [GpuVertex::zeroed(); BLOCK];
    let slots = node.pos.chunks(BLOCK).zip(node.nrm.chunks(BLOCK));
    for (k, (p, m)) in slots.enumerate() {
        let block = &mut block[..p.len().min(m.len())];
        for (v, (p, m)) in block.iter_mut().zip(p.iter().zip(m)) {
            *v = GpuVertex::new(p.to_array(), m.to_array());
        }
        put(k * BLOCK, block);
    }
    (bounds, cone)
}

/// The normal cone of node `node`'s live faces around the sum of its vertex normals.
fn cone(node: &NodeView<'_>, pos: &PosReader<'_>) -> Cone {
    let axis = node.nrm.iter().fold([0.0; 3], |a, m| add(a, m.to_array()));
    let mut cone = ConeBuilder::new(axis);
    let at = |v: u32| -> V3 {
        let local = v.wrapping_sub(node.base) as usize;
        node.pos
            .get(local)
            .map_or_else(|| pos.get(v), |p| *p)
            .to_array()
    };
    for f in node.faces.iter().filter(|f| f[0] != NO) {
        let [a, b, c] = [f[0], f[1], f[2]].map(at);
        if arity(*f) == 3 {
            cone.add(a, b, c);
            continue;
        }
        // The triangles as they are drawn: a quad split along its shorter diagonal.
        let p = [a, b, c, at(f[3])];
        for t in quad_triangles(quad_diagonal(p)) {
            cone.add(p[t[0]], p[t[1]], p[t[2]]);
        }
    }
    cone.finish()
}
