//! GPU copies of a mesh (vertex pool, index pool, chunk draws, culling records) and of MatCap
//! images.

use std::collections::{BTreeMap, BTreeSet};

use crate::color::srgb16_to_linear;
use crate::cull::{ChunkCull, Cone};
use crate::device::Gpu;
use crate::error::RenderError;
use crate::matcap::{Baked, MatCapImage};
use crate::mesh::{Bounds, Chunk, ChunkPart, CpuPools, GpuVertex};
use crate::nodes::NodeLayout;
use crate::part::Placement;
use crate::pipeline::Pipelines;
use crate::snapshot::Renderer;
use crate::split::NodeSplit;
use crate::staging::{Staged, Staging};
use crate::streams::{Streams, send_words};
use bytemuck::{Pod, Zeroable};
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::util::DeviceExt;

/// A new number for every GPU mesh and MatCap made in this process: what a frame target
/// compares to tell whether its last frame showed the same things (decision 0019). Never part
/// of what is drawn.
pub(crate) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Which faces of a mesh are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sides {
    /// Both sides: an open mesh shows its back, shaded from that side. Culling skips only
    /// chunks outside the view.
    #[default]
    Both,
    /// Front faces only (counter-clockwise seen from outside). Culling also skips chunks whose
    /// every triangle faces away from the eye.
    Front,
}

/// A mesh on the GPU: one vertex pool, one index pool and a chunk table (ARCHITECTURE section
/// 6). Where the device has `multi_draw_indexed_indirect_count`, a compute pass culls chunks
/// against the view each frame and the visible ones are drawn with one indirect call; elsewhere
/// every chunk is drawn from draw records written once on upload.
pub struct GpuMesh {
    pub(crate) vertices: wgpu::Buffer,
    pub(crate) indices: wgpu::Buffer,
    /// One `DrawIndexedIndirectArgs` per chunk; `first_instance` carries the chunk's first triangle.
    pub(crate) indexed_draws: wgpu::Buffer,
    /// One `DrawIndirectArgs` per chunk for the vertex-pulling path.
    pub(crate) pulled_draws: wgpu::Buffer,
    pub(crate) bary_group: wgpu::BindGroup,
    /// `None` when a pool is larger than the adapter lets a shader bind as storage.
    pub(crate) pull_group: Option<wgpu::BindGroup>,
    /// Group 1 of the plain path: the mask and point color streams.
    pub(crate) mask_group: wgpu::BindGroup,
    hidden_edges: wgpu::Buffer,
    /// The mask, point color and group color streams (see `streams.rs`).
    pub(crate) streams: Streams,
    /// The culling pass's buffers; `None` without `multi_draw_indexed_indirect_count`.
    pub(crate) cull: Option<CullBuffers>,
    /// Where each node's vertices sit, for a mesh uploaded from a `MeshStore`.
    pub(crate) layout: Option<NodeLayout>,
    /// Group 3: an open deformer's preview, or the still group (see `deform.rs`).
    pub(crate) deform: crate::deform::Previewed,
    /// Where the store's nodes are packed when fed (decision 0012).
    pub(crate) staging: Option<Staging>,
    /// How each node's quads are drawn split, for a mesh uploaded from a `MeshStore`.
    pub(crate) splits: Vec<NodeSplit>,
    chunks: Vec<Chunk>,
    /// The culling records as last written, one per chunk.
    records: Vec<ChunkCull>,
    /// Chunks written without a measured normal cone (while drawn both sides): their records
    /// hold none, so they are never skipped for facing away until they get one.
    pub(crate) coneless: BTreeSet<u32>,
    /// Chunks that draw nothing, every face hidden: their records make the culling pass skip
    /// them.
    blank: BTreeSet<u32>,
    /// A chunk's range moved or changed length since the draw records were written.
    draws_stale: bool,
    /// Vertex slots in the pool.
    vertex_count: usize,
    bytes: u64,
    sides: Sides,
    culling: bool,
    /// This mesh's number and how many times its data was written since (see [`GpuMesh::stamp`]).
    id: u64,
    revision: u64,
    /// Where the mesh is drawn, and whether (decision 0029; see [`GpuMesh::placed`]).
    pub(crate) placement: Placement,
}

/// What the culling pass reads and writes for one mesh.
pub(crate) struct CullBuffers {
    /// One [`ChunkCull`] per chunk.
    pub chunks: wgpu::Buffer,
    /// The visible chunks' draw records, written by the pass.
    pub draws: wgpu::Buffer,
    /// How many draw records the pass wrote.
    pub drawn: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IndexedDraw {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PulledDraw {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

/// A buffer holding `parts` one after another, copied straight into the mapped buffer (no
/// joined copy first); 16 zero bytes when there are none, since empty bindings are invalid.
fn buffer_of(gpu: &Gpu, label: &str, parts: &[&[u8]], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let size: usize = parts.iter().map(|p| p.len()).sum();
    if size == 0 {
        return buffer(gpu, label, &[], usage);
    }
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT as usize) as u64,
        usage,
        mapped_at_creation: true,
    });
    {
        // A buffer mapped at creation always maps; the copy is only a fallback.
        let Ok(mut view) = buffer.slice(..).get_mapped_range_mut() else {
            return self::buffer(gpu, label, &parts.concat(), usage);
        };
        let mut rest = view.slice(..);
        for part in parts {
            let (mut dst, tail) = rest.split_at(part.len());
            dst.copy_from_slice(part);
            rest = tail;
        }
    }
    buffer.unmap();
    buffer
}

/// A buffer holding `bytes`; 16 zero bytes when there are none, since empty bindings are invalid.
fn buffer(gpu: &Gpu, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let contents = if bytes.is_empty() {
        &[0u8; 16][..]
    } else {
        bytes
    };
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
}

fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

/// Draw records for every chunk: indexed, and for the vertex-pulling path.
fn draw_records(chunks: &[Chunk]) -> (Vec<IndexedDraw>, Vec<PulledDraw>) {
    chunks
        .iter()
        .map(|c| {
            let (first, count) = (c.first_triangle * 3, c.triangle_count * 3);
            let indexed = IndexedDraw {
                index_count: count,
                instance_count: 1,
                first_index: first,
                base_vertex: 0,
                first_instance: c.first_triangle,
            };
            let pulled = PulledDraw {
                vertex_count: count,
                instance_count: 1,
                first_vertex: first,
                first_instance: 0,
            };
            (indexed, pulled)
        })
        .unzip()
}

fn cull_buffers(gpu: &Gpu, records: &[ChunkCull]) -> Option<CullBuffers> {
    use wgpu::BufferUsages as B;
    if !gpu.capabilities().multi_draw_indirect_count || records.is_empty() {
        return None;
    }
    let output = |label, size: usize| {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size as u64,
            usage: B::STORAGE.union(B::INDIRECT).union(B::COPY_SRC),
            mapped_at_creation: false,
        })
    };
    Some(CullBuffers {
        chunks: buffer(
            gpu,
            "chunk culling",
            bytemuck::cast_slice(records),
            B::STORAGE.union(B::COPY_DST).union(B::COPY_SRC),
        ),
        draws: output("culled draws", size_of::<IndexedDraw>() * records.len()),
        drawn: output("culled count", 16),
    })
}

/// The draw record buffers of `chunks`: indexed, and for the vertex-pulling path.
fn draw_buffers(gpu: &Gpu, chunks: &[Chunk]) -> (wgpu::Buffer, wgpu::Buffer) {
    let (indexed, pulled) = draw_records(chunks);
    let usage = wgpu::BufferUsages::INDIRECT;
    (
        buffer(gpu, "indexed draws", bytemuck::cast_slice(&indexed), usage),
        buffer(gpu, "pulled draws", bytemuck::cast_slice(&pulled), usage),
    )
}

/// A copy of `old`, `size` bytes long, the rest zero: the copy is made on the GPU.
pub(crate) fn grown(gpu: &Gpu, old: &wgpu::Buffer, size: u64) -> wgpu::Buffer {
    let new = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grown pool"),
        size,
        usage: old.usage(),
        mapped_at_creation: false,
    });
    let mut encoder =
        (gpu.device).create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(old, 0, &new, 0, old.size());
    gpu.queue.submit([encoder.finish()]);
    new
}

/// The vertex pool: `vertices` one after another. Updates write into it and
/// [`Renderer::read_pools`] reads it back.
fn vertex_pool(gpu: &Gpu, vertices: &[&[GpuVertex]]) -> wgpu::Buffer {
    use wgpu::BufferUsages as B;
    let parts: Vec<&[u8]> = vertices.iter().map(|v| bytemuck::cast_slice(v)).collect();
    let usage = B::VERTEX
        .union(B::STORAGE)
        .union(B::COPY_DST)
        .union(B::COPY_SRC);
    buffer_of(gpu, "vertex pool", &parts, usage)
}

/// Each path's group 1 over a mesh's buffers (vertex pool, index pool, hidden edges, and the
/// mask, point color and group color streams): the plain path's, the barycentric path's, and
/// the vertex-pulling path's, which is `None` when a pool is larger than the adapter lets a
/// shader bind as storage.
fn groups(
    gpu: &Gpu,
    pipelines: &Pipelines,
    [vertices, indices, hidden]: [&wgpu::Buffer; 3],
    [mask, colors, group_colors]: [&wgpu::Buffer; 3],
) -> (wgpu::BindGroup, wgpu::BindGroup, Option<wgpu::BindGroup>) {
    let group = |layout, entries: &[wgpu::BindGroupEntry]| {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries,
        })
    };
    let fits = [vertices, indices]
        .iter()
        .all(|b| b.size() <= gpu.capabilities().max_storage_binding);
    let pulled = [
        entry(0, vertices),
        entry(1, indices),
        entry(2, hidden),
        entry(3, mask),
        entry(4, colors),
        entry(5, group_colors),
    ];
    let bary = [
        entry(2, hidden),
        entry(3, mask),
        entry(4, colors),
        entry(5, group_colors),
    ];
    (
        group(&pipelines.mask, &[entry(3, mask), entry(4, colors)]),
        group(&pipelines.bary, &bary),
        fits.then(|| group(&pipelines.pull, &pulled)),
    )
}

/// The pools on the GPU: the vertex pool is `vertices` one after another; the point colors
/// came from the vertex column `column`.
pub(crate) fn upload_mesh(
    gpu: &Gpu,
    pipelines: &Pipelines,
    (vertices, column): (&[&[GpuVertex]], &str),
    cpu: &CpuPools,
) -> GpuMesh {
    use wgpu::BufferUsages as B;
    let vertex_count = vertices.iter().map(|v| v.len()).sum();
    let vertices = vertex_pool(gpu, vertices);
    let indices = buffer(
        gpu,
        "index pool",
        bytemuck::cast_slice(&cpu.indices),
        // Written again where faces are hidden or shown; read back by `read_pools`.
        B::INDEX
            .union(B::STORAGE)
            .union(B::COPY_DST)
            .union(B::COPY_SRC),
    );
    let hidden = buffer(
        gpu,
        "hidden edges",
        bytemuck::cast_slice(&cpu.hidden_edges),
        // Written where a node's faces change; copied when the pool grows.
        B::STORAGE.union(B::COPY_DST).union(B::COPY_SRC),
    );
    let (indexed_draws, pulled_draws) = draw_buffers(gpu, &cpu.chunks);
    let streams = Streams::new(
        gpu,
        cpu.mask.as_deref(),
        (cpu.colors.as_deref(), column),
        cpu.groups.as_deref(),
    );
    let pools = [&vertices, &indices, &hidden];
    let (mask_group, bary_group, pull_group) =
        groups(gpu, pipelines, pools, streams.bound(pipelines));
    let cull = cull_buffers(gpu, &cpu.cull);
    let mut mesh = GpuMesh {
        vertices,
        indices,
        indexed_draws,
        pulled_draws,
        bary_group,
        pull_group,
        mask_group,
        hidden_edges: hidden,
        streams,
        cull,
        layout: None,
        deform: crate::deform::Previewed::still(pipelines),
        staging: None,
        splits: Vec::new(),
        chunks: cpu.chunks.clone(),
        records: cpu.cull.clone(),
        coneless: BTreeSet::new(),
        blank: cpu.blank.clone(),
        draws_stale: false,
        vertex_count,
        bytes: 0,
        sides: Sides::Both,
        culling: true,
        id: next_id(),
        revision: 0,
        placement: Placement::default(),
    };
    mesh.bytes = mesh.sizes();
    mesh
}

impl GpuMesh {
    /// The chunk table, in index-pool order.
    pub fn chunks(&self) -> &[Chunk] {
        self.chunks.as_slice()
    }

    /// Number of triangles drawn (quads count as two).
    pub fn triangle_count(&self) -> u32 {
        self.chunks.iter().map(|c| c.triangle_count).sum()
    }

    /// Bounds of everything drawn.
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        self.chunks.iter().for_each(|c| b.union(&c.bounds));
        b
    }

    /// The corners of every chunk's bounds: what [`crate::Camera::fit`] frames and what the
    /// depth planes enclose. Tighter than the corners of the whole mesh's box. In the scene,
    /// where the mesh is placed. With an open deformer's preview, also where it moves them.
    pub fn fit_points(&self) -> Vec<[f32; 3]> {
        let t = self.placement.transform;
        self.chunks
            .iter()
            .flat_map(|c| c.bounds.corners())
            .chain(self.deform.fit.iter().copied())
            .map(|p| t.point(gs_core::Vec3::from_array(p)).to_array())
            .collect()
    }

    /// Which faces are drawn. [`Sides::Both`] unless set.
    pub fn sides(&self) -> Sides {
        self.sides
    }

    /// Drawn front faces only, chunks are also culled by facing, so the next
    /// [`Renderer::update_store`] measures the normal cones of the nodes it sends and of those
    /// sent without one before.
    pub fn set_sides(&mut self, sides: Sides) {
        self.sides = sides;
    }

    /// Whether chunks are culled on the GPU when the device can (on unless turned off). Off
    /// draws every chunk, as on devices without `multi_draw_indexed_indirect_count`.
    pub fn culling(&self) -> bool {
        self.culling
    }

    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// Size of every GPU buffer the mesh holds, in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Which mesh this is and how often its data changed: equal stamps draw the same pixels
    /// (with the same sides). A frame target restarts its accumulation when a stamp changes.
    pub fn stamp(&self) -> (u64, u64) {
        (self.id, self.revision)
    }

    /// Records that the mesh's data changed (see [`GpuMesh::stamp`]).
    pub(crate) fn changed(&mut self) {
        self.revision += 1;
    }

    /// Bytes of vertex data the pool holds (the buffer may be padded past them).
    fn vertex_bytes(&self) -> usize {
        self.vertex_count * size_of::<GpuVertex>()
    }

    /// The sizes of its buffers, added up.
    fn sizes(&self) -> u64 {
        let cull = (self.cull.as_ref()).map_or(0, |c| c.chunks.size() + c.draws.size());
        let buffers = [
            &self.vertices,
            &self.indices,
            &self.hidden_edges,
            &self.indexed_draws,
            &self.pulled_draws,
        ];
        buffers.iter().map(|b| b.size()).sum::<u64>() + cull + self.streams.bytes()
    }

    /// Makes every path's group 1 again over the mesh's buffers, after one was replaced.
    pub(crate) fn rebind(&mut self, gpu: &Gpu, pipelines: &Pipelines) {
        let pools = [&self.vertices, &self.indices, &self.hidden_edges];
        let streams = self.streams.bound(pipelines);
        (self.mask_group, self.bary_group, self.pull_group) =
            groups(gpu, pipelines, pools, streams);
        self.bytes = self.sizes();
    }

    /// Room in the pools for `slots` vertex slots and `triangles` triangles (decision 0021,
    /// item 7): a pool too small for them grows by a quarter more than they need, copied on the
    /// GPU, and so do the streams. False, changing nothing, when a pool would pass the largest
    /// buffer the device makes.
    pub(crate) fn reserve(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        slots: u32,
        triangles: u32,
    ) -> bool {
        let (slots, triangles) = (u64::from(slots), u64::from(triangles));
        let max = gpu.device.limits().max_buffer_size;
        let need = [slots * 16, triangles * 12, triangles.div_ceil(8) * 4];
        if need.iter().any(|&n| n > max) {
            return false;
        }
        let pools = [
            &mut self.vertices,
            &mut self.indices,
            &mut self.hidden_edges,
        ];
        let mut grew = false;
        for (pool, need) in pools.into_iter().zip(need) {
            if pool.size() < need {
                *pool = grown(gpu, pool, (need + need / 4).next_multiple_of(4).min(max));
                grew = true;
            }
        }
        grew |= self.streams.grow(gpu, slots, triangles);
        let mask = &self.streams.mask_bytes;
        let short = mask.as_ref().is_some_and(|m| (m.len() as u64) < slots);
        if short && let Some(mut bytes) = self.streams.mask_bytes.take() {
            let len = (slots + slots / 4).next_multiple_of(4);
            bytes.resize(len as usize, 0);
            let fits = len <= gpu.capabilities().max_storage_binding;
            self.set_mask(gpu, pipelines, fits.then_some(bytes));
        } else if grew {
            self.rebind(gpu, pipelines);
        }
        self.bytes = self.sizes();
        true
    }

    /// `count` chunks: new ones empty until written, dropped ones forgotten.
    pub(crate) fn resize_chunks(&mut self, count: usize) {
        if count == self.chunks.len() {
            return;
        }
        let empty = Chunk {
            first_triangle: 0,
            triangle_count: 0,
            bounds: Bounds::EMPTY,
        };
        self.chunks.resize(count, empty);
        (self.records).resize(count, ChunkCull::new(0, 0, &Bounds::EMPTY, Cone::NONE));
        let kept = |n: &u32| (*n as usize) < count;
        self.blank.retain(kept);
        self.coneless.retain(kept);
        self.draws_stale = true;
    }

    /// Takes `fresh`'s buffers and chunks, keeping this mesh's sides and culling settings.
    pub(crate) fn replace(&mut self, fresh: GpuMesh) {
        *self = GpuMesh {
            sides: self.sides,
            culling: self.culling,
            placement: self.placement,
            ..fresh
        };
    }

    /// One write view per node `(n, first, len)` onto its `len` vertex slots from slot
    /// `first`, for packing to fill in place; each is sent when dropped. Returns them and their
    /// bytes.
    pub(crate) fn vertex_views(
        &self,
        gpu: &Gpu,
        nodes: impl IntoIterator<Item = (u32, u32, u32)>,
    ) -> (BTreeMap<u32, wgpu::QueueWriteBufferView>, u64) {
        let unit = size_of::<GpuVertex>() as u64;
        let mut bytes = 0;
        let views = (nodes.into_iter())
            .filter_map(|(n, first, len)| {
                let size = wgpu::BufferSize::new(u64::from(len) * unit)?;
                let offset = u64::from(first) * unit;
                let view = gpu.queue.write_buffer_with(&self.vertices, offset, size)?;
                bytes += size.get();
                Some((n, view))
            })
            .collect();
        (views, bytes)
    }

    /// Takes staged nodes' boxes and normal cones into their chunk bounds and culling records.
    /// (Their slots were written while they were packed, into [`GpuMesh::vertex_views`].)
    pub(crate) fn write_nodes(&mut self, staged: &BTreeMap<u32, Staged>) {
        for (&n, node) in staged {
            let chunk = &mut self.chunks[n as usize];
            chunk.bounds = node.bounds;
            let (first_triangle, mut count) = (chunk.first_triangle, chunk.triangle_count);
            if self.blank.contains(&n) {
                count = 0;
            }
            let cone = node.cone.unwrap_or(Cone::NONE);
            self.records[n as usize] = ChunkCull::new(first_triangle, count, &node.bounds, cone);
            if node.cone.is_some() {
                self.coneless.remove(&n);
            } else {
                self.coneless.insert(n);
            }
        }
    }

    /// Writes chunk `n` from `part` at triangle `first` of the index pool, a multiple of 8: its
    /// faces as they are now (decision 0015: hidden ones in place but drawing nothing), their
    /// hidden edges, their group colors when the mesh shows groups, and its culling record's
    /// draw. Returns the bytes written.
    pub(crate) fn write_faces(
        &mut self,
        gpu: &Gpu,
        n: u32,
        part: &ChunkPart,
        first: u32,
        slot: impl Fn(u32) -> u32,
    ) -> u64 {
        let indices: Vec<u32> = part.indices(slot).collect();
        let hidden = part.hidden_words();
        let at = |unit: usize, k: u32| u64::from(k) * unit as u64;
        for (buffer, offset, data) in [
            (&self.indices, at(12, first), bytemuck::cast_slice(&indices)),
            (
                &self.hidden_edges,
                at(4, first / 8),
                bytemuck::cast_slice(&hidden),
            ),
        ] {
            if !data.is_empty() {
                gpu.queue.write_buffer(buffer, offset, data);
            }
        }
        let count = part.triangles();
        let chunk = &mut self.chunks[n as usize];
        if (chunk.first_triangle, chunk.triangle_count) != (first, count) {
            (chunk.first_triangle, chunk.triangle_count) = (first, count);
            self.draws_stale = true;
        }
        let drawn = if part.drawn() {
            self.blank.remove(&n);
            count
        } else {
            self.blank.insert(n);
            0
        };
        self.records[n as usize] = self.records[n as usize].at(first).draws(drawn);
        let groups = send_words(gpu, self.streams.groups.as_ref(), first, part.group_words());
        ((indices.len() + hidden.len()) * size_of::<u32>()) as u64 + groups
    }

    /// Writes the draw records again if a chunk's range changed, making new culling buffers
    /// if the number of chunks did, and otherwise the culling table once if `changed`.
    /// Returns the bytes written.
    pub(crate) fn write_tables(&mut self, gpu: &Gpu, changed: bool) -> u64 {
        let mut bytes = 0;
        if std::mem::take(&mut self.draws_stale) {
            (self.indexed_draws, self.pulled_draws) = draw_buffers(gpu, &self.chunks);
            bytes += self.indexed_draws.size() + self.pulled_draws.size();
            let table = (size_of::<ChunkCull>() * self.records.len()) as u64;
            let culls = gpu.capabilities().multi_draw_indirect_count && table > 0;
            if self.cull.as_ref().map(|c| c.chunks.size()) != culls.then_some(table) {
                self.cull = cull_buffers(gpu, &self.records);
                self.bytes = self.sizes();
                return bytes + if culls { table } else { 0 };
            }
            self.bytes = self.sizes();
        }
        let Some(cull) = self.cull.as_ref().filter(|_| changed) else {
            return bytes;
        };
        let table: &[u8] = bytemuck::cast_slice(&self.records);
        gpu.queue.write_buffer(&cull.chunks, 0, table);
        bytes + table.len() as u64
    }
}

/// What the GPU holds for a mesh, as the shaders read it (see [`Renderer::read_pools`]), as
/// if packed tight: a mesh from a `MeshStore` gives each node room to grow and moves a node
/// that outgrows it (decision 0021), so its nodes' ranges are read in node order and the
/// indices and records renumbered to match, which is what a fresh upload holds wherever its
/// nodes sit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolBytes {
    /// The vertex pool: 16 bytes a vertex slot.
    pub vertices: Vec<u8>,
    /// The index pool: 12 bytes a triangle (a hidden face's triangles name one corner three
    /// times).
    pub indices: Vec<u8>,
    /// The culling records, 48 bytes a chunk; empty where the device does not cull.
    pub culling: Vec<u8>,
    /// The mask stream, one byte a vertex slot padded to whole words; empty without a mask.
    pub mask: Vec<u8>,
    /// The point color stream, 4 bytes a vertex slot; empty without one.
    pub colors: Vec<u8>,
    /// The group color stream, 4 bytes a triangle; empty without one.
    pub groups: Vec<u8>,
}

impl Renderer {
    /// Reads back what the GPU holds for `mesh`'s vertices, indices, culling records and
    /// streams. Two meshes with the same bytes here and the same chunk table draw the same
    /// pixels, so this shows that a mesh brought up to date holds exactly what a fresh upload
    /// would.
    pub fn read_pools(&self, mesh: &GpuMesh) -> Result<PoolBytes, RenderError> {
        let culling = mesh.cull.as_ref().map(|c| &c.chunks);
        let s = &mesh.streams;
        let mask = s.mask_bytes.as_ref().map(|_| &s.mask);
        let buffers = [
            Some(&mesh.vertices),
            Some(&mesh.indices),
            culling,
            mask,
            s.colors.as_ref(),
            s.groups.as_ref(),
        ];
        let sizes = buffers.map(|b| b.map_or(0, wgpu::Buffer::size));
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pool readback"),
            size: sizes.iter().sum::<u64>().max(16),
            usage: wgpu::BufferUsages::COPY_DST.union(wgpu::BufferUsages::MAP_READ),
            mapped_at_creation: false,
        });
        let mut encoder =
            (self.gpu.device).create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut at = 0;
        for (buffer, size) in buffers.iter().zip(sizes) {
            if let Some(buffer) = buffer {
                encoder.copy_buffer_to_buffer(buffer, 0, &readback, at, size);
            }
            at += size;
        }
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = self.read_mapped(&readback)?;
        let (vertices, rest) = bytes.split_at(sizes[0] as usize);
        let (indices, rest) = rest.split_at(sizes[1] as usize);
        let (culled, rest) = rest.split_at(sizes[2] as usize);
        let (mask, rest) = rest.split_at(sizes[3] as usize);
        let (colors, groups) = rest.split_at(sizes[4] as usize);
        let records = mesh.records.len() * size_of::<ChunkCull>();
        let culling = if culling.is_some() {
            culled[..records].to_vec()
        } else {
            Vec::new()
        };
        let mask_len = s.mask_bytes.as_ref().map_or(0, Vec::len);
        if let Some(layout) = &mesh.layout {
            let raw = PoolBytes {
                vertices: vertices.to_vec(),
                indices: indices.to_vec(),
                culling,
                mask: mask[..mask_len].to_vec(),
                colors: colors.to_vec(),
                groups: groups.to_vec(),
            };
            return Ok(layout.tight(&raw, &mesh.chunks));
        }
        let triangles = mesh.triangle_count() as usize;
        let first = |b: &[u8], len: usize| b[..len.min(b.len())].to_vec();
        Ok(PoolBytes {
            vertices: vertices[..mesh.vertex_bytes()].to_vec(),
            indices: indices[..triangles * 12].to_vec(),
            culling,
            mask: mask[..mask_len].to_vec(),
            colors: first(colors, mesh.vertex_count * 4),
            groups: first(groups, triangles * 4),
        })
    }
}

/// A MatCap on the GPU as half-float textures: D (diffuse response, times `albedo` when
/// drawn), S (specular, added) and, for an imported two-sphere image read as flat plus crease,
/// the crease image shown where ambient occlusion darkens (catalog item
/// `material.cavity-shading`, decision 0019).
pub struct GpuMatCap {
    pub(crate) diffuse: wgpu::TextureView,
    pub(crate) specular: wgpu::TextureView,
    pub(crate) crease: Option<wgpu::TextureView>,
    /// Linear, multiplies D.
    pub(crate) albedo: [f32; 3],
    /// The look's GGX roughness (perceptual), for specular occlusion.
    pub(crate) roughness: f32,
    /// How much paint tints S: the look's metalness (research 11), 0 for an image.
    pub(crate) metalness: f32,
    /// Cavity Transition, -1 to 1: leans a flat-plus-crease pair's blend (see `transition`).
    pub(crate) transition: f32,
    /// The images are display-referred (8-bit images as they look), so their pixels are shown
    /// with exposure but no view transform (see `display.rs`).
    pub(crate) display_referred: bool,
    pub(crate) id: u64,
}

/// A baked look: its D and S layers, physical light.
pub(crate) fn upload_bake(gpu: &Gpu, baked: &Baked) -> GpuMatCap {
    GpuMatCap {
        diffuse: texture(gpu, baked.size, &baked.d),
        specular: texture(gpu, baked.size, &baked.s),
        crease: None,
        albedo: baked.albedo(),
        roughness: baked.roughness(),
        metalness: baked.metalness(),
        transition: 0.0,
        display_referred: false,
        id: next_id(),
    }
}

/// An 8-bit MatCap image as D with an empty S, shown as the file looks; with a crease image
/// when given.
pub(crate) fn upload_matcap(
    gpu: &Gpu,
    image: &MatCapImage,
    crease: Option<&MatCapImage>,
) -> GpuMatCap {
    GpuMatCap {
        diffuse: upload_image(gpu, image),
        specular: texture(gpu, 1, &[[0, 0, 0, 0]]),
        crease: crease.map(|b| upload_image(gpu, b)),
        albedo: [1.0; 3],
        roughness: 1.0,
        metalness: 0.0,
        transition: 0.0,
        display_referred: true,
        id: next_id(),
    }
}

/// A square RGBA16F texture of `size` texels a side from half-float bits.
fn texture(gpu: &Gpu, size: u32, texels: &[[u16; 4]]) -> wgpu::TextureView {
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("matcap"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST),
        view_formats: &[],
    });
    let layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(size * 8),
        rows_per_image: None,
    };
    gpu.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(texels),
        layout,
        extent,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// An 8-bit image, display-linear (shown without a view transform). Alpha is not drawn yet,
/// so the texture is opaque.
fn upload_image(gpu: &Gpu, image: &MatCapImage) -> wgpu::TextureView {
    let texels: Vec<[u16; 4]> = (image.pixels.iter())
        .map(|p| {
            let rgb = [p[0], p[1], p[2]].map(srgb16_to_linear);
            [rgb[0], rgb[1], rgb[2], 1.0].map(|c| half::f16::from_f32(c).to_bits())
        })
        .collect();
    texture(gpu, image.size, &texels)
}
