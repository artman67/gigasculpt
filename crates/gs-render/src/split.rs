//! A store's quads as they are drawn, split along their shorter diagonals (decision 0040,
//! section 2), kept current as their corners move.
//!
//! A node's chunk holds its faces in an order that depends on its topology alone, worked out
//! when the chunk is cut: meshoptimizer's cache order of its faces split from corner 0 to 2,
//! each quad's two triangles together where the later of them was. Split either way, a quad's
//! diagonal is the edge opposite corner 1 of its first triangle and corner 2 of its second, so
//! a flip changes the quad's six indices and nothing else: no face moves, and hidden edges and
//! group colors stay. Each update works out the split of every quad stored in a node whose
//! points moved, and of each quad with a corner in another node in a node whose faces use a
//! moved node's points, and writes each node with a flip again from its first flipped quad to
//! its last.

use std::ops::Range;

use gs_core::{ColumnSet, MeshStore, NO, NodeId, NodeView, PosRead, PosReader};
use gs_protocol::arity;

use crate::cull::Cone;
use crate::device::Gpu;
use crate::mesh::{
    Bounds, ChunkPart, Triangles, diagonal, group_words, optimize_chunk, triangulate,
};
use crate::pool::GpuMesh;

/// A node's live faces in drawing order and how its quads are split, kept so an update can
/// find the quads whose split flipped. Empty for a node without quads, which never flips.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeSplit {
    /// The node's live face slots, in drawing order.
    order: Vec<u32>,
    /// Bit `k`: face `order[k]` is a quad split from corner 1 to 3.
    ones: Vec<u64>,
}

impl NodeSplit {
    /// Node `node`'s live faces in drawing order, its quads split as they are now. The order is
    /// meshoptimizer's cache order of the faces split from corner 0 to 2, each quad where the
    /// later of its two triangles was.
    pub fn cut(node: &NodeView<'_>, pos: &PosReader<'_>) -> NodeSplit {
        let faces = node.faces;
        let (mut tris, mut of, mut one) = (Vec::new(), Vec::new(), vec![false; faces.len()]);
        for (j, face) in (0..).zip(faces).filter(|f| f.1[0] != NO) {
            let [a, b, c, d] = *face;
            tris.push([a, b, c]);
            of.push(j);
            if arity(*face) == 4 {
                tris.push([a, c, d]);
                of.push(j);
                one[j] = diagonal(face, |v| at(node, pos, v)) == 1;
            }
        }
        let mut seen = vec![false; faces.len()];
        let mut split = NodeSplit::default();
        for k in optimize_chunk(&tris) {
            let j = of[k];
            if arity(faces[j]) == 4 && !std::mem::replace(&mut seen[j], true) {
                continue;
            }
            let k = split.order.len();
            if k % 64 == 0 {
                split.ones.push(0);
            }
            split.ones[k / 64] |= u64::from(one[j]) << (k % 64);
            split.order.push(j as u32);
        }
        split
    }

    /// The live faces it holds.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Faces `range` of the drawing order as triangles, split as it holds them, each with
    /// whether `shown(slot)` draws it and, given the node's group ids, its group color.
    pub fn triangles(
        &self,
        node: &NodeView<'_>,
        shown: impl Fn(u32) -> bool,
        groups: Option<&[u32]>,
        range: Range<usize>,
    ) -> Triangles {
        let face = |k: usize| &node.faces[self.order[k] as usize];
        let mut t = triangulate(
            range
                .clone()
                .map(|k| (face(k), shown(self.order[k]), self.one(k))),
        );
        if let Some(ids) = groups {
            let id = |k: usize| ids.get(self.order[k] as usize).copied().unwrap_or(0);
            t.colors = group_words(range.map(|k| (face(k), id(k))));
        }
        t
    }

    /// Whether the face at place `k` of the drawing order is a quad split from corner 1 to 3.
    fn one(&self, k: usize) -> usize {
        (self.ones[k / 64] >> (k % 64)) as usize & 1
    }

    /// Itself, or nothing for a node without quads, which never flips.
    pub fn kept(self, node: &NodeView<'_>) -> NodeSplit {
        let quads = self
            .order
            .iter()
            .any(|&j| arity(node.faces[j as usize]) == 4);
        if quads { self } else { NodeSplit::default() }
    }

    /// Its quads whose split is now the other one: their places in the drawing order, and the
    /// chunk triangle the first of them starts at. `None` without a flip. Unless `own` (its own
    /// points moved), only its quads with a corner in another node can have moved, and only
    /// they are checked.
    fn flips(
        &self,
        node: &NodeView<'_>,
        pos: &PosReader<'_>,
        own: bool,
    ) -> Option<(Vec<u32>, u32)> {
        let len = node.pos.len();
        let moved = |face: &[u32; 4]| {
            own || face
                .iter()
                .any(|&v| v.wrapping_sub(node.base) as usize >= len)
        };
        let (mut flips, mut first, mut triangle) = (Vec::new(), 0, 0);
        for (k, &j) in self.order.iter().enumerate() {
            let face = &node.faces[j as usize];
            if arity(*face) == 3 {
                triangle += 1;
                continue;
            }
            if moved(face) && diagonal(face, |v| at(node, pos, v)) != self.one(k) {
                if flips.is_empty() {
                    first = triangle;
                }
                flips.push(k as u32);
            }
            triangle += 2;
        }
        (!flips.is_empty()).then_some((flips, first))
    }
}

/// Where vertex `v`, a corner of one of `node`'s faces, is now.
fn at(node: &NodeView<'_>, pos: &PosReader<'_>, v: u32) -> [f32; 3] {
    let local = v.wrapping_sub(node.base) as usize;
    node.pos
        .get(local)
        .map_or_else(|| pos.get(v), |p| *p)
        .to_array()
}

/// Writes again the quads of `store` whose drawn split flipped since `mesh` last held them, in
/// the nodes whose points moved (as `dirty` says) or whose faces use moved points, but for the
/// nodes `written` (ascending) afresh this update. `slot` numbers a vertex as the pools do and
/// `first(n)` is node `n`'s first triangle. Returns the nodes written and the bytes.
pub(crate) fn write_flips(
    gpu: &Gpu,
    mesh: &mut GpuMesh,
    store: &MeshStore,
    dirty: &[(NodeId, ColumnSet)],
    written: &[NodeId],
    slot: impl Fn(u32) -> u32,
    first: impl Fn(NodeId) -> u32,
) -> (usize, u64) {
    let count = store.node_count();
    let mut moved = vec![false; count];
    for d in dirty.iter().filter(|d| d.1.contains(ColumnSet::POSITION)) {
        if let Some(m) = moved.get_mut(d.0 as usize) {
            *m = true;
        }
    }
    if !moved.contains(&true) {
        return (0, 0);
    }
    let (cfg, pos) = (store.config(), store.pos());
    let splits = &mesh.splits;
    let found = gs_par::map(count, |i| {
        let n = i as NodeId;
        if splits[i].order.is_empty() || written.binary_search(&n).is_ok() {
            return None;
        }
        let node = store.node(n);
        let uses = |v: &u32| moved.get(cfg.node_of(*v) as usize) == Some(&true);
        if !moved[i] && !node.shared.iter().any(uses) {
            return None;
        }
        splits[i].flips(&node, &pos, moved[i])
    });
    let visible = store.visible();
    let (mut nodes, mut bytes) = (0, 0);
    for (i, (flips, triangle)) in found
        .into_iter()
        .enumerate()
        .filter_map(|(i, f)| Some((i, f?)))
    {
        let (n, split) = (i as NodeId, &mut mesh.splits[i]);
        for &k in &flips {
            split.ones[k as usize / 64] ^= 1 << (k % 64);
        }
        let node = store.node(n);
        let shown = |j: u32| visible.face(node.first_face + j);
        let range = flips[0] as usize..flips[flips.len() - 1] as usize + 1;
        let t = split.triangles(&node, shown, None, range);
        let part = ChunkPart::whole(t, Bounds::EMPTY, Cone::NONE);
        let indices: Vec<u32> = part.indices(&slot).collect();
        let offset = u64::from(first(n) + triangle) * 12;
        gpu.queue
            .write_buffer(&mesh.indices, offset, bytemuck::cast_slice(&indices));
        (nodes, bytes) = (nodes + 1, bytes + indices.len() as u64 * 4);
    }
    (nodes, bytes)
}
