//! The open deformer's preview (decision 0041, amended 2026-10-12 for `DF9a`): while a deformer
//! of DF2's closed forms is open, a part's GPU mesh keeps its points as they were at the open
//! and the vertex stage moves each one (`shaders/deform.wgsl`) by a small uniform block of the
//! operations, at the point's weight from a stream sent once per session. An update sends the
//! block alone, 464 bytes; Reset binds a block of no operations and keeps the weights; the end
//! of the session binds the shared still block and frees both.
//!
//! The weights are 16-bit unorm, two to a word, in the vertex pool's slot order: exact at 0 and
//! 1, within 7.6e-6 between (section 3 of the amendment).
//!
//! Amended 2026-10-13 for `DF9b`: noise, the cage and the curve too. The cage and the curve read
//! a table, gs-core's own grid or arcs (`Prepared::table`), from a third buffer beside the
//! weights, sent only when it changes and replaced only when it grows.

use bytemuck::{Pod, Zeroable};
use gs_core::Vec3;
use gs_core::deform::Deform;
use gs_core::transform::{Op, Prepared, Transform};
use wgpu::util::DeviceExt;

use crate::device::Gpu;
use crate::error::RenderError;
use crate::pipeline::{
    FRAME_WGSL, Pipelines, UNIFORM, bind_group, bind_layout, layout_entry, module,
};
use crate::pool::GpuMesh;
use crate::snapshot::Renderer;

/// The vertex stage's operations, after `FRAME_WGSL` and before every pass that draws parts.
pub(crate) const DEFORM_WGSL: &str = concat!(
    include_str!("shaders/deform.wgsl"),
    "\n",
    include_str!("shaders/deform_noise.wgsl"),
    "\n",
    include_str!("shaders/deform_tables.wgsl"),
);

/// Operations the block holds: a chain of up to four (section 1).
const OPS: usize = 4;

/// One operation as the vertex stage reads it (`DeformOp` in `shaders/deform.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct OpBlock {
    axes: [[f32; 4]; 3],
    origin: [f32; 4],
    lo: [f32; 4],
    hi: [f32; 4],
    code: [u32; 4],
}

/// The uniform block (`Deforms`): how many operations, then the operations.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct Block {
    count: [u32; 4],
    ops: [OpBlock; OPS],
}

/// The block's size: 464 bytes.
pub(crate) const BLOCK_BYTES: u64 = size_of::<Block>() as u64;

/// The tables' buffer: read by the vertex stage, written while the tables fit it.
const TABLES: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST);

/// A point and its normal, as [`Renderer::read_preview`] reads them back.
pub type PointNormal = ([f32; 3], [f32; 3]);

/// What the window hands the renderer while a deformer is open (see [`Renderer::preview`]).
#[derive(Clone, Copy, Debug)]
pub struct Preview<'a> {
    /// The operations in the part's own space, in order: at most four deformers (bend, fold,
    /// twist, taper, shear, clip, spherize, noise, the cage, the curve), each with its box and
    /// spherize radius.
    pub ops: &'a [Transform],
    /// Each vertex slot's weight, node by node and every slot of a node, as gs-core's
    /// `MeshStore::weights` gives them.
    pub weights: &'a [f32],
    /// A number that changes when the weights do: while it stays, they are not sent again.
    pub weighed: u64,
}

/// A GPU mesh's preview: what its draws bind as group 3, and what it holds while previewed.
pub(crate) struct Previewed {
    /// The shared still block, or the mesh's own while it is previewed.
    pub(crate) group: wgpu::BindGroup,
    /// The mesh's own block and weights, from the first preview to the end of the session.
    own: Option<Own>,
    /// The moved chunks' points framing the view (see `fit`); empty while not previewed.
    pub(crate) fit: Vec<[f32; 3]>,
    /// Drawn moved: the mesh's node boxes and normal cones are the open's, so it is not culled.
    pub(crate) on: bool,
}

struct Own {
    block: wgpu::Buffer,
    sent: Block,
    weighed: u64,
    weights: wgpu::Buffer,
    /// The chain's tables, at least 16 bytes, and the ones last sent.
    tables: wgpu::Buffer,
    sent_tables: Vec<[f32; 4]>,
    group: wgpu::BindGroup,
}

/// A chain made ready for the vertex stage: its block, its cages' and curves' tables one after
/// another, each transform prepared once for the fit, and how far its noise can spread a
/// chunk's points apart (twice `√3 |amount|` each).
struct Chain {
    block: Block,
    tables: Vec<[f32; 4]>,
    prepared: Vec<Prepared>,
    pad: f32,
}

impl Previewed {
    /// Drawn as stored.
    pub(crate) fn still(pipelines: &Pipelines) -> Previewed {
        Previewed {
            group: pipelines.still.clone(),
            own: None,
            fit: Vec::new(),
            on: false,
        }
    }
}

/// Group 3 of every pass that draws parts: the block (binding 0), the weights (binding 1) and
/// the tables (binding 2), and the shared still group, a block of no operations, bound when a
/// mesh is not previewed.
pub(crate) fn layout(gpu: &Gpu) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    use wgpu::ShaderStages as S;
    let storage = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let stages = S::VERTEX.union(S::COMPUTE);
    let layout = bind_layout(
        gpu,
        "deform",
        &[
            layout_entry(0, stages, UNIFORM),
            layout_entry(1, stages, storage),
            layout_entry(2, stages, storage),
        ],
    );
    let zeros = |label, size: usize, usage| {
        (gpu.device).create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &vec![0u8; size],
            usage,
        })
    };
    let block = zeros(
        "still block",
        size_of::<Block>(),
        wgpu::BufferUsages::UNIFORM,
    );
    let weights = zeros("still weights", 16, wgpu::BufferUsages::STORAGE);
    let tables = zeros("still tables", 16, wgpu::BufferUsages::STORAGE);
    let still = bind_group(
        gpu,
        &layout,
        &[
            block.as_entire_binding(),
            weights.as_entire_binding(),
            tables.as_entire_binding(),
        ],
    );
    (layout, still)
}

/// `ops` made ready; `None` for a chain the vertex stage does not run.
fn chain(ops: &[Transform]) -> Option<Chain> {
    if ops.len() > OPS {
        return None;
    }
    let mut block = Block::zeroed();
    block.count[0] = ops.len() as u32;
    let (mut tables, mut prepared, mut pad) = (Vec::new(), Vec::new(), 0.0);
    for (slot, t) in block.ops.iter_mut().zip(ops) {
        let ready = t.prepared();
        *slot = operation(t, tables.len())?;
        tables.extend(ready.table());
        if let Op::Deform(Deform::Noise { amount, .. }) = t.op {
            pad += 2.0 * 3f32.sqrt() * amount.abs();
        }
        prepared.push(ready);
    }
    Some(Chain {
        block,
        tables,
        prepared,
        pad,
    })
}

/// One operation: its frame, box, numbers and code, its table's first entry at `first`.
fn operation(t: &Transform, first: usize) -> Option<OpBlock> {
    let Op::Deform(d) = &t.op else {
        return None;
    };
    let (kind, axis, toward, amount, second, smooth) = match *d {
        Deform::Bend {
            axis,
            toward,
            angle,
        } => (0, axis, toward, angle, 0.0, false),
        Deform::Fold {
            axis,
            toward,
            angle,
        } => (1, axis, toward, angle, 0.0, false),
        Deform::Twist { axis, angle } => (2, axis, 0, angle, 0.0, false),
        Deform::Taper {
            axis,
            amount,
            exponent,
        } => (3, axis, 0, amount, exponent, false),
        Deform::Shear {
            axis,
            toward,
            amount,
            smooth,
        } => (4, axis, toward, amount, 0.0, smooth),
        Deform::Clip { axis, depth } => (5, axis, 0, 0.0, depth, false),
        Deform::Spherize { amount, radius } => (6, 0, 0, amount, radius.unwrap_or(0.0), false),
        // The seed's low and high words in place of the axes.
        Deform::Noise {
            amount,
            scale,
            seed,
        } => (
            7,
            seed as u32 as usize,
            (seed >> 32) as usize,
            amount,
            scale,
            false,
        ),
        Deform::Lattice(_) => (8, 0, first, 0.0, 0.0, false),
        Deform::Curve(ref c) => (9, c.axis, first, 0.0, 0.0, false),
    };
    let o = t.frame.origin;
    let (x, y, z) = t.frame.axes.axes();
    let far = o.len2().sqrt();
    let column = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
    let corner = |v: Option<Vec3>| column(v.unwrap_or(Vec3::ZERO), 0.0);
    let keep = (t.scope.axes.iter().enumerate()).fold(0, |b, (a, &k)| b | (u32::from(k) << a));
    Some(OpBlock {
        axes: [column(x, far), column(y, amount), column(z, second)],
        origin: column(o, 0.0),
        lo: corner(t.scope.bounds.map(|b| b.lo)),
        hi: corner(t.scope.bounds.map(|b| b.hi)),
        code: [
            kind,
            axis as u32,
            toward as u32,
            keep | (u32::from(smooth) << 3),
        ],
    })
}

/// `weights`, node by node, as unorm16 words in the pool's slot order over `spans` (each
/// node's first slot and slots) up to slot `end`; `None` when they are not one a slot.
fn words(weights: &[f32], (spans, end): &(Vec<(u32, u32)>, u32)) -> Option<Vec<u32>> {
    let slots: usize = spans.iter().map(|&(_, len)| len as usize).sum();
    if slots != weights.len() {
        return None;
    }
    let mut words = vec![0u32; end.div_ceil(2).max(4) as usize];
    let mut given = weights.iter();
    for &(start, len) in spans {
        for (slot, w) in (start..start + len).zip(&mut given) {
            let q = (w.clamp(0.0, 1.0) * 65535.0).round() as u32;
            words[slot as usize / 2] |= q << (16 * (slot % 2));
        }
    }
    Some(words)
}

/// Points that frame the moved part: each chunk's centre moved by the chain at weights one
/// third, two thirds and one, as the corners of a cube half the chunk's diagonal across from
/// it, widened by the chain's noise.
fn fit(chain: &Chain, mesh: &GpuMesh) -> Vec<[f32; 3]> {
    let mut points = Vec::new();
    for chunk in mesh
        .chunks()
        .iter()
        .filter(|c| c.bounds.min[0] <= c.bounds.max[0])
    {
        let (lo, hi) = (
            chunk.bounds.min.map(f64::from),
            chunk.bounds.max.map(f64::from),
        );
        let centre = Vec3::from_array([0, 1, 2].map(|a| f64::midpoint(lo[a], hi[a]) as f32));
        let span = |a: usize| hi[a] - lo[a];
        let half = ((0..3).map(|a| span(a) * span(a)).sum::<f64>().sqrt() / 2.0) as f32 + chain.pad;
        for w in [1.0 / 3.0, 2.0 / 3.0, 1.0] {
            let at = (chain.prepared.iter()).fold(centre, |p, t| t.deformed(p, w));
            for k in 0..8 {
                let side = |a: u32| if k >> a & 1 == 0 { -half } else { half };
                points.push((at + Vec3::new(side(0), side(1), side(2))).to_array());
            }
        }
    }
    points
}

impl Renderer {
    /// Draws `mesh`, a part uploaded from its store, as an open deformer moves it (decision
    /// 0041, amended 2026-10-12 for `DF9a` and 2026-10-13 for `DF9b`), or as stored with `None`.
    /// The weights are sent when `weighed` changes, the block and the tables each when they do;
    /// no operations draw the part as stored and keep all three; `None` frees them. Any change
    /// restarts the mesh's accumulation. Returns the bytes sent.
    pub fn preview(&self, mesh: &mut GpuMesh, preview: Option<Preview>) -> u64 {
        let ready = preview.and_then(|p| Some((p, chain(p.ops)?, mesh.layout.as_ref()?)));
        let Some((p, chain, layout)) = ready else {
            if mesh.deform.on || mesh.deform.own.is_some() {
                mesh.deform = Previewed::still(&self.pipelines);
                mesh.changed();
            }
            return 0;
        };
        let mut sent = 0;
        let stale = (mesh.deform.own.as_ref()).is_none_or(|own| own.weighed != p.weighed);
        if stale {
            let Some(words) = words(p.weights, &layout.vertex_ranges()) else {
                return self.preview(mesh, None);
            };
            let own = self.own(&chain, &words, p.weighed);
            sent += BLOCK_BYTES + own.weights.size() + own.tables.size();
            mesh.deform.own = Some(own);
        }
        let Some(own) = &mut mesh.deform.own else {
            return sent;
        };
        let on = !p.ops.is_empty();
        if on {
            sent += self.send(own, &chain);
        }
        let group = if on {
            &own.group
        } else {
            &self.pipelines.still
        }
        .clone();
        if sent > 0 || on != mesh.deform.on {
            mesh.changed();
            mesh.deform.fit = if on { fit(&chain, mesh) } else { Vec::new() };
        }
        (mesh.deform.group, mesh.deform.on) = (group, on);
        sent
    }

    /// Sends the chain's block and its tables, each only where it differs from the one last
    /// sent; tables that outgrow their buffer go into a new one, and a new group. Returns the
    /// bytes sent.
    fn send(&self, own: &mut Own, chain: &Chain) -> u64 {
        let queue = &self.gpu.queue;
        let mut sent = 0;
        if own.sent != chain.block {
            queue.write_buffer(&own.block, 0, bytemuck::bytes_of(&chain.block));
            (own.sent, sent) = (chain.block, BLOCK_BYTES);
        }
        if own.sent_tables != chain.tables {
            let bytes: &[u8] = bytemuck::cast_slice(&chain.tables);
            if bytes.len() as u64 > own.tables.size() {
                own.tables = self.buffer("deform tables", bytes, TABLES);
                own.group = self.group(&own.block, &own.weights, &own.tables);
            } else if !bytes.is_empty() {
                queue.write_buffer(&own.tables, 0, bytes);
            }
            own.sent_tables.clone_from(&chain.tables);
            sent += bytes.len() as u64;
        }
        sent
    }

    /// A mesh's own block, weights holding `words` and tables, holding the chain's, and their
    /// group.
    fn own(&self, chain: &Chain, words: &[u32], weighed: u64) -> Own {
        use wgpu::BufferUsages as B;
        let block = self.buffer(
            "deform block",
            bytemuck::bytes_of(&chain.block),
            B::UNIFORM.union(B::COPY_DST),
        );
        let weights = self.buffer("deform weights", bytemuck::cast_slice(words), B::STORAGE);
        let tables = self.buffer("deform tables", bytemuck::cast_slice(&chain.tables), TABLES);
        let group = self.group(&block, &weights, &tables);
        Own {
            block,
            sent: chain.block,
            weighed,
            weights,
            tables,
            sent_tables: chain.tables.clone(),
            group,
        }
    }

    /// A buffer holding `contents`, at least 16 bytes.
    fn buffer(&self, label: &str, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        let mut padded = contents.to_vec();
        padded.resize(contents.len().max(16), 0);
        (self.gpu.device).create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &padded,
            usage,
        })
    }

    /// Group 3 of a previewed mesh.
    fn group(
        &self,
        block: &wgpu::Buffer,
        weights: &wgpu::Buffer,
        tables: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        bind_group(
            &self.gpu,
            &self.pipelines.deform,
            &[
                block.as_entire_binding(),
                weights.as_entire_binding(),
                tables.as_entire_binding(),
            ],
        )
    }

    /// Every point and normal of `mesh` as its draws place them now, preview and all, read back
    /// from the GPU node by node and every slot of a node: what the vertex stage's `deformed`
    /// makes of each slot (`shaders/deform_read.wgsl`). Empty for a mesh not uploaded from a
    /// store.
    pub fn read_preview(&self, mesh: &GpuMesh) -> Result<Vec<PointNormal>, RenderError> {
        let Some(layout) = &mesh.layout else {
            return Ok(Vec::new());
        };
        let slots = mesh.vertices.size() / 16;
        let out = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview readback"),
            size: slots * 32,
            usage: wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC),
            mapped_at_creation: false,
        });
        self.run_read(mesh, &out, slots);
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview readback copy"),
            size: out.size(),
            usage: wgpu::BufferUsages::COPY_DST.union(wgpu::BufferUsages::MAP_READ),
            mapped_at_creation: false,
        });
        let mut encoder =
            (self.gpu.device).create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(&out, 0, &readback, 0, out.size());
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = self.read_mapped(&readback)?;
        let floats: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
        let (spans, _) = layout.vertex_ranges();
        let three = |v: [f32; 4]| [v[0], v[1], v[2]];
        Ok((spans.iter())
            .flat_map(|&(start, len)| start..start + len)
            .map(|s| {
                (
                    three(floats[2 * s as usize]),
                    three(floats[2 * s as usize + 1]),
                )
            })
            .collect())
    }

    /// Records and submits the readback pass over `slots` slots of `mesh`'s pool into `out`.
    fn run_read(&self, mesh: &GpuMesh, out: &wgpu::Buffer, slots: u64) {
        use wgpu::ShaderStages as S;
        let gpu = &self.gpu;
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let layout = bind_layout(
            gpu,
            "preview readback",
            &[
                layout_entry(0, S::COMPUTE, storage(true)),
                layout_entry(1, S::COMPUTE, storage(false)),
            ],
        );
        let pipeline = read_pipeline(gpu, &layout, &self.pipelines.deform);
        let group = bind_group(
            gpu,
            &layout,
            &[mesh.vertices.as_entire_binding(), out.as_entire_binding()],
        );
        let mut encoder =
            (gpu.device).create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(1, &group, &[]);
            pass.set_bind_group(3, &mesh.deform.group, &[]);
            let groups = slots.div_ceil(64) as u32;
            pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
        }
        gpu.queue.submit([encoder.finish()]);
    }
}

/// The readback pass: the frame's helpers, the vertex stage's operations and the reader.
fn read_pipeline(
    gpu: &Gpu,
    read: &wgpu::BindGroupLayout,
    deform: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let reader = include_str!("shaders/deform_read.wgsl");
    let shader = module(
        gpu,
        "preview readback",
        format!("{FRAME_WGSL}\n{DEFORM_WGSL}\n{reader}"),
    );
    let layout = (gpu.device).create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("preview readback"),
        bind_group_layouts: &[None, Some(read), None, Some(deform)],
        immediate_size: 0,
    });
    (gpu.device).create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("preview readback"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("read_preview"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}
