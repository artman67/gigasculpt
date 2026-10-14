//! The window surface: a device that can present to a window, and the copy of each finished
//! frame onto the window's surface texture.
//!
//! Tuned for pen latency (research report 05, section 4.3): Mailbox if the surface offers it,
//! else no vsync, else Fifo, and at most one frame queued.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::device::{AdapterKind, Gpu, adapters};
use crate::error::RenderError;
use crate::frame::FrameTarget;
use crate::snapshot::Renderer;

/// Anything a surface can be made for: display and window handles that may be held on any
/// thread (gs-platform's `SurfaceHandle` answers only on the window's own thread).
pub trait SurfaceSource: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static {}

impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static> SurfaceSource for T {}

/// How the window presents, for status displays and `ui.describe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceInfo {
    /// The adapter, for example `NVIDIA GeForce RTX 5090`.
    pub adapter: String,
    /// The surface texture format, for example `Bgra8UnormSrgb`.
    pub format: String,
    /// `Mailbox`, `Immediate` or `Fifo`.
    pub present_mode: String,
    /// Frames the swapchain may queue ahead.
    pub frame_latency: u32,
    /// The surface's size in pixels.
    pub size: [u32; 2],
}

/// A window's surface, configured for the renderer's device.
pub struct WindowSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blit: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    adapter: String,
}

/// The present mode with the least latency the surface offers.
fn pick_present_mode(offered: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    use wgpu::PresentMode as P;
    [P::Mailbox, P::Immediate]
        .into_iter()
        .find(|m| offered.contains(m))
        .unwrap_or(P::Fifo)
}

impl Renderer {
    /// Opens a device that can present to `source` (on `kind` if given, else the best adapter
    /// that can present) and configures the surface at `width` x `height` pixels.
    pub fn for_window(
        source: impl SurfaceSource,
        kind: Option<AdapterKind>,
        width: u32,
        height: u32,
    ) -> Result<(Renderer, WindowSurface), RenderError> {
        let (instance, adapters) = adapters();
        let surface = instance
            .create_surface(source)
            .map_err(|e| RenderError::Device(format!("window surface: {e}")))?;
        let adapter = match kind {
            // The fastest adapter that can present: the discrete GPU where there is one.
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            }))
            .ok(),
            Some(kind) => adapters
                .into_iter()
                .find(|a| kind.matches(&a.get_info()) && a.is_surface_supported(&surface)),
        };
        let adapter = &adapter.ok_or_else(|| {
            let wanted = kind.map_or("an adapter", AdapterKind::name);
            RenderError::NoAdapter(format!("{wanted} that can present to this window"))
        })?;
        let caps = surface.get_capabilities(adapter);
        let mut config = surface
            .get_default_config(adapter, width.max(1), height.max(1))
            .ok_or_else(|| RenderError::Unsupported("a surface with no formats".into()))?;
        // Whatever format the surface prefers: the copy onto it is pixel-exact either way.
        config.present_mode = pick_present_mode(&caps.present_modes);
        config.desired_maximum_frame_latency = 1;
        if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        }
        let renderer = Renderer::with_gpu(Gpu::on(adapter)?);
        surface.configure(&renderer.gpu.device, &config);
        let (blit, layout) = blit_pipeline(&renderer.gpu, config.format);
        let window = WindowSurface {
            surface,
            config,
            blit,
            layout,
            adapter: adapter.get_info().name,
        };
        Ok((renderer, window))
    }
}

impl WindowSurface {
    /// How it presents.
    pub fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            adapter: self.adapter.clone(),
            format: format!("{:?}", self.config.format),
            present_mode: format!("{:?}", self.config.present_mode),
            frame_latency: self.config.desired_maximum_frame_latency,
            size: [self.config.width, self.config.height],
        }
    }

    /// Follows the window to a new size in pixels.
    pub fn resize(&mut self, renderer: &Renderer, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&renderer.gpu.device, &self.config);
    }

    /// Copies `frame` onto the next surface texture and presents it. Returns false when the
    /// surface skipped this frame (hidden window, timeout, a size change); it is then set up
    /// again for the next one.
    pub fn present(&mut self, renderer: &Renderer, frame: &FrameTarget) -> bool {
        let (wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture)) = self.surface.get_current_texture()
        else {
            // Outdated, lost, hidden or late: set the surface up again and skip this frame.
            self.surface.configure(&renderer.gpu.device, &self.config);
            return false;
        };
        let gpu = &renderer.gpu;
        // Read the frame with the surface's own sRGB-ness, so the bytes pass through unchanged.
        let source = frame.view(if self.config.format.is_srgb() {
            crate::pipeline::COLOR_FORMAT
        } else {
            crate::ui::UI_FORMAT
        });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source),
            }],
        });
        let target = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(texture);
        true
    }
}

fn blit_pipeline(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blit"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}
