//! Drawing straight from a gs-core [`MeshStore`] (ARCHITECTURE 3.1 and 6): each node has a
//! range of the vertex pool for its vertex slots and a range of the index pool for its faces,
//! which are one chunk (decision 0021, item 7).
//!
//! Node data reaches the GPU through one door (decision 0012): [`Renderer::update_store`]
//! feeds the nodes the store reports dirty, whatever changed them (strokes, undo, redo, job
//! commits), through gs-core's node sink into the mesh's staging, where one function packs
//! them in parallel, each straight into a write view onto its vertex range;
//! [`Renderer::upload_store`] feeds every node the same way, into buffers of their own.
//!
//! A topology change (Live Detail, a node cut, compaction, or undo of any of them) rewrites
//! only the nodes whose topology changed. A node that still fits its ranges is written in
//! place; one that outgrew a range gets a new one at the end of the pool, a quarter larger than
//! it needs, and the nodes whose faces use its vertices have their indices written again. A
//! full upload packs the ranges tight, so only nodes that grew hold room and a part never
//! refined takes no more memory than before. The pools are packed afresh, by a full upload,
//! only when the ranges nodes left behind pass half of a pool.
//!
//! Hidden faces (decision 0015) keep their place in their node's chunk as triangles with one
//! corner three times, which draw nothing, and a node with every face hidden is skipped by the
//! culling pass. Hiding or showing faces rewrites only the index ranges of the nodes it touched.
//!
//! The streams beside the pools (decision 0039, section 9) follow the same ranges: the mask and
//! the point colors by vertex slot, written for the nodes whose column or topology changed, so
//! a paint-only edit sends only color; the face group colors by triangle, written with a node's
//! faces, so a group edit rewrites the nodes it touched, as hiding does.

use std::collections::BTreeSet;

use gs_core::{
    AttrId, AttrKind, AttrVec, ColumnSet, Domain, MeshStore, NodeConfig, NodeId, VertId,
};
use gs_protocol::{GROUP, HIDE, MASK};

use crate::cull::{ChunkCull, Cone};
use crate::mesh::{Bounds, Chunk, ChunkPart, GpuVertex, assemble, mask_byte};
use crate::pool::{GpuMesh, PoolBytes, Sides, upload_mesh};
use crate::snapshot::Renderer;
use crate::split::{NodeSplit, write_flips};
use crate::staging::Staging;
use crate::streams::{COLOR, WHITE, send_words};

/// A node's range of a pool: where it starts, the room it has, and how much of it it uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: u32,
    room: u32,
    len: u32,
}

/// The room a node of `len` vertex slots gets when it moves: a quarter more.
fn vertex_room(len: u32) -> u32 {
    len + len / 4
}

/// The room a chunk of `len` triangles gets when it moves: a quarter more, in whole words of
/// hidden edges (8 triangles a word), so no two chunks share a word.
fn triangle_room(len: u32) -> u32 {
    vertex_room(len).next_multiple_of(8)
}

/// The room a node of `len` vertex slots gets in a full upload: none.
fn vertex_tight(len: u32) -> u32 {
    len
}

/// The room a chunk of `len` triangles gets in a full upload: whole words of hidden edges.
fn triangle_tight(len: u32) -> u32 {
    len.next_multiple_of(8)
}

/// The nodes' ranges of one pool.
#[derive(Clone, Debug, Default)]
struct Ranges {
    spans: Vec<Span>,
    /// Where the last range ends.
    end: u32,
    /// Room in ranges no node uses any more.
    dead: u32,
}

impl Ranges {
    /// Ranges one after another for nodes of these lengths.
    fn packed(lens: impl IntoIterator<Item = u32>, room: fn(u32) -> u32) -> Ranges {
        let mut ranges = Ranges::default();
        for (n, len) in lens.into_iter().enumerate() {
            ranges.place(n, len, room);
        }
        ranges
    }

    /// Node `n` (at most one past the last) now uses `len`: in its range if that has room,
    /// else in a new range at the end, leaving the old one dead. Whether it got a new range.
    fn place(&mut self, n: usize, len: u32, room: fn(u32) -> u32) -> bool {
        if let Some(span) = self.spans.get_mut(n)
            && len <= span.room
        {
            span.len = len;
            return false;
        }
        let new = Span {
            start: self.end,
            room: room(len),
            len,
        };
        self.end += new.room;
        if let Some(old) = self.spans.get_mut(n) {
            self.dead += old.room;
            *old = new;
        } else {
            debug_assert_eq!(n, self.spans.len(), "nodes are added in order");
            self.spans.push(new);
        }
        true
    }

    /// Forgets the nodes from `count` on, leaving their ranges dead.
    fn truncate(&mut self, count: usize) {
        for span in self.spans.drain(count.min(self.spans.len())..) {
            self.dead += span.room;
        }
    }

    /// Whether ranges no node uses fill more than half of the pool.
    fn sparse(&self) -> bool {
        u64::from(self.dead) * 2 > u64::from(self.end)
    }
}

/// Where each node's vertex slots and triangles sit in the pools.
#[derive(Clone, Debug)]
pub(crate) struct NodeLayout {
    cfg: NodeConfig,
    vertices: Ranges,
    triangles: Ranges,
    /// Each node's face slots, which with its vertex slots tell a store this layout was not
    /// made for.
    faces: Vec<u32>,
}

impl NodeLayout {
    /// Ranges one after another, without room, for `store`'s nodes and their chunks `parts`.
    fn packed(store: &MeshStore, parts: &[ChunkPart]) -> NodeLayout {
        let nodes = (0..store.node_count() as NodeId).map(|n| store.node(n));
        NodeLayout {
            cfg: store.config(),
            vertices: Ranges::packed(nodes.clone().map(|n| n.pos.len() as u32), vertex_tight),
            triangles: Ranges::packed(parts.iter().map(ChunkPart::triangles), triangle_tight),
            faces: nodes.map(|n| n.faces.len() as u32).collect(),
        }
    }

    /// The pool slot of vertex `v`.
    fn slot(&self, v: VertId) -> u32 {
        self.vertices.spans[self.cfg.node_of(v) as usize].start + self.cfg.local_of(v)
    }

    /// Node `n`'s first vertex slot.
    fn first_slot(&self, n: NodeId) -> u32 {
        self.vertices.spans[n as usize].start
    }

    /// Each node's first vertex slot and slots, and where the last range ends.
    pub(crate) fn vertex_ranges(&self) -> (Vec<(u32, u32)>, u32) {
        let spans = self.vertices.spans.iter().map(|s| (s.start, s.len));
        (spans.collect(), self.vertices.end)
    }

    /// Node `n`'s first triangle.
    fn first_triangle(&self, n: NodeId) -> u32 {
        self.triangles.spans[n as usize].start
    }

    /// Whether `store` is the store this layout was made for, but for the nodes in `changed`:
    /// every other node it has here has as many vertex and face slots.
    fn matches(&self, store: &MeshStore, changed: &BTreeSet<NodeId>) -> bool {
        let both = self.faces.len().min(store.node_count()) as NodeId;
        self.cfg == store.config()
            && (0..both).filter(|n| !changed.contains(n)).all(|n| {
                let node = store.node(n);
                let span = self.vertices.spans[n as usize];
                (node.pos.len() as u32, node.faces.len() as u32)
                    == (span.len, self.faces[n as usize])
            })
    }

    /// `raw`, every byte of a mesh's pools laid out by this layout with these `chunks`, as a
    /// mesh packed tight in node order holds it (see [`PoolBytes`]). A corner in no node's
    /// vertex slots reads as `u32::MAX`, so it never matches.
    pub(crate) fn tight(&self, raw: &PoolBytes, chunks: &[Chunk]) -> PoolBytes {
        let spans = &self.vertices.spans;
        let range =
            |s: &Span, unit: usize| s.start as usize * unit..(s.start + s.len) as usize * unit;
        let vertices = (spans.iter())
            .flat_map(|s| &raw.vertices[range(s, 16)])
            .copied()
            .collect();
        let starts: Vec<u32> = (spans.iter())
            .scan(0, |at, s| {
                *at += s.len;
                Some(*at - s.len)
            })
            .collect();
        let mut by_start: Vec<usize> = (0..spans.len()).filter(|&n| spans[n].len > 0).collect();
        by_start.sort_by_key(|&n| spans[n].start);
        let tight = |i: u32| {
            let k = by_start.partition_point(|&n| spans[n].start <= i);
            let n = k.checked_sub(1).map(|k| by_start[k]);
            let n = n.filter(|&n| i < spans[n].start + spans[n].len);
            n.map_or(u32::MAX, |n| starts[n] + i - spans[n].start)
        };
        let (mut indices, mut firsts) = (Vec::new(), Vec::with_capacity(chunks.len()));
        for c in chunks {
            firsts.push((indices.len() / 3) as u32);
            let bytes =
                &raw.indices[c.first_triangle as usize * 12..][..c.triangle_count as usize * 12];
            let words: Vec<u32> = bytemuck::pod_collect_to_vec(bytes);
            indices.extend(words.into_iter().map(tight));
        }
        let records: Vec<ChunkCull> = bytemuck::pod_collect_to_vec(&raw.culling);
        let records: Vec<ChunkCull> = (records.into_iter().zip(&firsts))
            .map(|(r, &first)| r.at(first))
            .collect();
        // A stream's bytes for each node's slots, in node order; empty without the stream.
        let by_slot = |stream: &[u8], unit: usize| -> Vec<u8> {
            if stream.is_empty() {
                return Vec::new();
            }
            (spans.iter())
                .flat_map(|s| &stream[range(s, unit)])
                .copied()
                .collect()
        };
        let mut mask = by_slot(&raw.mask, 1);
        mask.resize(mask.len().next_multiple_of(4), 0);
        let groups = if raw.groups.is_empty() {
            Vec::new()
        } else {
            (chunks.iter())
                .flat_map(|c| {
                    &raw.groups[c.first_triangle as usize * 4..][..c.triangle_count as usize * 4]
                })
                .copied()
                .collect()
        };
        PoolBytes {
            vertices,
            indices: bytemuck::cast_slice(&indices).to_vec(),
            culling: bytemuck::cast_slice(&records).to_vec(),
            mask,
            colors: by_slot(&raw.colors, 4),
            groups,
        }
    }
}

/// Where an update puts the nodes it changed.
struct Placed {
    layout: NodeLayout,
    /// Nodes whose topology changed (every node the store has that the layout had not).
    changed: BTreeSet<NodeId>,
    /// Nodes whose index range is written, ascending, and their chunks and splits.
    rewrite: Vec<NodeId>,
    parts: Vec<ChunkPart>,
    splits: Vec<NodeSplit>,
    /// Nodes given a new range in either pool.
    moved: usize,
}

/// Where every node goes once `dirty` is sent to `mesh`; `None` when it needs a full upload
/// instead: it was not uploaded from this store, or the ranges nodes left behind would fill
/// more than half of a pool.
fn place(mesh: &GpuMesh, store: &MeshStore, dirty: &[(NodeId, ColumnSet)]) -> Option<Placed> {
    let mut layout = mesh.layout.clone().filter(|_| mesh.staging.is_some())?;
    // Groups that came or went change every chunk's stream.
    if group_column(store).is_some() != mesh.streams.grouped {
        return None;
    }
    let count = store.node_count();
    let changed: BTreeSet<NodeId> = (dirty.iter())
        .filter(|d| d.1.contains(ColumnSet::TOPOLOGY))
        .map(|d| d.0)
        .chain(layout.faces.len() as NodeId..count as NodeId)
        .collect();
    if !layout.matches(store, &changed) {
        return None;
    }
    layout.vertices.truncate(count);
    layout.triangles.truncate(count);
    layout.faces.resize(count, 0);
    let mut moved = BTreeSet::new();
    for &n in &changed {
        let node = store.node(n);
        layout.faces[n as usize] = node.faces.len() as u32;
        if (layout.vertices).place(n as usize, node.pos.len() as u32, vertex_room) {
            moved.insert(n);
        }
    }
    // Faces hidden or shown, faces whose group changed while groups are drawn, and faces naming
    // a moved node's vertices, are written again too.
    let hide = store.attr_id(HIDE).map(ColumnSet::attr);
    let drawn = |_: &AttrId| mesh.streams.groups.is_some();
    let group = group_column(store).filter(drawn).map(ColumnSet::attr);
    let mut rewrite = changed.clone();
    rewrite.extend(
        (dirty.iter())
            .filter(|d| [hide, group].into_iter().flatten().any(|c| d.1.contains(c)))
            .map(|d| d.0),
    );
    if !moved.is_empty() {
        let cfg = layout.cfg;
        let uses =
            |m: NodeId| (store.node(m).shared.iter()).any(|&v| moved.contains(&cfg.node_of(v)));
        rewrite.extend((0..count as NodeId).filter(|&m| uses(m)));
    }
    let rewrite: Vec<NodeId> = rewrite.into_iter().collect();
    let (parts, splits): (Vec<_>, Vec<_>) = gs_par::map(rewrite.len(), |i| {
        node_part(store, rewrite[i], Bounds::EMPTY, Cone::NONE)
    })
    .into_iter()
    .unzip();
    for (&n, part) in rewrite.iter().zip(&parts) {
        if (layout.triangles).place(n as usize, part.triangles(), triangle_room) {
            moved.insert(n);
        }
    }
    if layout.vertices.sparse() || layout.triangles.sparse() {
        return None;
    }
    Some(Placed {
        layout,
        changed,
        rewrite,
        parts,
        splits,
        moved: moved.len(),
    })
}

/// What [`Renderer::update_store`] sent to the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upload {
    /// Nodes whose vertex range was written (every node for a full upload).
    pub nodes: usize,
    /// Nodes whose index range was written: their topology changed, faces were hidden or
    /// shown, or a node their faces use moved (every node for a full upload).
    pub faces: usize,
    /// Nodes given a new range at the end of a pool, new nodes included (none for a full
    /// upload).
    pub moved: usize,
    /// Nodes whose quads' drawn split flipped as their corners moved (decision 0040, section
    /// 2), written again from the first flipped quad to the last, but for those whose index
    /// range was written whole (none for a full upload).
    pub flipped: usize,
    /// Bytes sent to GPU buffers (a pool growing is copied on the GPU, and not counted).
    pub bytes: u64,
    /// Whether everything was uploaded again: the mesh was not uploaded from this store, or
    /// ranges nodes left behind would have filled more than half of a pool.
    pub full: bool,
}

/// Node `n`'s live faces as one chunk, in the order their topology gives (`split.rs`), its
/// quads split as they are now, its hidden faces kept in place but not drawn, with their group
/// colors when the store has groups, and the `bounds` and `cone` it is culled by; and the split
/// it is drawn with.
fn node_part(store: &MeshStore, n: NodeId, bounds: Bounds, cone: Cone) -> (ChunkPart, NodeSplit) {
    let (node, visible) = (store.node(n), store.visible());
    let ids = group_column(store).map(|group| match node.attr(group) {
        Some(AttrVec::U32(v)) => v.as_slice(),
        _ => &[],
    });
    let split = NodeSplit::cut(&node, &store.pos());
    let shown = |j: u32| visible.face(node.first_face + j);
    let t = split.triangles(&node, shown, ids, 0..split.len());
    (ChunkPart::whole(t, bounds, cone), split.kept(&node))
}

/// The store's face group column (decision 0026), when it has one.
fn group_column(store: &MeshStore) -> Option<AttrId> {
    let id = store.attr_id(GROUP)?;
    let def = &store.attrs()[id.index()];
    (def.domain == Domain::Face && def.kind == AttrKind::Label).then_some(id)
}

/// The store's `Rgba8` vertex column `name`, when it has one to draw point colors from.
fn color_column(store: &MeshStore, name: &str) -> Option<AttrId> {
    let id = store.attr_id(name)?;
    let def = &store.attrs()[id.index()];
    (def.domain == Domain::Vertex && def.kind == AttrKind::Rgba8).then_some(id)
}

/// Writes node `n`'s point colors from column `id` into `words`, its vertex slots' words:
/// white where it holds none.
fn node_colors(store: &MeshStore, id: AttrId, n: NodeId, words: &mut [u32]) {
    match store.node(n).attr(id) {
        Some(AttrVec::U32(v)) => (words.iter_mut().zip(v)).for_each(|(w, &c)| *w = c),
        _ => words.fill(WHITE),
    }
}

/// The point color stream of `store` from column `id` (decision 0039, section 9), or `None`
/// when no node holds it.
fn store_colors(store: &MeshStore, layout: &NodeLayout, id: AttrId) -> Option<Vec<u32>> {
    let nodes = 0..store.node_count() as NodeId;
    nodes
        .clone()
        .any(|n| store.node(n).attr(id).is_some())
        .then(|| {
            let mut words = vec![WHITE; layout.vertices.end as usize];
            for n in nodes {
                let span = layout.vertices.spans[n as usize];
                let range = span.start as usize..(span.start + span.len) as usize;
                node_colors(store, id, n, &mut words[range]);
            }
            words
        })
}

/// Writes node `n`'s mask bytes into `bytes` at its vertex range (zeros where it holds no mask).
fn node_mask(store: &MeshStore, mask: AttrId, layout: &NodeLayout, n: NodeId, bytes: &mut [u8]) {
    let span = layout.vertices.spans[n as usize];
    let range = &mut bytes[span.start as usize..(span.start + span.len) as usize];
    match store.node(n).attr(mask) {
        Some(AttrVec::F32(v)) => (range.iter_mut().zip(v)).for_each(|(b, &m)| *b = mask_byte(m)),
        _ => range.fill(0),
    }
}

/// The mask stream of `store` (decision 0015), or `None` when no node holds a mask.
fn store_mask(store: &MeshStore, layout: &NodeLayout) -> Option<Vec<u8>> {
    let mask = store.attr_id(MASK)?;
    let nodes = 0..store.node_count() as NodeId;
    nodes
        .clone()
        .any(|n| store.node(n).attr(mask).is_some())
        .then(|| {
            let mut bytes = vec![0; layout.vertices.end as usize];
            nodes.for_each(|n| node_mask(store, mask, layout, n, &mut bytes));
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            bytes
        })
}

impl Renderer {
    /// Copies a [`MeshStore`] to the GPU: the vertex pool in node order with 16-bit octahedral
    /// normals from the store, and one chunk per node whose faces are in cache-optimized order.
    /// Chunk `k` is node `k`. Vertex ranges are packed tight; each chunk starts on a whole word
    /// of 8 triangles, its unused triangles holding zeros. Every node's normal cone is measured,
    /// however the mesh is drawn. Point colors come from the `color` column.
    pub fn upload_store(&self, store: &MeshStore) -> GpuMesh {
        self.upload_store_from(store, COLOR)
    }

    /// [`Renderer::upload_store`] with the point colors from the `Rgba8` vertex column
    /// `column` (decision 0039, section 9), such as a display column worked out for the
    /// heatmap. Updates keep drawing from it.
    pub fn upload_store_from(&self, store: &MeshStore, column: &str) -> GpuMesh {
        let mut staging = Staging::default();
        staging.measure_cones(true);
        let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
        store.feed(&all, &staging);
        let staged = staging.take();
        let (parts, splits): (Vec<_>, Vec<_>) = gs_par::map(all.len(), |k| {
            let node = &staged[&(k as NodeId)];
            node_part(
                store,
                k as NodeId,
                node.bounds,
                node.cone.unwrap_or(Cone::NONE),
            )
        })
        .into_iter()
        .unzip();
        let layout = NodeLayout::packed(store, &parts);
        let vertices: Vec<&[GpuVertex]> = staged.values().map(|n| n.vertices.as_slice()).collect();
        let mut cpu = assemble(&parts, |v| layout.slot(v), triangle_tight);
        cpu.mask = store_mask(store, &layout);
        cpu.colors = color_column(store, column).and_then(|id| store_colors(store, &layout, id));
        let pools = (&vertices[..], column);
        let mut mesh = upload_mesh(&self.gpu, &self.pipelines, pools, &cpu);
        mesh.layout = Some(layout);
        mesh.staging = Some(staging);
        mesh.splits = splits;
        mesh
    }

    /// Brings `mesh` (from [`Renderer::upload_store`]) up to date after an edit. `dirty` is what
    /// [`MeshStore::take_dirty`] returned: every node with moved positions or normals is fed
    /// through gs-core's node sink, packed in parallel straight into a write view onto its
    /// vertex range, and has its bounds and culling record written again; a node whose topology
    /// changed also has its chunk written again, in place or at the end of the pools (see the
    /// module docs), and quads whose drawn split flipped as their corners moved are written
    /// again (`split.rs`). Nothing else is written. If `mesh` was not uploaded from this store, or the ranges nodes left behind
    /// would fill more than half of a pool, everything is uploaded again. The result draws
    /// exactly like a fresh [`Renderer::upload_store`].
    ///
    /// Normal cones are measured only for a mesh drawn front faces only ([`Sides::Front`]);
    /// drawn both sides it never culls by facing, so the nodes sent then get none. Drawn front
    /// faces only, an update also measures the nodes still without one, and the mesh then holds
    /// exactly what a fresh upload holds, cones included.
    pub fn update_store(
        &self,
        mesh: &mut GpuMesh,
        store: &MeshStore,
        dirty: &[(NodeId, ColumnSet)],
    ) -> Upload {
        if !dirty.is_empty() {
            // Whatever changed may change the picture: a still view's accumulation starts over.
            mesh.changed();
        }
        let Some(placed) = place(mesh, store, dirty) else {
            return self.full(mesh, store);
        };
        let Placed {
            layout,
            changed,
            rewrite,
            parts,
            splits,
            moved,
        } = placed;
        let (slots, triangles) = (layout.vertices.end, layout.triangles.end);
        if !mesh.reserve(&self.gpu, &self.pipelines, slots, triangles) {
            return self.full(mesh, store);
        }
        mesh.resize_chunks(store.node_count());
        let mut bytes = (rewrite.iter().zip(&parts))
            .map(|(&n, part)| {
                let first = layout.first_triangle(n);
                mesh.write_faces(&self.gpu, n, part, first, |v| layout.slot(v))
            })
            .sum::<u64>();
        mesh.splits
            .resize_with(store.node_count(), NodeSplit::default);
        for (&n, split) in rewrite.iter().zip(splits) {
            mesh.splits[n as usize] = split;
        }
        let (slot, first) = (|v| layout.slot(v), |n| layout.first_triangle(n));
        let (flipped, sent) = write_flips(&self.gpu, mesh, store, dirty, &rewrite, slot, first);
        bytes += sent;
        bytes += self.update_mask(mesh, store, &layout, dirty, &changed);
        bytes += self.update_colors(mesh, store, &layout, dirty, &changed);
        let mut feed: BTreeSet<NodeId> = (dirty.iter())
            .filter(|d| d.1.contains(ColumnSet::POSITION) || d.1.contains(ColumnSet::NORMAL))
            .map(|d| d.0)
            .chain(changed.iter().copied())
            .collect();
        let front = mesh.sides() == Sides::Front;
        if front {
            feed.extend(&mesh.coneless);
        }
        let feed: Vec<NodeId> = feed.into_iter().collect();
        // `place` found a layout, so the mesh has its staging; a view opened and not packed
        // into would send zeros.
        let Some(mut staging) = mesh.staging.take() else {
            return self.full(mesh, store);
        };
        let ranges =
            (feed.iter()).map(|&n| (n, layout.first_slot(n), store.node(n).pos.len() as u32));
        let (views, sent) = mesh.vertex_views(&self.gpu, ranges);
        bytes += sent;
        staging.measure_cones(front);
        staging.pack_into(views);
        store.feed(&feed, &staging);
        let staged = staging.take();
        mesh.staging = Some(staging);
        mesh.write_nodes(&staged);
        bytes += mesh.write_tables(&self.gpu, !(rewrite.is_empty() && staged.is_empty()));
        mesh.layout = Some(layout);
        Upload {
            nodes: feed.len(),
            faces: rewrite.len(),
            moved,
            flipped,
            bytes,
            full: false,
        }
    }

    /// Replaces `mesh` with a fresh upload of `store`, its point colors from the same column.
    fn full(&self, mesh: &mut GpuMesh, store: &MeshStore) -> Upload {
        let fresh = self.upload_store_from(store, &mesh.streams.column.clone());
        let bytes = fresh.bytes();
        mesh.replace(fresh);
        Upload {
            nodes: store.node_count(),
            faces: store.node_count(),
            moved: 0,
            flipped: 0,
            bytes,
            full: true,
        }
    }

    /// Writes the mask stream's ranges of the nodes whose mask or topology changed, giving the
    /// mesh a mask stream first if it had none, or dropping it once no node holds a mask (as a
    /// fresh upload would have none). Returns the bytes written.
    fn update_mask(
        &self,
        mesh: &mut GpuMesh,
        store: &MeshStore,
        layout: &NodeLayout,
        dirty: &[(NodeId, ColumnSet)],
        changed: &BTreeSet<NodeId>,
    ) -> u64 {
        let Some(mask) = store.attr_id(MASK) else {
            return 0;
        };
        let mut nodes: BTreeSet<NodeId> = (dirty.iter())
            .filter(|d| d.1.contains(ColumnSet::attr(mask)))
            .map(|d| d.0)
            .collect();
        nodes.extend(changed);
        if nodes.is_empty() {
            return 0;
        }
        if (0..store.node_count() as NodeId).all(|n| store.node(n).attr(mask).is_none()) {
            mesh.drop_mask(&self.gpu, &self.pipelines);
            return 0;
        }
        let fill = || store_mask(store, layout).unwrap_or_default();
        let Some(bytes) = mesh.mask_bytes(&self.gpu, &self.pipelines, fill) else {
            return 0;
        };
        for &n in &nodes {
            node_mask(store, mask, layout, n, bytes);
        }
        let spans: Vec<(u32, u32)> = (nodes.iter())
            .map(|&n| layout.vertices.spans[n as usize])
            .map(|s| (s.start, s.start + s.len))
            .collect();
        mesh.send_mask(&self.gpu, &spans)
    }

    /// Writes the point color stream's ranges of the nodes whose colors or topology changed,
    /// giving the mesh the stream first if it had none, or dropping it once no node holds the
    /// column (as a fresh upload would have none). Returns the bytes written.
    fn update_colors(
        &self,
        mesh: &mut GpuMesh,
        store: &MeshStore,
        layout: &NodeLayout,
        dirty: &[(NodeId, ColumnSet)],
        changed: &BTreeSet<NodeId>,
    ) -> u64 {
        let (gpu, pipelines) = (&self.gpu, &self.pipelines);
        let Some(id) = color_column(store, &mesh.streams.column) else {
            return 0;
        };
        let mut nodes: BTreeSet<NodeId> = (dirty.iter())
            .filter(|d| d.1.contains(ColumnSet::attr(id)))
            .map(|d| d.0)
            .collect();
        nodes.extend(changed);
        if nodes.is_empty() {
            return 0;
        }
        if mesh.streams.colors.is_none() {
            let size = u64::from(layout.vertices.end) * 4;
            if size > gpu.capabilities().max_storage_binding {
                return 0;
            }
            let Some(words) = store_colors(store, layout, id) else {
                return 0;
            };
            return mesh.set_colors(gpu, pipelines, Some(&words));
        }
        if (0..store.node_count() as NodeId).all(|n| store.node(n).attr(id).is_none()) {
            mesh.set_colors(gpu, pipelines, None);
            return 0;
        }
        let mut bytes = 0;
        for &n in &nodes {
            let span = layout.vertices.spans[n as usize];
            let mut words = vec![WHITE; span.len as usize];
            node_colors(store, id, n, &mut words);
            bytes += send_words(gpu, mesh.streams.colors.as_ref(), span.start, &words);
        }
        bytes
    }
}
