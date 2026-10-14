//! The interface layer. An interface toolkit's output (textured, colored triangles with clip
//! rectangles, plus texture changes) in plain types, so the toolkit never touches wgpu and
//! this crate never touches the toolkit, and the painter that draws it over a frame.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::device::Gpu;
use crate::error::RenderError;
use crate::frame::FrameTarget;
use crate::snapshot::Renderer;

/// One interface vertex: position in physical pixels, texture coordinate, and an sRGB-encoded,
/// premultiplied color.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

/// Triangles that share one texture and one clip rectangle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiMesh {
    /// Clip rectangle in physical pixels: min x, min y, max x, max y.
    pub clip: [f32; 4],
    /// Which texture (see [`UiTexture`]).
    pub texture: u64,
    pub vertices: Vec<UiVertex>,
    /// Three per triangle, into `vertices`.
    pub indices: Vec<u32>,
}

/// An image for the interface: sRGB-encoded, premultiplied RGBA8 rows, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Linear filtering; nearest when false.
    pub smooth: bool,
}

/// A change to the interface's textures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiTexture {
    /// Creates texture `id` (when `at` is `None`), or writes `image` into it with its top-left
    /// corner at `at`.
    Set {
        id: u64,
        at: Option<[u32; 2]>,
        image: UiImage,
    },
    /// Drops texture `id`.
    Free(u64),
}

/// Everything the interface draws in one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiLayer {
    /// Applied in order, before drawing.
    pub textures: Vec<UiTexture>,
    /// Drawn in order, later meshes on top.
    pub meshes: Vec<UiMesh>,
}

/// The target format the interface is drawn into: the frame's non-sRGB view, so blending
/// works on sRGB-encoded values.
pub(crate) const UI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Renderer {
    /// Lets the interface draw the last frame drawn into `target` as its texture `id`: a second
    /// view placed inside the interface, such as the Reference View (decision 0020). The
    /// texture is the frame itself, not a copy, so drawing into `target` again changes what the
    /// interface shows; call this again after making a new target. Pick an `id` the interface's
    /// own textures never use.
    pub fn show_frame(&self, painter: &mut UiPainter, id: u64, target: &FrameTarget) {
        painter.show(&self.gpu, id, target);
    }
}

/// Draws [`UiLayer`]s. Keeps the interface's textures between frames.
pub struct UiPainter {
    pipeline: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    image_layout: wgpu::BindGroupLayout,
    /// Nearest and linear.
    samplers: [wgpu::Sampler; 2],
    textures: BTreeMap<u64, (wgpu::Texture, wgpu::BindGroup)>,
}

impl UiPainter {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        use wgpu::ShaderStages as S;
        let device = &gpu.device;
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui screen"),
            entries: &[entry(0, S::VERTEX, uniform)],
        });
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui image"),
            entries: &[
                entry(0, S::FRAGMENT, texture),
                entry(1, S::FRAGMENT, sampler),
            ],
        });
        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM.union(wgpu::BufferUsages::COPY_DST),
            mapped_at_creation: false,
        });
        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui screen"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("ui"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            pipeline: pipeline(gpu, &[&screen_layout, &image_layout]),
            screen,
            screen_group,
            image_layout,
            samplers: [
                sampler(wgpu::FilterMode::Nearest),
                sampler(wgpu::FilterMode::Linear),
            ],
            textures: BTreeMap::new(),
        }
    }

    /// Ids of the textures it holds.
    pub fn texture_ids(&self) -> Vec<u64> {
        self.textures.keys().copied().collect()
    }

    /// Applies texture changes in order.
    pub(crate) fn update(&mut self, gpu: &Gpu, changes: &[UiTexture]) -> Result<(), RenderError> {
        for change in changes {
            match change {
                UiTexture::Set {
                    id,
                    at: None,
                    image,
                } => {
                    let texture = self.create(gpu, image)?;
                    self.textures.insert(*id, texture);
                }
                UiTexture::Set {
                    id,
                    at: Some(at),
                    image,
                } => {
                    let (texture, _) = self.textures.get(id).ok_or_else(|| {
                        RenderError::Unsupported(format!("a patch for missing texture {id}"))
                    })?;
                    write(gpu, texture, *at, image)?;
                }
                UiTexture::Free(id) => {
                    self.textures.remove(id);
                }
            }
        }
        Ok(())
    }

    /// Makes the colour texture of `target` the interface's texture `id`, replacing any texture
    /// of that id. It is the frame itself, not a copy.
    fn show(&mut self, gpu: &Gpu, id: u64, target: &FrameTarget) {
        let view = target.view(UI_FORMAT);
        let group = self.group(gpu, &view, true);
        self.textures.insert(id, (target.color.clone(), group));
    }

    /// The bind group drawing `view` with the linear sampler when `smooth`, else the nearest.
    fn group(&self, gpu: &Gpu, view: &wgpu::TextureView, smooth: bool) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui image"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.samplers[usize::from(smooth)]),
                },
            ],
        })
    }

    fn create(
        &self,
        gpu: &Gpu,
        image: &UiImage,
    ) -> Result<(wgpu::Texture, wgpu::BindGroup), RenderError> {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui image"),
            size: extent(image.width.max(1), image.height.max(1)),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: UI_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST),
            view_formats: &[],
        });
        write(gpu, &texture, [0, 0], image)?;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.group(gpu, &view, image.smooth);
        Ok((texture, group))
    }

    /// Draws `meshes` over `target` (`size` pixels). Meshes with an unknown texture are
    /// skipped; an empty clip rectangle draws nothing.
    pub(crate) fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        meshes: &[UiMesh],
    ) {
        let (vertices, indices) = pack(meshes);
        if indices.is_empty() {
            return;
        }
        let screen = [size[0] as f32, size[1] as f32, 0.0, 0.0];
        gpu.queue
            .write_buffer(&self.screen, 0, bytemuck::cast_slice(&screen));
        let buffer = |label, contents: &[u8], usage| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let vertex_buffer = buffer(
            "ui vertices",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let index_buffer = buffer(
            "ui indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.screen_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let mut first = 0u32;
        for mesh in meshes {
            let count = mesh.indices.len() as u32;
            let group = self.textures.get(&mesh.texture).map(|(_, g)| g);
            if let Some(group) = group {
                let [x, y, w, h] = scissor(mesh.clip, size);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(1, group, &[]);
                pass.draw_indexed(first..first + count, 0, 0..1);
            }
            first += count;
        }
    }
}

/// All meshes' vertices and indices in one pair of arrays, indices moved past the vertices of
/// the meshes before.
fn pack(meshes: &[UiMesh]) -> (Vec<UiVertex>, Vec<u32>) {
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for mesh in meshes {
        let base = vertices.len() as u32;
        indices.extend(mesh.indices.iter().map(|i| i + base));
        vertices.extend_from_slice(&mesh.vertices);
    }
    (vertices, indices)
}

/// The clip rectangle as a scissor rectangle inside a `size` target: x, y, width, height. An
/// empty or inverted clip gives a zero-sized scissor, which draws nothing.
fn scissor(clip: [f32; 4], size: [u32; 2]) -> [u32; 4] {
    let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
    let (x0, y0) = (
        clamp(clip[0].floor(), size[0]),
        clamp(clip[1].floor(), size[1]),
    );
    let (x1, y1) = (
        clamp(clip[2].ceil(), size[0]),
        clamp(clip[3].ceil(), size[1]),
    );
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

/// Writes `image` into `texture` at `at`, refusing writes that do not fit.
fn write(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    at: [u32; 2],
    image: &UiImage,
) -> Result<(), RenderError> {
    let fits = at[0] + image.width <= texture.width() && at[1] + image.height <= texture.height();
    let bytes = (image.width * image.height * 4) as usize;
    if !fits || image.rgba.len() != bytes {
        return Err(RenderError::Unsupported(format!(
            "a {}x{} image with {} bytes at {at:?} in a {}x{} texture",
            image.width,
            image.height,
            image.rgba.len(),
            texture.width(),
            texture.height()
        )));
    }
    if bytes == 0 {
        return Ok(());
    }
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: at[0],
                y: at[1],
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &image.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 4),
            rows_per_image: None,
        },
        extent(image.width, image.height),
    );
    Ok(())
}

fn pipeline(gpu: &Gpu, groups: &[&wgpu::BindGroupLayout]) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4];
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui.wgsl").into()),
        });
    let groups: Vec<_> = groups.iter().map(|g| Some(*g)).collect();
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });
    let premultiplied = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRIBUTES,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: UI_FORMAT,
                    blend: Some(premultiplied),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}
