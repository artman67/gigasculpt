//! The renderer: offscreen snapshots and contact sheets.

use crate::Camera;
use crate::device::{AdapterKind, Gpu};
use crate::display::Display;
use crate::error::RenderError;
use crate::frame::Scene;
use crate::grid::FloorGrid;
use crate::image::Image;
use crate::lit::{GpuEnvironment, LitTextures, Subsurface};
use crate::matcap::{Baked, MatCapImage};
use crate::mesh::{RenderMesh, UploadOptions, prepare};
use crate::occlusion::{Occlusion, quality};
use crate::passes::Passes;
use crate::pipeline::{Path, Pipelines};
use crate::pool::{CullBuffers, GpuMatCap, GpuMesh, upload_bake, upload_matcap, upload_mesh};
use crate::shading::light::Lights;
use crate::shading::{Material, Rgb};
use crate::streams::COLOR;
use crate::timing::Probe;

/// How surfaces are colored.
#[derive(Clone, Copy)]
pub enum Shading<'a> {
    /// The clay look (`light-brown-clay`) in Lit mode, under the view's light (decision 0023).
    Clay,
    /// A MatCap looked up by view-space normal: a baked look's `albedo x D + S`, or an 8-bit
    /// image, with ambient occlusion on its environment light (decision 0019). Ignores the
    /// view's light.
    MatCap(&'a GpuMatCap),
    /// Reflection stripes over Clay mode, for checking shape (ARCHITECTURE section 10.3).
    Zebra,
    /// World-space normal as color: each byte is `n * 127.5 + 127.5`.
    Normals,
    /// A material lit live by the view's light (decision 0023).
    Lit(&'a Material),
    /// Flat Color (decision 0043, step (c)): the paint times this base color (linear), unlit,
    /// shown as stored: no light, occlusion, exposure, view transform or dither.
    Flat(Rgb),
}

/// The light of Lit mode, and of Clay and zebra, which draw in it (decision 0023): an
/// environment fixed to the camera and the lights of `light.palette`, in view space.
#[derive(Clone, Copy)]
pub struct Light<'a> {
    /// `None`: the default studio.
    pub environment: Option<&'a GpuEnvironment>,
    /// Ambient (the environment's light scaled) and lights 1 to 8.
    pub lights: &'a Lights,
}

impl Default for Light<'_> {
    /// The default studio, Ambient 1, its key light on and no other.
    fn default() -> Self {
        Light {
            environment: None,
            lights: &Lights::DEFAULT,
        }
    }
}

/// The wireframe overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Off,
    /// Barycentric where the adapter supports it, vertex pulling elsewhere.
    Auto,
    /// Hardware barycentrics. `Unsupported` on adapters without them (llvmpipe).
    Barycentric,
    /// The vertex-pulling fallback; works everywhere for pools up to the storage binding limit.
    VertexPulling,
}

/// How the wireframe is drawn while it is on (catalog item `canvas.wireframe-view`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireStyle {
    /// The polygon edges.
    pub line: bool,
    /// Every part's face group colors in place of its paint, as `View::face_groups` shows them.
    pub fill: bool,
    /// The edges' color, 8-bit sRGB, drawn over the finished picture.
    pub color: [u8; 3],
    /// How much an edge covers the surface, 0 to 1.
    pub opacity: f32,
}

impl Default for WireStyle {
    /// Near-black edges covering the surface, no fill: the wire as it was before it had a
    /// style. The session's default is lighter (gs-engine's `WireStyle`).
    fn default() -> Self {
        WireStyle {
            line: true,
            fill: false,
            color: [20, 20, 26],
            opacity: 1.0,
        }
    }
}

/// The background (catalog item `canvas.background`): a color with a vertical black-to-white
/// gradient over it, white at the top, mixed in sRGB as the color is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    /// 8-bit sRGB.
    pub color: [u8; 3],
    /// How much of the gradient lies over the color: 0 none, 1 the gradient alone.
    pub gradient: f32,
    /// How far down the viewport the gradient is half white: 0 the top, 1 the bottom.
    pub center: f32,
    /// How much of the viewport's height the gradient spreads over: 0 a hard edge.
    pub softness: f32,
}

impl Default for Background {
    /// [`BACKGROUND`], flat.
    fn default() -> Self {
        Background {
            color: [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]],
            gradient: 0.0,
            center: 0.5,
            softness: 0.5,
        }
    }
}

/// What to draw and how to show it: camera, shading, wireframe, and the view's display,
/// ambient occlusion, floor grid and mask settings (research 11, 12.1; decision 0019).
#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per thing a view may draw"
)]
pub struct View<'a> {
    pub camera: Camera,
    pub shading: Shading<'a>,
    pub wire: Wire,
    /// How the wireframe is drawn while `wire` is on.
    pub wire_style: WireStyle,
    /// Color every part by the size of its polygons on screen in place of its paint: red where
    /// they are large, green, blue where they are small (catalog item `canvas.wireframe-view`);
    /// off by default.
    pub density: bool,
    /// Exposure, view transform and dither for physical light (Lit, Clay, zebra and baked
    /// MatCaps). Normals are shown as they are; display-referred MatCaps take the exposure only.
    pub display: Display,
    pub occlusion: Occlusion,
    /// The floor grid (catalog item `canvas.floor-grid`); off by default.
    pub grid: bool,
    /// How the floor grid is drawn while it is on: by default one plane just below the parts.
    pub floor: FloorGrid,
    /// How much a fully masked surface is darkened, 0 to 1: 0.75 by default (Blender's
    /// default overlay opacity), 0 while the mask is hidden (decision 0023 as amended for Q2).
    pub mask_opacity: f32,
    /// What Lit, Clay and zebra are lit by.
    pub light: Light<'a>,
    /// Live subsurface for a Lit look that scatters.
    pub subsurface: Subsurface,
    /// Outline the marked mesh, the active part (`GpuMesh::marked`, decision 0033 section 5),
    /// when two meshes or more are drawn; off by default.
    pub outline: bool,
    /// Show every mesh's face group colors in place of its paint (decision 0039, section 9);
    /// off by default.
    pub face_groups: bool,
    /// Mirror the MatCap lookup left to right (catalog item `material.matcap-shading`); off by
    /// default.
    pub matcap_flip: bool,
    /// What shows where no part does; flat [`BACKGROUND`] by default.
    pub background: Background,
}

impl<'a> View<'a> {
    /// `camera` and `shading` with no wireframe (styled by default) or density view, the default
    /// display (exposure 0, PBR Neutral, dithered), the default ambient occlusion, no grid, the
    /// mask at 0.75, the default light (the default studio with its key light), the default
    /// subsurface (on except on an integrated GPU), no outline, no face groups, the MatCap
    /// unflipped and the flat background.
    pub fn new(camera: Camera, shading: Shading<'a>) -> View<'a> {
        View {
            camera,
            shading,
            wire: Wire::Off,
            wire_style: WireStyle::default(),
            density: false,
            display: Display::default(),
            occlusion: Occlusion::default(),
            grid: false,
            floor: FloorGrid::default(),
            mask_opacity: 0.75,
            light: Light::default(),
            subsurface: Subsurface::DEFAULT,
            outline: false,
            face_groups: false,
            matcap_flip: false,
            background: Background::default(),
        }
    }
}

/// Output size in pixels, and how many jittered samples a still picture accumulates (1: one
/// unjittered sample, no anti-aliasing; decision 0019). Snapshots draw every sample; the
/// window draws one per frame while nothing changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSize {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

impl RenderSize {
    /// A square image, one sample per pixel: the golden-image setting.
    pub fn square(size: u32) -> RenderSize {
        RenderSize {
            width: size,
            height: size,
            samples: 1,
        }
    }
}

/// Most samples a picture may accumulate (Blender's Workbench offers up to 32).
pub const MAX_SAMPLES: u32 = 64;

/// The background, sRGB, unless a view sets its own ([`Background`]). Every pixel no triangle
/// covers has exactly this value.
pub const BACKGROUND: [u8; 4] = [40, 42, 46, 255];

/// A headless device with its pipelines. Shareable between threads.
pub struct Renderer {
    pub(crate) gpu: Gpu,
    pub(crate) pipelines: Pipelines,
    pub(crate) passes: Passes,
    pub(crate) sampler: wgpu::Sampler,
    /// Bound where the view's shading has no MatCap.
    pub(crate) blank: GpuMatCap,
    /// Lit mode's sampler, the DFG table and the default studio, and stand-ins.
    pub(crate) lit: LitTextures,
    /// Ambient occlusion's slices and steps per side and frame pixels per texel, by the
    /// adapter's tier.
    pub(crate) quality: (u32, u32, u32),
    /// Live Boolean's ghost pipelines (`live.rs`), made the first time a frame draws a ghost.
    pub(crate) ghosts: std::sync::OnceLock<crate::live::Ghosts>,
}

impl Renderer {
    /// Opens a device on `kind`.
    pub fn new(kind: AdapterKind) -> Result<Renderer, RenderError> {
        Ok(Renderer::with_gpu(Gpu::open(kind)?))
    }

    /// A renderer on an open device.
    pub(crate) fn with_gpu(gpu: Gpu) -> Renderer {
        let pipelines = Pipelines::new(&gpu);
        let passes = Passes::new(&gpu);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("matcap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = upload_matcap(
            &gpu,
            &MatCapImage {
                size: 1,
                pixels: vec![[u16::MAX; 4]],
            },
            None,
        );
        let quality = quality(gpu.integrated());
        let lit = LitTextures::new(&gpu);
        Renderer {
            gpu,
            pipelines,
            passes,
            sampler,
            blank,
            lit,
            quality,
            ghosts: std::sync::OnceLock::new(),
        }
    }

    /// This renderer with the full ambient occlusion of the discrete GPU and lavapipe, even on
    /// an integrated GPU: for checks that compare drivers at one quality, such as the golden
    /// references' noise across GPUs. The integrated GPU's cheaper occlusion has checks of its
    /// own.
    #[must_use]
    pub fn with_full_occlusion(mut self) -> Renderer {
        self.quality = quality(false);
        self
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// Copies a mesh to the GPU. Pass a `&PlainMesh`, or a [`RenderMesh`] with normals.
    pub fn upload<'m>(
        &self,
        mesh: impl Into<RenderMesh<'m>>,
        options: UploadOptions,
    ) -> Result<GpuMesh, RenderError> {
        let (vertices, cpu) = prepare(mesh.into(), options)?;
        let pools = (&[&vertices[..]][..], COLOR);
        Ok(upload_mesh(&self.gpu, &self.pipelines, pools, &cpu))
    }

    /// Copies an 8-bit MatCap image (as a user brings it) to the GPU: shown as the file looks.
    pub fn upload_matcap(&self, image: &MatCapImage) -> GpuMatCap {
        upload_matcap(&self.gpu, image, None)
    }

    /// Copies a two-sphere 8-bit MatCap read as flat plus crease to the GPU: `flat` everywhere,
    /// blending toward `crease` where ambient occlusion darkens (catalog item
    /// `material.cavity-shading`).
    pub fn upload_matcap_pair(&self, flat: &MatCapImage, crease: &MatCapImage) -> GpuMatCap {
        upload_matcap(&self.gpu, flat, Some(crease))
    }

    /// Copies a baked look's D and S layers to the GPU.
    pub fn upload_bake(&self, baked: &Baked) -> GpuMatCap {
        upload_bake(&self.gpu, baked)
    }

    pub(crate) fn path(&self, wire: Wire, mesh: &GpuMesh) -> Result<Path, RenderError> {
        let bary = self.gpu.capabilities().barycentric_wireframe;
        let path = match wire {
            Wire::Off => Path::Plain,
            Wire::Auto | Wire::Barycentric if bary => Path::Barycentric,
            Wire::Barycentric => {
                return Err(RenderError::Unsupported(format!(
                    "barycentric wireframe on {}",
                    self.gpu.adapter_name()
                )));
            }
            Wire::Auto | Wire::VertexPulling => Path::Pulled,
        };
        if path == Path::Pulled && mesh.pull_group.is_none() {
            return Err(RenderError::Unsupported(format!(
                "vertex-pulling wireframe for a mesh this large on {}",
                self.gpu.adapter_name()
            )));
        }
        Ok(path)
    }

    pub(crate) fn check_size(&self, size: RenderSize) -> Result<(), RenderError> {
        let max = self.gpu.device.limits().max_texture_dimension_2d;
        if size.width == 0 || size.height == 0 || size.width > max || size.height > max {
            return Err(RenderError::Unsupported(format!(
                "image size {}x{} (1 to {max})",
                size.width, size.height
            )));
        }
        if !(1..=MAX_SAMPLES).contains(&size.samples) {
            return Err(RenderError::Unsupported(format!(
                "{} samples per pixel (1 to {MAX_SAMPLES})",
                size.samples
            )));
        }
        Ok(())
    }

    /// Renders `meshes` (one mesh, or a scene's several) as seen by `view`, every one of
    /// `size.samples` jittered samples averaged, and reads the image back. The window draws the
    /// same samples into its frame one per paint (`draw_frame`), so at one sample the two are
    /// the same bytes where the window's view starts at its frame's corner (decision 0024).
    pub fn snapshot<'m>(
        &self,
        meshes: impl Into<Meshes<'m>>,
        view: &View,
        size: RenderSize,
    ) -> Result<Image, RenderError> {
        self.picture(&meshes.into().0, view, size)
    }

    fn picture(
        &self,
        meshes: &[&GpuMesh],
        view: &View,
        size: RenderSize,
    ) -> Result<Image, RenderError> {
        let target = self.frame_target(size)?;
        let scene = Scene {
            meshes,
            view: *view,
            viewport: [0, 0, size.width, size.height],
        };
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for _ in 0..size.samples {
            self.draw_scene(&mut encoder, &target, &scene, Probe::default())?;
        }
        self.gpu.queue.submit([encoder.finish()]);
        self.read_frame(&target)
    }

    /// Renders every view at `tile` size and tiles them `columns` across, left to right and
    /// top to bottom, with each view's name burned into its top-left corner. Every cell draws
    /// the same `meshes`, so a cell is the snapshot of its view at `tile` until its label.
    pub fn contact_sheet<'m>(
        &self,
        meshes: impl Into<Meshes<'m>>,
        views: &[(&str, View)],
        tile: RenderSize,
        columns: u32,
    ) -> Result<Image, RenderError> {
        let meshes = meshes.into();
        let shots = (views.iter())
            .map(|(name, view)| Ok((*name, self.picture(&meshes.0, view, tile)?)))
            .collect::<Result<Vec<_>, RenderError>>()?;
        let tiles: Vec<(&str, &Image)> = shots.iter().map(|(n, s)| (*n, s)).collect();
        Ok(Image::sheet(&tiles, columns))
    }
}

/// What a snapshot or a contact sheet draws: one mesh, or a scene's several, as a [`Scene`]
/// holds them.
pub struct Meshes<'m>(Vec<&'m GpuMesh>);

impl<'m> From<&'m GpuMesh> for Meshes<'m> {
    fn from(mesh: &'m GpuMesh) -> Self {
        Self(vec![mesh])
    }
}

impl<'m> From<&[&'m GpuMesh]> for Meshes<'m> {
    fn from(meshes: &[&'m GpuMesh]) -> Self {
        Self(meshes.to_vec())
    }
}

/// One multi-draw: of the chunks the culling pass wrote into `culled` (see `GpuMesh::culls`),
/// otherwise of every chunk.
pub(crate) fn draw(
    pass: &mut wgpu::RenderPass,
    mesh: &GpuMesh,
    path: Path,
    culled: Option<&CullBuffers>,
) {
    let chunks = mesh.chunks().len() as u32;
    if chunks == 0 {
        // wgpu's indirect validation rejects a draw from a buffer shorter than one record.
        return;
    }
    pass.set_bind_group(3, &mesh.deform.group, &[]);
    if let (Path::Pulled, Some(group)) = (path, &mesh.pull_group) {
        pass.set_bind_group(1, group, &[]);
        pass.multi_draw_indirect(&mesh.pulled_draws, 0, chunks);
        return;
    }
    let group = match path {
        Path::Barycentric => &mesh.bary_group,
        _ => &mesh.mask_group,
    };
    pass.set_bind_group(1, group, &[]);
    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
    match culled {
        Some(cull) => {
            pass.multi_draw_indexed_indirect_count(&cull.draws, 0, &cull.drawn, 0, chunks);
        }
        None => pass.multi_draw_indexed_indirect(&mesh.indexed_draws, 0, chunks),
    }
}
