//! The full-screen passes of the frame (decision 0019): occlusion and its filter
//! (`shaders/occlusion.wgsl`), live subsurface's diffuse pass, and the display
//! (`shaders/display.wgsl`) with the lighting stage (`shaders/lighting.wgsl`, with Lit mode in
//! `shaders/lit.wgsl`) inside it. Each reads the frame's uniforms at binding 0 and textures
//! after it; the diffuse pass shares the display's bindings (decision 0023).

use crate::device::Gpu;
use crate::lit::DIFFUSE_FORMAT;
use crate::pipeline::{
    HISTORY_FORMAT, OCCLUSION_FORMAT, UINT_TEXEL, UNIFORM, bind_layout, fullscreen, layout_entry,
    texel,
};
use crate::ui::UI_FORMAT;

/// The pipelines and their group layouts.
pub(crate) struct Passes {
    pub occlusion_layout: wgpu::BindGroupLayout,
    pub filter_layout: wgpu::BindGroupLayout,
    pub display_layout: wgpu::BindGroupLayout,
    pub occlusion: wgpu::RenderPipeline,
    pub filter: wgpu::RenderPipeline,
    /// Live subsurface's diffuse irradiance, for the display pass to gather.
    pub diffuse: wgpu::RenderPipeline,
    /// Lighting and display: a new sample, averaged into the history. A settled frame runs no
    /// pass: it copies the last sample's picture (`frame.rs`).
    pub display: wgpu::RenderPipeline,
    /// Nearest and clamped at the edges: the display pass gathers the part flags with it.
    pub gather: wgpu::Sampler,
}

/// The group layouts: occlusion, its filter, and the display with its lighting.
fn layouts(gpu: &Gpu) -> [wgpu::BindGroupLayout; 3] {
    use wgpu::ShaderStages as S;
    let f = S::FRAGMENT;
    let filterable = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
    let nearest = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering);
    // Uniforms, depth, the surface target, and the occlusion (the filter's input, or the
    // filtered occlusion the lighting reads).
    let reads = [
        layout_entry(0, S::VERTEX_FRAGMENT, UNIFORM),
        layout_entry(1, f, texel(true)),
        layout_entry(2, f, texel(false)),
        layout_entry(3, f, texel(false)),
    ];
    // Then the MatCap's D layer, its sampler, S layer and crease image, the history, and Lit
    // mode's uniforms, prefiltered environment cube, its sampler, the DFG table, the diffuse
    // irradiance, the part flags with the sampler that gathers them (decision 0033), and the
    // paint (decision 0039, section 9).
    let mut display = reads.to_vec();
    let cube = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::Cube,
        multisampled: false,
    };
    display.extend([
        layout_entry(4, f, filterable),
        layout_entry(5, f, sampler),
        layout_entry(6, f, filterable),
        layout_entry(7, f, filterable),
        layout_entry(8, f, texel(false)),
        layout_entry(9, f, UNIFORM),
        layout_entry(10, f, cube),
        layout_entry(11, f, sampler),
        layout_entry(12, f, texel(false)),
        layout_entry(13, f, texel(false)),
        layout_entry(14, f, UINT_TEXEL),
        layout_entry(15, f, nearest),
        layout_entry(16, f, texel(false)),
    ]);
    [
        bind_layout(gpu, "occlusion", &reads[..3]),
        bind_layout(gpu, "occlusion filter", &reads),
        bind_layout(gpu, "display", &display),
    ]
}

impl Passes {
    pub fn new(gpu: &Gpu) -> Passes {
        let [occlusion_layout, filter_layout, display_layout] = layouts(gpu);
        let occlusion_wgsl = include_str!("shaders/occlusion.wgsl");
        let display_wgsl = concat!(
            include_str!("shaders/lighting.wgsl"),
            include_str!("shaders/lit.wgsl"),
            include_str!("shaders/display.wgsl")
        );
        let pipeline = |label, source, layout, targets: &[wgpu::TextureFormat]| {
            fullscreen(gpu, label, source, layout, targets)
        };
        Passes {
            occlusion: pipeline(
                "occlusion",
                (occlusion_wgsl, "fs_occlusion"),
                &occlusion_layout,
                &[OCCLUSION_FORMAT],
            ),
            filter: pipeline(
                "occlusion filter",
                (occlusion_wgsl, "fs_filter"),
                &filter_layout,
                &[OCCLUSION_FORMAT],
            ),
            diffuse: pipeline(
                "diffuse",
                (display_wgsl, "fs_diffuse"),
                &display_layout,
                &[DIFFUSE_FORMAT],
            ),
            // The frame's color through its non-sRGB view, and the history.
            display: pipeline(
                "display",
                (display_wgsl, "fs"),
                &display_layout,
                &[UI_FORMAT, HISTORY_FORMAT],
            ),
            occlusion_layout,
            filter_layout,
            display_layout,
            gather: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("part flags"),
                ..Default::default()
            }),
        }
    }
}

/// Begins a full-screen pass writing `targets`, each cleared to `clear`, with its timestamps.
pub(crate) fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    (label, clear): (&str, wgpu::Color),
    targets: &[&wgpu::TextureView],
    timestamps: Option<wgpu::RenderPassTimestampWrites<'e>>,
) -> wgpu::RenderPass<'e> {
    let attachments: Vec<_> = (targets.iter())
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: timestamps,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
