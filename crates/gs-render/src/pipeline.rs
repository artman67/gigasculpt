//! Shader modules, bind group layouts and pipelines: the geometry pass's render pipelines
//! (built lazily and cached), the culling compute pipeline, and the builder the full-screen
//! passes share (decision 0019).

use crate::device::Gpu;
use crate::pool::Sides;
use std::sync::{Mutex, PoisonError};
use wgpu::util::DeviceExt;

/// The frame's color texture: what the display pass writes and the window shows.
pub(crate) const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Reversed depth: 1 on the near plane, 0 at infinity (nothing drawn).
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The geometry pass's surface target: octahedral view normal, wire coverage, mask.
pub(crate) const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The geometry pass's second target: each pixel's part flags (decision 0033, section 5), 0
/// where nothing is drawn.
pub(crate) const PARTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;
/// The geometry pass's third target: each pixel's paint (decision 0039, section 9), written in
/// linear light and stored as sRGB; white where nothing is painted or drawn.
pub(crate) const PAINT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Ambient occlusion, raw and filtered: visibility, and the view z of the pixel it is for (f32,
/// which the depth-guided filter and upsample compare at a fraction of a pixel's size).
pub(crate) const OCCLUSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
/// The accumulated mean of displayed pixels (rgb) and of dither coverage (alpha).
pub(crate) const HISTORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The frame's uniforms and shared helpers, put in front of every pass's shader.
pub(crate) const FRAME_WGSL: &str = include_str!("shaders/frame.wgsl");

/// How triangles reach the rasterizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Path {
    /// Indexed chunks, no wireframe.
    Plain,
    /// Indexed chunks, wireframe from hardware barycentrics.
    Barycentric,
    /// Non-indexed chunks reading both pools as storage, wireframe from `vertex_index`.
    Pulled,
}

/// What a geometry pipeline is built for: path, which faces are drawn, and whether it writes
/// the paint target (a frame where no part shows paint leaves it out).
type Key = (Path, Sides, bool);

pub(crate) struct Pipelines {
    /// Group 0 of the geometry pass: the frame's uniforms.
    pub frame: wgpu::BindGroupLayout,
    /// Group 1 for [`Path::Pulled`]: vertex pool, index pool, hidden edges, mask, point
    /// colors, group colors.
    pub pull: wgpu::BindGroupLayout,
    /// Group 1 for [`Path::Barycentric`]: hidden edges, mask, point colors, group colors.
    pub bary: wgpu::BindGroupLayout,
    /// Group 1 for [`Path::Plain`]: the mask (binding 3), one byte a vertex slot, which darkens
    /// what it covers (decision 0015), and the point colors (binding 4), one word a vertex slot
    /// (decision 0039, section 9).
    pub mask: wgpu::BindGroupLayout,
    /// White words, bound for the color streams a mesh lacks.
    pub white: wgpu::Buffer,
    /// Group 2 of the geometry pass: the mesh's `Part`, its model matrix, part to scene
    /// (decision 0029), and its flags (decision 0033), at a dynamic offset into the frame's
    /// buffer of them.
    pub part: wgpu::BindGroupLayout,
    /// Group 3 of the geometry pass: the open deformer's block, weights and tables, and the still
    /// group of none (see `deform.rs`).
    pub deform: wgpu::BindGroupLayout,
    pub still: wgpu::BindGroup,
    /// The culling pass's group: camera, chunk records, draw records out, count out.
    pub cull_layout: wgpu::BindGroupLayout,
    pub cull: wgpu::ComputePipeline,
    /// Live Boolean's peel (`peel.rs`), made the first time a frame peels.
    pub peel: std::sync::OnceLock<crate::peel::Peel>,
    geometry: wgpu::ShaderModule,
    bary_module: Option<wgpu::ShaderModule>,
    cache: Mutex<Vec<(Key, wgpu::RenderPipeline)>>,
}

pub(crate) fn layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

pub(crate) const UNIFORM: wgpu::BindingType = wgpu::BindingType::Buffer {
    ty: wgpu::BufferBindingType::Uniform,
    has_dynamic_offset: false,
    min_binding_size: None,
};

/// A texture of unsigned integers read with `textureLoad`.
pub(crate) const UINT_TEXEL: wgpu::BindingType = wgpu::BindingType::Texture {
    sample_type: wgpu::TextureSampleType::Uint,
    view_dimension: wgpu::TextureViewDimension::D2,
    multisampled: false,
};

/// A texture read with `textureLoad`: unfilterable floats, or depth.
pub(crate) fn texel(depth: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: if depth {
            wgpu::TextureSampleType::Depth
        } else {
            wgpu::TextureSampleType::Float { filterable: false }
        },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

pub(crate) fn module(gpu: &Gpu, label: &str, source: String) -> wgpu::ShaderModule {
    gpu.device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
}

pub(crate) fn bind_layout(
    gpu: &Gpu,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
}

/// A bind group of whole buffers and texture views, bound in order from 0.
pub(crate) fn bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    resources: &[wgpu::BindingResource],
) -> wgpu::BindGroup {
    let entries: Vec<_> = (resources.iter().cloned().enumerate())
        .map(|(i, r)| (i as u32, r))
        .collect();
    bind_at(gpu, layout, &entries)
}

/// A bind group of whole buffers and texture views, each at its binding.
pub(crate) fn bind_at(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    resources: &[(u32, wgpu::BindingResource)],
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry> = (resources.iter())
        .map(|(binding, r)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: r.clone(),
        })
        .collect();
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    })
}

/// A full-screen pass's pipeline: the frame's shared vertex shader drawing one triangle, the
/// fragment entry `fs` of `source` (put after the frame's shared WGSL), one bind group.
pub(crate) fn fullscreen(
    gpu: &Gpu,
    label: &str,
    (source, fs): (&str, &str),
    layout: &wgpu::BindGroupLayout,
    targets: &[wgpu::TextureFormat],
) -> wgpu::RenderPipeline {
    let shader = module(gpu, label, format!("{FRAME_WGSL}\n{source}"));
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
    let targets: Vec<_> = targets.iter().map(|&t| Some(t.into())).collect();
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_full"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}

/// The culling compute pipeline and its group layout.
fn cull_pipeline(gpu: &Gpu) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    use wgpu::ShaderStages as S;
    let layout = bind_layout(
        gpu,
        "cull",
        &[
            layout_entry(0, S::COMPUTE, UNIFORM),
            layout_entry(1, S::COMPUTE, storage(true)),
            layout_entry(2, S::COMPUTE, storage(false)),
            layout_entry(3, S::COMPUTE, storage(false)),
        ],
    );
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cull"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let shader = module(gpu, "cull", include_str!("shaders/cull.wgsl").into());
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cull"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cull"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
    (layout, pipeline)
}

impl Pipelines {
    pub fn new(gpu: &Gpu) -> Pipelines {
        use wgpu::ShaderStages as S;
        let frame = bind_layout(
            gpu,
            "frame",
            &[layout_entry(0, S::VERTEX_FRAGMENT, UNIFORM)],
        );
        let mask_entry = layout_entry(3, S::VERTEX, storage(true));
        let colors_entry = layout_entry(4, S::VERTEX, storage(true));
        let pull = bind_layout(
            gpu,
            "pull",
            &[
                layout_entry(0, S::VERTEX, storage(true)),
                layout_entry(1, S::VERTEX, storage(true)),
                layout_entry(2, S::VERTEX, storage(true)),
                mask_entry,
                colors_entry,
                layout_entry(5, S::VERTEX, storage(true)),
            ],
        );
        let bary = bind_layout(
            gpu,
            "bary",
            &[
                layout_entry(2, S::FRAGMENT, storage(true)),
                mask_entry,
                colors_entry,
                layout_entry(5, S::FRAGMENT, storage(true)),
            ],
        );
        let mask = bind_layout(gpu, "mask", &[mask_entry, colors_entry]);
        let white = (gpu.device).create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("white"),
            contents: &[u8::MAX; 16],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let part_entry = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(crate::part::PART_BYTES),
        };
        let part = bind_layout(
            gpu,
            "part",
            &[layout_entry(0, S::VERTEX_FRAGMENT, part_entry)],
        );
        let (deform, still) = crate::deform::layout(gpu);
        let geometry_wgsl = include_str!("shaders/geometry.wgsl");
        let source = format!(
            "{FRAME_WGSL}\n{}\n{geometry_wgsl}",
            crate::deform::DEFORM_WGSL
        );
        let geometry = module(gpu, "geometry", source.clone());
        let bary_module = gpu.capabilities().barycentric_wireframe.then(|| {
            let bary = include_str!("shaders/bary.wgsl");
            module(
                gpu,
                "bary",
                format!("enable primitive_index;\n{source}\n{bary}"),
            )
        });
        let (cull_layout, cull) = cull_pipeline(gpu);
        Pipelines {
            frame,
            pull,
            bary,
            mask,
            white,
            part,
            deform,
            still,
            cull_layout,
            cull,
            peel: std::sync::OnceLock::new(),
            geometry,
            bary_module,
            cache: Mutex::new(Vec::new()),
        }
    }

    /// The geometry pipeline for `path`, drawing `sides`, with the paint target or without.
    /// `Path::Barycentric` must only be asked for when the device supports it.
    pub fn get(&self, gpu: &Gpu, key: Key) -> wgpu::RenderPipeline {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, p)) = cache.iter().find(|(k, _)| *k == key) {
            return p.clone();
        }
        let pipeline = self.build(gpu, key);
        cache.push((key, pipeline.clone()));
        pipeline
    }

    fn build(&self, gpu: &Gpu, (path, sides, paint): Key) -> wgpu::RenderPipeline {
        const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm16x2];
        let pool = [Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        })];
        let (module, vs, fs, buffers, group1) = match path {
            Path::Plain => (&self.geometry, "vs", "fs", &pool[..], &self.mask),
            Path::Barycentric => (
                self.bary_module.as_ref().unwrap_or(&self.geometry),
                "vs",
                "fs_bary",
                &pool[..],
                &self.bary,
            ),
            Path::Pulled => (&self.geometry, "vs_pull", "fs_pull", &[][..], &self.pull),
        };
        let groups = [
            Some(&self.frame),
            Some(group1),
            Some(&self.part),
            Some(&self.deform),
        ];
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            });
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[
                        Some(SURFACE_FORMAT.into()),
                        Some(PARTS_FORMAT.into()),
                        paint.then(|| PAINT_FORMAT.into()),
                    ],
                }),
                // Both sides by default: open meshes show their back side, shaded from that side.
                primitive: wgpu::PrimitiveState {
                    cull_mode: (sides == Sides::Front).then_some(wgpu::Face::Back),
                    ..wgpu::PrimitiveState::default()
                },
                // Reversed depth: nearer is greater.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }
}
