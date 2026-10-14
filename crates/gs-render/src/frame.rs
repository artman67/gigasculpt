//! Frames (decision 0019): the textures a scene, and the interface over it, are drawn into, and
//! the passes that draw it, in order: cull, the geometry pass (reversed depth and the surface
//! target), ambient occlusion and its filter, live subsurface's diffuse pass while it runs
//! (decision 0023), and the display pass, which lights each pixel and shows it (view
//! transform, floor grid, accumulation, sRGB encode). Snapshots and the window share this
//! path; the window then copies the frame to its surface (see `window.rs`), and a screenshot
//! reads the frame back, so it shows exactly what was presented.
//!
//! Each mesh is drawn as its part's drawing state says (`part.rs`): at its place, with its
//! flags, its paint from what the view and the part choose.

use std::sync::{Mutex, OnceLock, PoisonError};

use crate::accumulate::{Accumulation, Key, Step};
use crate::error::RenderError;
use crate::image::Image;
use crate::lit::{LitFrame, LitUniforms, diffuse_texture};
use crate::math::mul4;
use crate::part::Drawn;
use crate::passes::begin;
use crate::pipeline::{
    COLOR_FORMAT, DEPTH_FORMAT, HISTORY_FORMAT, OCCLUSION_FORMAT, PAINT_FORMAT, PARTS_FORMAT, Path,
    SURFACE_FORMAT, bind_group,
};
use crate::pool::{CullBuffers, GpuMesh};
use crate::snapshot::{RenderSize, Renderer, Shading, View, draw};
use crate::timing::{Probe, Timing};
use crate::ui::{UI_FORMAT, UiLayer, UiPainter};
use crate::uniforms::{FrameUniforms, Sample};
use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

/// The model part of a frame.
#[derive(Clone, Copy)]
pub struct Scene<'a> {
    /// Every part to draw, with one camera.
    pub meshes: &'a [&'a GpuMesh],
    pub view: View<'a>,
    /// Where in the frame the model is drawn: x, y, width, height in pixels. The rest of the
    /// frame keeps the background color. Clipped to the frame.
    pub viewport: [u32; 4],
}

/// The textures one frame is drawn into (decision 0019): reversed depth, the surface target,
/// the part flags and the paint from the geometry pass, raw and filtered occlusion, two
/// histories for the accumulated mean, the sRGB color texture the display pass writes (also
/// readable through a non-sRGB view, for the interface), the last sample's picture, and live
/// subsurface's diffuse irradiance, made the first time it runs. It remembers what its last
/// frame showed, so while nothing changes each frame adds a jittered sample, up to
/// `size.samples`, and then shows that picture again.
pub struct FrameTarget {
    size: RenderSize,
    pub(crate) color: wgpu::Texture,
    /// The last sample's color, before ghosts and the interface: a settled frame copies it
    /// back, so it repeats that frame byte for byte (decision 0019, amended).
    shown: wgpu::Texture,
    pub(crate) depth: wgpu::TextureView,
    pub(crate) surface: wgpu::TextureView,
    /// Each pixel's part flags (decision 0033): 0 where nothing is drawn.
    pub(crate) parts: wgpu::TextureView,
    /// Each pixel's paint (decision 0039, section 9): white where nothing is painted or drawn.
    /// A frame where no part shows paint leaves it alone and reads a white texel instead.
    pub(crate) paint: wgpu::TextureView,
    raw_occlusion: wgpu::TextureView,
    occlusion: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    diffuse: OnceLock<wgpu::TextureView>,
    /// Live Boolean's ghost cover (`live.rs`), made the first time a frame draws a ghost.
    pub(crate) ghost: OnceLock<wgpu::TextureView>,
    /// Live Boolean's peel targets (`peel.rs`), held while frames peel.
    pub(crate) peel: Mutex<Option<crate::peel::Targets>>,
    accumulation: Mutex<Accumulation>,
    /// The one pixel the next frame keeps for the window's color pick ([`FrameTarget::pick`]).
    pick: Mutex<Pick>,
}

/// A pixel a frame is asked to keep: where, and whether before the interface goes over it; then
/// the buffer the frame copied it into, until it is read.
#[derive(Default)]
struct Pick {
    asked: Option<([u32; 2], bool)>,
    copied: Option<wgpu::Buffer>,
}

impl FrameTarget {
    /// Asks the next frame drawn into this target ([`Renderer::draw_frame`]) to keep its pixel at
    /// `at`, in pixels from the top-left: as the scene drew it, before the interface, when
    /// `scene`, else as shown. [`Renderer::picked`] reads it. A frame not asked copies nothing.
    pub fn pick(&self, at: [u32; 2], scene: bool) {
        let mut pick = self.pick.lock().unwrap_or_else(PoisonError::into_inner);
        pick.asked = Some((at, scene));
    }

    /// Size, and how many samples a still picture converges to.
    pub fn size(&self) -> RenderSize {
        self.size
    }

    /// Whether the last frame drawn holds every sample of its scene, so drawing the same scene
    /// again changes nothing. The window keeps drawing until it has.
    pub fn settled(&self) -> bool {
        let acc = self
            .accumulation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        acc.settled(self.size.samples)
    }

    /// Forgets what the last frame showed: the next frame starts again from one unjittered
    /// sample, as after any change.
    pub fn restart(&self) {
        let mut acc = self
            .accumulation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *acc = Accumulation::default();
    }

    /// The color texture seen as `format`: [`COLOR_FORMAT`] or its non-sRGB twin.
    pub(crate) fn view(&self, format: wgpu::TextureFormat) -> wgpu::TextureView {
        self.color.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        })
    }
}

impl Renderer {
    /// Textures for frames of `size`.
    pub fn frame_target(&self, size: RenderSize) -> Result<FrameTarget, RenderError> {
        use wgpu::TextureUsages as T;
        self.check_size(size)?;
        let texture = |label, format, usage, view_formats, scale: u32| {
            self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.width.div_ceil(scale),
                    height: size.height.div_ceil(scale),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let pass = T::RENDER_ATTACHMENT.union(T::TEXTURE_BINDING);
        let view = |label, format, scale| {
            let t = texture(label, format, pass, &[], scale);
            t.create_view(&wgpu::TextureViewDescriptor::default())
        };
        // Occlusion is worked out once per `scale` by `scale` frame pixels.
        let scale = self.quality.2;
        let copies = T::COPY_SRC.union(T::COPY_DST);
        let color_usage = (T::RENDER_ATTACHMENT.union(T::TEXTURE_BINDING)).union(copies);
        Ok(FrameTarget {
            size,
            color: texture("frame", COLOR_FORMAT, color_usage, &[UI_FORMAT], 1),
            shown: texture("frame shown", COLOR_FORMAT, copies, &[], 1),
            depth: view("frame depth", DEPTH_FORMAT, 1),
            surface: view("frame surface", SURFACE_FORMAT, 1),
            parts: view("frame parts", PARTS_FORMAT, 1),
            paint: view("frame paint", PAINT_FORMAT, 1),
            raw_occlusion: view("frame raw occlusion", OCCLUSION_FORMAT, scale),
            occlusion: view("frame occlusion", OCCLUSION_FORMAT, scale),
            history: [
                view("frame history", HISTORY_FORMAT, 1),
                view("frame history", HISTORY_FORMAT, 1),
            ],
            diffuse: OnceLock::new(),
            ghost: OnceLock::new(),
            peel: Mutex::new(None),
            accumulation: Mutex::new(Accumulation::default()),
            pick: Mutex::default(),
        })
    }

    /// A painter for the interface layer on this device.
    pub fn ui_painter(&self) -> UiPainter {
        UiPainter::new(&self.gpu)
    }

    /// Encodes the next frame of `scene` into `target`: the next jittered sample averaged into
    /// the last frames while the scene is unchanged, a fresh unjittered one after any change,
    /// or, once every sample is in, the last sample's picture again.
    pub(crate) fn draw_scene(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &FrameTarget,
        scene: &Scene,
        probe: Probe,
    ) -> Result<(), RenderError> {
        // Live Boolean's ghosts are drawn after the display, as outlines (`live.rs`), and its
        // peeled groups after the geometry pass (`peel.rs`).
        let (meshes, ghosts, groups) = self.split(scene.meshes);
        // Each mesh's path and part flags.
        let drawn = (meshes.iter())
            .map(|mesh| self.drawn(&scene.view, mesh))
            .collect::<Result<Vec<_>, _>>()?;
        // The paint target, when a part shows paint or the peel draws (it writes paint always).
        let paint = paints(&drawn) || !groups.is_empty();
        let viewport = clip_viewport(scene.viewport, target.size);
        let all = || meshes.iter().chain(crate::peel::members(&groups));
        let points: Vec<[f32; 3]> = (all().chain(&ghosts))
            .flat_map(|m| m.fit_points())
            .collect();
        let stamps: Vec<_> = (meshes.iter().zip(&drawn))
            .map(|(m, &(_, flags))| (m.stamp(), m.sides(), m.placement.transform, flags))
            .chain(crate::peel::stamps(&groups))
            .collect();
        // The active part's outline: only where it tells two drawn parts apart.
        let outline = scene.view.outline && all().count() > 1 && all().any(|m| m.placement.marked);
        let lit = self.lit_frame(&scene.view)?;
        let lit_key = (lit.as_ref()).map(|l| (l.environment.id, l.uniforms.words()));
        let key = Key::of(&scene.view, (viewport, lit_key), &stamps);
        let (step, (read, write)) = {
            let mut acc = (target.accumulation.lock()).unwrap_or_else(PoisonError::into_inner);
            let step = acc.next(key, target.size.samples);
            (step, acc.histories(step))
        };
        let (sample, weight) = match step {
            Step::Sample(k) => (k, 1.0 / (k + 1) as f32),
            Step::Hold => (0, 0.0),
        };
        let uniforms = FrameUniforms::new(&Sample {
            view: &scene.view,
            viewport,
            points: &points,
            index: sample,
            weight,
            quality: self.quality,
            outline,
        });
        let buffer = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("frame"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        if let Some(write) = write {
            let frame = (target, scene, viewport, paint);
            let parts = (&meshes[..], &drawn[..], &groups[..]);
            self.sample_passes(encoder, frame, parts, (&uniforms, &buffer), probe);
            let paint = if paint {
                &target.paint
            } else {
                &self.blank.diffuse
            };
            let frame = (target, &scene.view, lit.as_ref(), paint);
            let group = self.lit_passes(encoder, frame, (viewport, read), &buffer, probe.timing);
            self.display_pass(encoder, target, write, &group, probe.timing);
            // The last sample keeps its picture for the settled frames after it.
            if step == Step::Sample(target.size.samples - 1) {
                copy_texture(encoder, &target.color, &target.shown);
            }
        } else {
            // Settled: the last sample's picture again, copied, not drawn again. Shown by another
            // shader it could differ by a level here and there on some drivers (decision 0019).
            copy_texture(encoder, &target.shown, &target.color);
        }
        let ghosted = (target, viewport);
        self.ghost_pass(encoder, ghosted, &ghosts, &buffer, scene.view.outline);
        // Overlays drawn as geometry (the pivot dot, the gizmo) go here, after the display
        // and before the interface, depth-tested against the reversed depth (decision 0019).
        Ok(())
    }

    /// A new sample's passes before the display: cull, the geometry pass, Live Boolean's peel
    /// and ambient occlusion.
    fn sample_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (target, scene, viewport, paint): (&FrameTarget, &Scene, [u32; 4], bool),
        (meshes, drawn, groups): (&[&GpuMesh], &[Drawn], &[crate::peel::Group]),
        (uniforms, buffer): (&FrameUniforms, &wgpu::Buffer),
        probe: Probe,
    ) {
        // A frame that pulls any part's vertices culls nothing: on lavapipe a vertex-pulled
        // draw recorded after a culled one (its count written by the GPU) draws short.
        let pulls = drawn.iter().any(|&(path, _)| path == Path::Pulled);
        let culled: Vec<_> = (meshes.iter().zip(drawn))
            .map(|(mesh, &(path, _))| mesh.culls(path).filter(|_| !pulls))
            .collect();
        let view_proj = mul4(&uniforms.proj, &uniforms.view);
        // Culling goes first: a compute pass cannot run inside the render pass.
        for (mesh, buffers) in meshes.iter().zip(&culled) {
            if let Some(buffers) = buffers {
                let (camera, view_proj) = mesh.part_view(&scene.view.camera, &view_proj);
                let camera = (&camera, &view_proj);
                self.encode_cull(encoder, mesh, buffers, camera, probe.timing);
            }
        }
        let drawn = (meshes, drawn, &culled[..]);
        let into = (target, scene.viewport, paint);
        self.geometry_pass(encoder, into, drawn, buffer, probe);
        self.peel_passes(encoder, (target, viewport), groups, buffer, probe.timing);
        if uniforms.occludes() {
            self.occlusion_passes(encoder, (target, viewport), buffer, probe.timing);
        }
    }

    /// The geometry pass: every part once, at its place, into reversed depth, the surface
    /// target, the part flags and, with `paint`, the paint.
    fn geometry_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (target, viewport, paint): (&FrameTarget, [u32; 4], bool),
        (meshes, drawn, culled): (&[&GpuMesh], &[Drawn], &[Option<&CullBuffers>]),
        uniforms: &wgpu::Buffer,
        probe: Probe,
    ) {
        let group = bind_group(
            &self.gpu,
            &self.pipelines.frame,
            &[uniforms.as_entire_binding()],
        );
        let (parts, stride) = self.parts(meshes, drawn);
        let [x, y, w, h] = clip_viewport(viewport, target.size);
        let cleared = [
            Some((&target.surface, wgpu::Color::TRANSPARENT)),
            Some((&target.parts, wgpu::Color::TRANSPARENT)),
            paint.then_some((&target.paint, wgpu::Color::WHITE)),
        ];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("geometry"),
            color_attachments: &cleared.map(|c| {
                c.map(|(view, clear)| wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })
            }),
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: probe.timing.and_then(|t| t.render("geometry")),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // A zero-sized viewport is allowed, and draws nothing.
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_scissor_rect(x, y, w, h);
        pass.set_bind_group(0, &group, &[]);
        if let Some(set) = probe.stats {
            pass.begin_pipeline_statistics_query(set, 0);
        }
        for (i, ((mesh, &(path, _)), &culled)) in meshes.iter().zip(drawn).zip(culled).enumerate() {
            let key = (path, mesh.sides(), paint);
            pass.set_pipeline(&self.pipelines.get(&self.gpu, key));
            pass.set_bind_group(2, &parts, &[(i as u64 * stride) as u32]);
            draw(&mut pass, mesh, path, culled);
        }
        if probe.stats.is_some() {
            pass.end_pipeline_statistics_query();
        }
    }

    /// Ambient occlusion from depth and normals, then its depth-aware filter.
    fn occlusion_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (target, viewport): (&FrameTarget, [u32; 4]),
        uniforms: &wgpu::Buffer,
        timing: Option<&Timing>,
    ) {
        let reads = [
            uniforms.as_entire_binding(),
            wgpu::BindingResource::TextureView(&target.depth),
            wgpu::BindingResource::TextureView(&target.surface),
        ];
        let group = bind_group(&self.gpu, &self.passes.occlusion_layout, &reads);
        let timestamps = timing.and_then(|t| t.render("occlusion"));
        // Texels where nothing is drawn: fully visible, and no depth (`NOTHING` in the shader).
        let nothing = wgpu::Color {
            r: 1.0,
            g: 1e30,
            b: 0.0,
            a: 0.0,
        };
        let s = self.quality.2;
        let [x, y, w, h] = viewport;
        let texels = [
            x / s,
            y / s,
            (x + w).div_ceil(s) - x / s,
            (y + h).div_ceil(s) - y / s,
        ];
        let label = ("occlusion", nothing);
        let mut pass = begin(encoder, label, &[&target.raw_occlusion], timestamps);
        scissor(&mut pass, texels);
        pass.set_pipeline(&self.passes.occlusion);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        let mut reads = reads.to_vec();
        reads.push(wgpu::BindingResource::TextureView(&target.raw_occlusion));
        let group = bind_group(&self.gpu, &self.passes.filter_layout, &reads);
        let timestamps = timing.and_then(|t| t.render("occlusion filter"));
        let label = ("occlusion filter", nothing);
        let mut pass = begin(encoder, label, &[&target.occlusion], timestamps);
        scissor(&mut pass, texels);
        pass.set_pipeline(&self.passes.filter);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The display pass's bindings, after live subsurface's diffuse pass when the frame's look
    /// scatters.
    fn lit_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: (&FrameTarget, &View, Option<&LitFrame>, &wgpu::TextureView),
        (viewport, read): ([u32; 4], usize),
        uniforms: &wgpu::Buffer,
        timing: Option<&Timing>,
    ) -> wgpu::BindGroup {
        let (target, _, lit, _) = frame;
        let lit_uniforms = lit.map_or_else(LitUniforms::zeroed, |l| l.uniforms);
        let lit_buffer = (self.gpu.device).create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit"),
            contents: bytemuck::bytes_of(&lit_uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let buffers = (uniforms, &lit_buffer);
        let blank = &self.lit.blank_diffuse;
        if !lit.is_some_and(LitFrame::scatters) {
            return self.display_group(frame, read, buffers, blank);
        }
        let size = [target.size.width, target.size.height];
        let diffuse = (target.diffuse).get_or_init(|| diffuse_texture(&self.gpu, size));
        let group = self.display_group(frame, read, buffers, blank);
        let timestamps = timing.and_then(|t| t.render("diffuse"));
        let label = ("diffuse", wgpu::Color::TRANSPARENT);
        let mut pass = begin(encoder, label, &[diffuse], timestamps);
        scissor(&mut pass, viewport);
        pass.set_pipeline(&self.passes.diffuse);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        self.display_group(frame, read, buffers, diffuse)
    }

    /// The display pass's bindings: the frame's textures, the view's MatCap or stand-ins, Lit
    /// mode's (or stand-ins), history `read`, `diffuse` at binding 13, the part flags at 14
    /// and `paint` at 16.
    fn display_group(
        &self,
        (target, view, lit, paint): (&FrameTarget, &View, Option<&LitFrame>, &wgpu::TextureView),
        read: usize,
        (uniforms, lit_buffer): (&wgpu::Buffer, &wgpu::Buffer),
        diffuse: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let matcap = match view.shading {
            Shading::MatCap(m) => m,
            _ => &self.blank,
        };
        let (cube, dfg) = lit.map_or((&self.lit.blank_cube, &self.lit.blank_dfg), |l| {
            (&l.environment.cube, l.dfg)
        });
        let texture = wgpu::BindingResource::TextureView;
        bind_group(
            &self.gpu,
            &self.passes.display_layout,
            &[
                uniforms.as_entire_binding(),
                texture(&target.depth),
                texture(&target.surface),
                texture(&target.occlusion),
                texture(&matcap.diffuse),
                wgpu::BindingResource::Sampler(&self.sampler),
                texture(&matcap.specular),
                texture(matcap.crease.as_ref().unwrap_or(&self.blank.diffuse)),
                texture(&target.history[read]),
                lit_buffer.as_entire_binding(),
                texture(cube),
                wgpu::BindingResource::Sampler(&self.lit.sampler),
                texture(dfg),
                texture(diffuse),
                texture(&target.parts),
                wgpu::BindingResource::Sampler(&self.passes.gather),
                texture(paint),
            ],
        )
    }

    /// Lighting and display: each pixel's light from the surface target, the occlusion and the
    /// view's shading, shown over the background with the grid, averaged into the history (read
    /// from the history `group` holds, written to history `write`), sRGB with dither.
    fn display_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &FrameTarget,
        write: usize,
        group: &wgpu::BindGroup,
        timing: Option<&Timing>,
    ) {
        let color = target.view(UI_FORMAT);
        let timestamps = timing.and_then(|t| t.render("display"));
        let label = ("display", wgpu::Color::BLACK);
        let targets = [&color, &target.history[write]];
        let mut pass = begin(encoder, label, &targets, timestamps);
        pass.set_pipeline(&self.passes.display);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draws the next frame of the scene, then the interface over it, and submits the work.
    pub fn draw_frame(
        &self,
        target: &FrameTarget,
        scene: &Scene,
        painter: &mut UiPainter,
        ui: &UiLayer,
    ) -> Result<(), RenderError> {
        painter.update(&self.gpu, &ui.textures)?;
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.draw_scene(&mut encoder, target, scene, Probe::default())?;
        self.copy_pick(&mut encoder, target, true);
        let size = [target.size.width, target.size.height];
        painter.draw(
            &self.gpu,
            &mut encoder,
            &target.view(UI_FORMAT),
            size,
            &ui.meshes,
        );
        self.copy_pick(&mut encoder, target, false);
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Copies the pixel `target` was asked to keep into a small buffer, when it was asked at
    /// this point of the frame: before the interface when `scene`, else after it.
    fn copy_pick(&self, encoder: &mut wgpu::CommandEncoder, target: &FrameTarget, scene: bool) {
        let mut pick = target.pick.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(([x, y], _)) = pick.asked.filter(|a| a.1 == scene) else {
            return;
        };
        pick.asked = None;
        if x >= target.size.width || y >= target.size.height {
            return;
        }
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picked pixel"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST.union(wgpu::BufferUsages::MAP_READ),
            mapped_at_creation: false,
        });
        let origin = wgpu::Origin3d { x, y, z: 0 };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                origin,
                ..target.color.as_image_copy()
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout::default(),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        pick.copied = Some(buffer);
    }

    /// The pixel the last frame drawn into `target` kept ([`FrameTarget::pick`]), as `[r, g, b,
    /// a]` in sRGB, waiting for the GPU to finish it; `None` when no frame has kept one since
    /// the last call.
    pub fn picked(&self, target: &FrameTarget) -> Result<Option<[u8; 4]>, RenderError> {
        let copied = (target.pick.lock().unwrap_or_else(PoisonError::into_inner))
            .copied
            .take();
        let Some(buffer) = copied else {
            return Ok(None);
        };
        let bytes = self.read_mapped(&buffer)?;
        Ok(bytes.get(..4).and_then(|b| b.try_into().ok()))
    }

    /// Draws the next frame of the scene into `target`, as [`Renderer::draw_frame`] does with
    /// no interface, and returns the GPU time of each pass in milliseconds, in frame order;
    /// empty on a device without timestamp queries. Call [`FrameTarget::restart`] first to time
    /// a moving view's frame.
    pub fn time_frame(
        &self,
        target: &FrameTarget,
        scene: &Scene,
    ) -> Result<Vec<(&'static str, f64)>, RenderError> {
        use wgpu::BufferUsages as B;
        let Some(timing) = Timing::new(&self.gpu) else {
            return Ok(Vec::new());
        };
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let probe = Probe {
            stats: None,
            timing: Some(&timing),
        };
        self.draw_scene(&mut encoder, target, scene, probe)?;
        let buffer = |usage| {
            self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass times"),
                size: 8 * 64,
                usage,
                mapped_at_creation: false,
            })
        };
        let resolved = buffer(B::QUERY_RESOLVE.union(B::COPY_SRC));
        let readback = buffer(B::COPY_DST.union(B::MAP_READ));
        let count = timing.resolve(&mut encoder, &resolved);
        encoder.copy_buffer_to_buffer(&resolved, 0, &readback, 0, 8 * 64);
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = self.read_mapped(&readback)?;
        let ticks: Vec<u64> = (bytes.chunks(8).take(count as usize))
            .map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()))
            .collect();
        Ok(timing.times(&ticks, self.gpu.queue.get_timestamp_period()))
    }

    /// Reads the frame back: what the window last presented, or the snapshot.
    pub fn read_frame(&self, target: &FrameTarget) -> Result<Image, RenderError> {
        let size = target.size;
        let padded_row = (size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST.union(wgpu::BufferUsages::MAP_READ),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            target.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);
        let mapped = self.read_mapped(&readback)?;
        let row = (size.width * 4) as usize;
        let rgba = mapped
            .chunks(padded_row as usize)
            .flat_map(|r| &r[..row])
            .copied()
            .collect();
        Ok(Image {
            width: size.width,
            height: size.height,
            rgba,
        })
    }

    /// Waits for the GPU and reads a `MAP_READ` buffer's contents.
    pub(crate) fn read_mapped(&self, readback: &wgpu::Buffer) -> Result<Vec<u8>, RenderError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            // The receiver outlives the poll below; a failed send only means it gave up.
            sender.send(r).unwrap_or_default();
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        receiver
            .recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        Ok(mapped.to_vec())
    }
}

/// Copies the whole of texture `from` into `to`, of the same size and format.
fn copy_texture(encoder: &mut wgpu::CommandEncoder, from: &wgpu::Texture, to: &wgpu::Texture) {
    encoder.copy_texture_to_texture(from.as_image_copy(), to.as_image_copy(), from.size());
}

/// Limits a full-screen pass to the scene's viewport; the rest keeps its clear value.
fn scissor(pass: &mut wgpu::RenderPass, [x, y, w, h]: [u32; 4]) {
    pass.set_scissor_rect(x, y, w, h);
}

/// The viewport clipped to the frame.
fn clip_viewport(viewport: [u32; 4], size: RenderSize) -> [u32; 4] {
    let x = viewport[0].min(size.width);
    let y = viewport[1].min(size.height);
    let w = viewport[2].min(size.width - x);
    let h = viewport[3].min(size.height - y);
    [x, y, w, h]
}

/// Whether any drawn part shows point colors or face groups, or is tinted (bits 1 to 3 of its
/// flags, see [`Renderer::drawn`]). In a frame where none does, every pixel's paint is white,
/// so the geometry pass leaves the paint target out and the display reads one white texel
/// instead.
fn paints(drawn: &[Drawn]) -> bool {
    drawn.iter().any(|&(_, flags)| flags >> 1 != 0)
}
