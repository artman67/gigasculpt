//! Live Boolean's peel (decision 0057, section 9, as amended for LB8): a group's cut drawn
//! from its operands as they are now, while the CPU's result is not current. A mesh peeled as
//! the [`Operand::Base`] starts a group, and every peeled mesh after it in the scene's list joins
//! it until the next base. Per group and new frame, after the geometry pass (`shaders/peel.wgsl`):
//!
//! 1. a clear: the first depth target to the near plane, the first mask to 0 (outside);
//! 2. each layer: every operand, both sides, keeping per pixel the nearest fragment behind the
//!    layer before (its depth moved by its operand's tie key) and its hit, then a full-screen
//!    pass that sets or clears that operand's bit, folds the bits and keeps the first layer the
//!    fold finds inside as the chosen one;
//! 3. a last pass: every operand again into the frame's surface, part flags, paint and depth,
//!    each fragment kept only where it is the chosen hit at the chosen depth, so occlusion,
//!    light, the outline and accumulation treat the cut as any part.
//!
//! The targets (22 bytes a pixel) are made the first time a frame peels and dropped by the first
//! new frame that peels nothing. The operands draw from their own uploads.

use std::sync::PoisonError;

use crate::device::Gpu;
use crate::frame::FrameTarget;
use crate::pipeline::{
    DEPTH_FORMAT, FRAME_WGSL, PAINT_FORMAT, PARTS_FORMAT, Path, Pipelines, SURFACE_FORMAT,
    UINT_TEXEL, UNIFORM, bind_at, bind_group, bind_layout, layout_entry, module, texel,
};
use crate::pool::{GpuMesh, Sides};
use crate::snapshot::{Renderer, draw};
use crate::timing::Timing;
use gs_core::transform::Similarity;
use wgpu::util::DeviceExt;

/// Most operands a peeled group takes: one bit each in the fold's words. A larger group draws
/// its meshes as unpeeled.
pub const PEEL_OPERANDS: usize = 32;

/// Most layers a group peels: a pixel still undecided after them draws nothing of the group.
const MAX_LAYERS: u32 = 8;

/// A layer's hit and the chosen hit: 0 for none, else twice the operand's place plus one, plus
/// one on a front face.
const HIT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;
/// The operand bits of each pixel's ray, one an operand.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// The targets' bytes a pixel: three depths and two masks at 4 bytes, two hits at 1.
const BYTES_PER_PIXEL: u64 = 22;

/// How a mesh joins Live Boolean's peel (see the module's notes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    /// The group's first part, which every other one joins or cuts.
    Base,
    /// Added to the cut so far.
    Union,
    /// Subtracted from the cut so far, its walls drawn in this color (sRGB bytes).
    Difference([u8; 3]),
    /// What the cut so far keeps of it, its walls drawn in this color (sRGB bytes).
    Intersect([u8; 3]),
}

impl Operand {
    /// Whether it cuts the group (a Subtract or Intersect), so its kept faces are walls.
    fn cuts(self) -> bool {
        matches!(self, Operand::Difference(_) | Operand::Intersect(_))
    }

    /// Its flags (see `shaders/peel.wgsl`) but the mark and the paint's source: bit 7, its
    /// operation in bits 4 and 5, and its walls' color in the bytes above.
    fn flags(self) -> u32 {
        let (code, [r, g, b]) = match self {
            Operand::Base => (0, [255; 3]),
            Operand::Union => (1, [255; 3]),
            Operand::Difference(c) => (2, c),
            Operand::Intersect(c) => (3, c),
        };
        0x80 | code << 4 | u32::from(r) << 8 | u32::from(g) << 16 | u32::from(b) << 24
    }
}

impl GpuMesh {
    /// This mesh as an operand of Live Boolean's peel, or drawn as a part (`None`, the
    /// default). Setting it sends nothing to the GPU.
    #[must_use]
    pub fn peeled(mut self, operand: Option<Operand>) -> GpuMesh {
        self.placement.peel = operand;
        self
    }
}

/// A group the peel draws: its operands in list order, the base first, with each one's words of
/// its `Part` (flags low byte, the flags above it, its place, its tie key), its fold (count, a
/// bit per Subtract, a bit per Intersect, the mark) and its layer cap.
pub(crate) struct Group<'a> {
    meshes: Vec<&'a GpuMesh>,
    words: Vec<[u32; 4]>,
    fold: [u32; 4],
    layers: u32,
}

/// Each operand's tie key (decision 0057, section 9, as amended for LB8): with m the group's
/// Subtract and Intersect operands, the k-th of them `+k` for a Subtract and `-k` for an
/// Intersect, the base `-(m + 1)` and the j-th Add `-(m + 1 + j)`.
fn keys(operands: &[Operand]) -> Vec<i32> {
    let m: i32 = operands.iter().map(|o| i32::from(o.cuts())).sum();
    let (mut k, mut j) = (0, 0);
    (operands.iter())
        .map(|operand| match operand {
            Operand::Base => -(m + 1),
            Operand::Union => {
                j += 1;
                -(m + 1 + j)
            }
            Operand::Difference(_) => {
                k += 1;
                k
            }
            Operand::Intersect(_) => {
                k += 1;
                -k
            }
        })
        .collect()
}

impl<'a> Group<'a> {
    fn of(meshes: Vec<&'a GpuMesh>) -> Group<'a> {
        let operands: Vec<Operand> = (meshes.iter())
            .map(|m| m.placement.peel.unwrap_or(Operand::Union))
            .collect();
        let words = (meshes
            .iter()
            .zip(&operands)
            .zip(keys(&operands))
            .enumerate())
        .map(|(i, ((mesh, operand), key))| {
            let p = &mesh.placement;
            // Its point colors while it shows them (`part.rs`'s `Source::Points`).
            let points = p.painted && mesh.streams.colors.is_some();
            let flags = u32::from(p.marked) | u32::from(points) << 1 | operand.flags();
            [flags & 0xFF, flags >> 8, i as u32, key as u32]
        })
        .collect();
        let bits = |pick: fn(&Operand) -> bool| {
            (operands.iter().enumerate())
                .filter(|(_, o)| pick(o))
                .fold(0u32, |bits, (i, _)| bits | 1 << i)
        };
        let marked = (meshes.iter().zip(&operands)).any(|(m, o)| m.placement.marked && !o.cuts());
        let cuts = operands.iter().filter(|o| o.cuts()).count() as u32;
        Group {
            fold: [
                operands.len() as u32,
                bits(|o| matches!(o, Operand::Difference(_))),
                bits(|o| matches!(o, Operand::Intersect(_))),
                u32::from(marked),
            ],
            layers: if cuts == 0 {
                1
            } else {
                (3 + 2 * cuts).min(MAX_LAYERS)
            },
            meshes,
            words,
        }
    }
}

/// Every peeled mesh of `groups`.
pub(crate) fn members<'g>(groups: &'g [Group]) -> impl Iterator<Item = &'g &'g GpuMesh> {
    groups.iter().flat_map(|g| g.meshes.iter())
}

/// What the accumulation keys a frame by for each peeled mesh: its upload, sides, place and
/// flags.
pub(crate) fn stamps<'g>(
    groups: &'g [Group],
) -> impl Iterator<Item = ((u64, u64), Sides, Similarity, u32)> + 'g {
    (groups.iter())
        .flat_map(|g| g.meshes.iter().zip(&g.words))
        .map(|(m, w)| {
            (
                m.stamp(),
                m.sides(),
                m.placement.transform,
                w[0] | w[1] << 8,
            )
        })
}

/// The peel's targets, each the frame's size: two depths taking turns as the layer peeled and
/// the layer before, the layer's hit, two masks taking turns as before and after the layer, and
/// the chosen depth and hit.
pub(crate) struct Targets {
    depth: [wgpu::TextureView; 2],
    hit: wgpu::TextureView,
    mask: [wgpu::TextureView; 2],
    chosen: wgpu::TextureView,
    chosen_hit: wgpu::TextureView,
    bytes: u64,
}

/// A texture of `format` and `size` that passes draw into and read.
pub(crate) fn target_view(
    gpu: &Gpu,
    label: &str,
    format: wgpu::TextureFormat,
    [width, height]: [u32; 2],
) -> wgpu::TextureView {
    use wgpu::TextureUsages as T;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: T::RENDER_ATTACHMENT.union(T::TEXTURE_BINDING),
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

impl Targets {
    fn new(gpu: &Gpu, size: [u32; 2]) -> Targets {
        let view = |label, format| target_view(gpu, label, format, size);
        Targets {
            depth: [
                view("peel depth", DEPTH_FORMAT),
                view("peel depth", DEPTH_FORMAT),
            ],
            hit: view("peel hit", HIT_FORMAT),
            mask: [
                view("peel mask", MASK_FORMAT),
                view("peel mask", MASK_FORMAT),
            ],
            chosen: view("peel chosen depth", DEPTH_FORMAT),
            chosen_hit: view("peel chosen hit", HIT_FORMAT),
            bytes: BYTES_PER_PIXEL * u64::from(size[0]) * u64::from(size[1]),
        }
    }
}

impl FrameTarget {
    /// Bytes of GPU memory the peel holds for this target: none until a frame peels, and none
    /// again after the first new frame that peels nothing.
    pub fn peel_bytes(&self) -> u64 {
        let held = self.peel.lock().unwrap_or_else(PoisonError::into_inner);
        held.as_ref().map_or(0, |t| t.bytes)
    }
}

/// A render pipeline on the pool's vertices (when the vertex entry is the geometry pass's
/// `vs`) or none: entries `vs` and `fs` of `shader`, bind groups `groups`, color `targets`, and
/// a depth target tested by `depth` and written, or none. The peel's and the ghosts' own.
pub(crate) fn pipeline(
    gpu: &Gpu,
    (shader, vs, fs): (&wgpu::ShaderModule, &str, &str),
    groups: &[Option<&wgpu::BindGroupLayout>],
    targets: &[wgpu::TextureFormat],
    depth: Option<wgpu::CompareFunction>,
) -> wgpu::RenderPipeline {
    let layout = (gpu.device).create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(fs),
        bind_group_layouts: groups,
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm16x2];
    // The pool's vertex as the geometry pass reads it (pipeline.rs).
    let pool = [Some(wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    let targets: Vec<_> = targets.iter().map(|&t| Some(t.into())).collect();
    let options = wgpu::PipelineCompilationOptions::default;
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs),
                compilation_options: options(),
                buffers: if vs == "vs" { &pool } else { &[] },
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs),
                compilation_options: options(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth.map(|compare| wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}

/// The peel's three pipelines and their fourth groups: the layer reads the depth before it; the
/// resolve reads the layer, its hit, the mask before it and the fold; the last pass reads the
/// fold and the chosen depth and hit. Made the first time a frame peels.
pub(crate) struct Peel {
    layer: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    last: wgpu::RenderPipeline,
    layer_group: wgpu::BindGroupLayout,
    resolve_group: wgpu::BindGroupLayout,
    last_group: wgpu::BindGroupLayout,
}

impl Peel {
    fn new(gpu: &Gpu, p: &Pipelines) -> Peel {
        let source = format!(
            "{FRAME_WGSL}\n{}\n{}\n{}",
            crate::deform::DEFORM_WGSL,
            include_str!("shaders/geometry.wgsl"),
            include_str!("shaders/peel.wgsl")
        );
        let shader = module(gpu, "peel", source);
        let depth = texel(true);
        let layout = |label, entries: &[(u32, wgpu::BindingType)]| {
            let entries: Vec<_> = (entries.iter())
                .map(|&(binding, ty)| layout_entry(binding, wgpu::ShaderStages::FRAGMENT, ty))
                .collect();
            bind_layout(gpu, label, &entries)
        };
        let layer_group = layout("peel layer", &[(0, depth)]);
        let resolve_group = layout(
            "peel resolve",
            &[(1, depth), (2, UINT_TEXEL), (3, UINT_TEXEL), (4, UNIFORM)],
        );
        let last_group = layout("peel", &[(4, UNIFORM), (5, depth), (6, UINT_TEXEL)]);
        let geometry = |fourth| [&p.frame, &p.mask, &p.part, &p.deform, fourth].map(Some);
        let greater = Some(wgpu::CompareFunction::Greater);
        Peel {
            layer: pipeline(
                gpu,
                (&shader, "vs", "fs_layer"),
                &geometry(&layer_group),
                &[HIT_FORMAT],
                greater,
            ),
            resolve: pipeline(
                gpu,
                (&shader, "vs_full", "fs_resolve"),
                &[None, None, None, None, Some(&resolve_group)],
                &[MASK_FORMAT, HIT_FORMAT],
                Some(wgpu::CompareFunction::GreaterEqual),
            ),
            last: pipeline(
                gpu,
                (&shader, "vs", "fs_peel"),
                &geometry(&last_group),
                &[SURFACE_FORMAT, PARTS_FORMAT, PAINT_FORMAT],
                greater,
            ),
            layer_group,
            resolve_group,
            last_group,
        }
    }
}

/// A peel pass at `viewport`: `colors` and `depth` each loaded or cleared as given, all stored.
fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    (label, [x, y, w, h]): (&str, [u32; 4]),
    colors: &[(&wgpu::TextureView, wgpu::LoadOp<wgpu::Color>)],
    (depth, depth_load): (&wgpu::TextureView, wgpu::LoadOp<f32>),
    timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
) -> wgpu::RenderPass<'e> {
    let store = wgpu::StoreOp::Store;
    let colors: Vec<_> = (colors.iter())
        .map(|&(view, load)| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store },
            })
        })
        .collect();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &colors,
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: depth_load,
                store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: timestamps,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
    pass.set_scissor_rect(x, y, w, h);
    pass
}

/// What one group's passes share.
struct Run<'a> {
    gpu: &'a Gpu,
    peel: &'a Peel,
    targets: &'a Targets,
    frame: &'a wgpu::BindGroup,
    viewport: [u32; 4],
    timing: Option<&'a Timing>,
    meshes: &'a [&'a GpuMesh],
    parts: wgpu::BindGroup,
    stride: u64,
    fold: wgpu::Buffer,
}

const ZERO: wgpu::LoadOp<wgpu::Color> = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);

impl Run<'_> {
    /// The first depth target to the near plane, the first mask to outside.
    fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        let t = self.targets;
        let near = (&t.depth[0], wgpu::LoadOp::Clear(1.0));
        begin(
            encoder,
            ("peel clear", self.viewport),
            &[(&t.mask[0], ZERO)],
            near,
            None,
        );
    }

    /// The group's operands drawn into `pass` with `pipeline` and `fourth` as group 4.
    fn operands(
        &self,
        pass: &mut wgpu::RenderPass,
        pipeline: &wgpu::RenderPipeline,
        fourth: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, self.frame, &[]);
        pass.set_bind_group(4, fourth, &[]);
        for (i, mesh) in self.meshes.iter().enumerate() {
            pass.set_bind_group(2, &self.parts, &[(i as u64 * self.stride) as u32]);
            draw(pass, mesh, Path::Plain, None);
        }
    }

    /// Layer `k`: the nearest fragments behind layer `k - 1` (the near plane for the first),
    /// then its resolve into the mask and the chosen targets.
    fn layer(&self, encoder: &mut wgpu::CommandEncoder, k: usize) {
        let (t, texture) = (self.targets, wgpu::BindingResource::TextureView);
        let (read, write) = (k % 2, (k + 1) % 2);
        let before = bind_at(
            self.gpu,
            &self.peel.layer_group,
            &[(0, texture(&t.depth[read]))],
        );
        let timestamps = self.timing.and_then(|t| t.render("peel layer"));
        let layer = (&t.depth[write], wgpu::LoadOp::Clear(0.0));
        let label = ("peel layer", self.viewport);
        let mut pass = begin(encoder, label, &[(&t.hit, ZERO)], layer, timestamps);
        self.operands(&mut pass, &self.peel.layer, &before);
        drop(pass);
        let reads = [
            (1, texture(&t.depth[write])),
            (2, texture(&t.hit)),
            (3, texture(&t.mask[read])),
            (4, self.fold.as_entire_binding()),
        ];
        let group = bind_at(self.gpu, &self.peel.resolve_group, &reads);
        let (hit, depth) = if k == 0 {
            (ZERO, wgpu::LoadOp::Clear(0.0))
        } else {
            (wgpu::LoadOp::Load, wgpu::LoadOp::Load)
        };
        let colors = [(&t.mask[write], ZERO), (&t.chosen_hit, hit)];
        let timestamps = self.timing.and_then(|t| t.render("peel resolve"));
        let label = ("peel resolve", self.viewport);
        let mut pass = begin(encoder, label, &colors, (&t.chosen, depth), timestamps);
        pass.set_pipeline(&self.peel.resolve);
        pass.set_bind_group(4, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The chosen fragments into the frame's surface, part flags, paint and depth.
    fn last(&self, encoder: &mut wgpu::CommandEncoder, target: &FrameTarget) {
        let (t, texture) = (self.targets, wgpu::BindingResource::TextureView);
        let reads = [
            (4, self.fold.as_entire_binding()),
            (5, texture(&t.chosen)),
            (6, texture(&t.chosen_hit)),
        ];
        let group = bind_at(self.gpu, &self.peel.last_group, &reads);
        let load = wgpu::LoadOp::Load;
        let colors = [
            (&target.surface, load),
            (&target.parts, load),
            (&target.paint, load),
        ];
        let timestamps = self.timing.and_then(|t| t.render("peel"));
        let label = ("peel", self.viewport);
        let depth = (&target.depth, wgpu::LoadOp::Load);
        let mut pass = begin(encoder, label, &colors, depth, timestamps);
        self.operands(&mut pass, &self.peel.last, &group);
    }
}

impl Renderer {
    /// Whether this renderer draws Live Boolean's peel: not on an integrated GPU, which draws
    /// the CPU's result until a proxy exists (decision 0057, section 9, as amended for LB8),
    /// and only where a pipeline takes five bind groups.
    pub fn peels(&self) -> bool {
        !self.gpu.integrated() && self.gpu.device.limits().max_bind_groups >= 5
    }

    /// A frame's shown meshes, split: the ones the geometry pass draws, the ghosts (`live.rs`)
    /// and the groups the peel draws (a peeled ghost is in both). A group of one or of more
    /// than [`PEEL_OPERANDS`], a peeled mesh before any base, and every mesh on a renderer
    /// that does not peel draw as unpeeled.
    pub(crate) fn split<'a>(
        &self,
        meshes: &[&'a GpuMesh],
    ) -> (Vec<&'a GpuMesh>, Vec<&'a GpuMesh>, Vec<Group<'a>>) {
        let shown: Vec<&GpuMesh> = (meshes.iter().copied())
            .filter(|m| m.placement.shown)
            .collect();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (i, mesh) in shown.iter().enumerate().filter(|_| self.peels()) {
            match (mesh.placement.peel, groups.last_mut()) {
                (Some(Operand::Base), _) => groups.push(vec![i]),
                (Some(_), Some(group)) => group.push(i),
                _ => {}
            }
        }
        groups.retain(|g| (2..=PEEL_OPERANDS).contains(&g.len()));
        let mut peeled = vec![false; shown.len()];
        groups.iter().flatten().for_each(|&i| peeled[i] = true);
        let ghost = |m: &&GpuMesh| m.placement.ghost.is_some();
        let ghosts = shown.iter().copied().filter(ghost).collect();
        let plain = (shown.iter().zip(&peeled))
            .filter(|&(m, &p)| !p && !ghost(m))
            .map(|(&m, _)| m)
            .collect();
        let groups = (groups.into_iter())
            .map(|g| Group::of(g.into_iter().map(|i| shown[i]).collect()))
            .collect();
        (plain, ghosts, groups)
    }

    /// Draws each of `groups` through the peel (see the module's notes) at `viewport` (clipped
    /// to the frame) with the frame's `uniforms`, after the geometry pass. Without groups it
    /// drops the targets.
    pub(crate) fn peel_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (target, viewport): (&FrameTarget, [u32; 4]),
        groups: &[Group],
        uniforms: &wgpu::Buffer,
        timing: Option<&Timing>,
    ) {
        let mut held = target.peel.lock().unwrap_or_else(PoisonError::into_inner);
        if groups.is_empty() {
            *held = None;
            return;
        }
        let size = target.size();
        let targets =
            held.get_or_insert_with(|| Targets::new(&self.gpu, [size.width, size.height]));
        if viewport[2] == 0 || viewport[3] == 0 {
            return;
        }
        let peel = (self.pipelines.peel).get_or_init(|| Peel::new(&self.gpu, &self.pipelines));
        let uniforms = [uniforms.as_entire_binding()];
        let frame = bind_group(&self.gpu, &self.pipelines.frame, &uniforms);
        for group in groups {
            let (parts, stride) = self.parts_of(&group.meshes, &group.words);
            let fold = (self.gpu.device).create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("peel fold"),
                contents: bytemuck::cast_slice(&group.fold),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let run = Run {
                gpu: &self.gpu,
                peel,
                targets,
                frame: &frame,
                viewport,
                timing,
                meshes: &group.meshes,
                parts,
                stride,
                fold,
            };
            run.clear(encoder);
            (0..group.layers as usize).for_each(|k| run.layer(encoder, k));
            run.last(encoder, target);
        }
    }
}
