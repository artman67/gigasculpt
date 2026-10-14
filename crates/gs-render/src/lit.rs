//! Lit mode on the GPU (decision 0023): an environment's textures (the prefiltered cube of
//! everything but light 1), the DFG table as a texture, the uniforms the display pass's Lit
//! branch (`shaders/lit.wgsl`) reads, and live subsurface's settings. `shading::lit` computes
//! the same light on the CPU and is what the branch is checked against.

use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};

use crate::device::Gpu;
use crate::error::RenderError;
use crate::matcap::Library;
use crate::pool::next_id;
use crate::shading::burley;
use crate::shading::cube::{Cube, Face};
use crate::shading::dfg::{self, Dfg};
use crate::shading::half_rgba;
use crate::shading::light::{DiscLight, Lights};
use crate::shading::lit::LitEnvironment;
use crate::shading::{Brdf, Material};
use crate::snapshot::{Renderer, Shading, View};

/// The look Clay and Zebra modes draw in Lit mode: tan modeling clay.
pub const CLAY_LOOK: &str = "light-brown-clay";
/// The environment Lit mode lights with when the view names none: the default studio.
pub const DEFAULT_ENVIRONMENT: &str = "studio";
/// Most lights drawn at once (`light.palette`'s eight).
const MAX_LIGHTS: usize = 8;
/// Live subsurface's samples per pixel and frame (HDRP's count, Golubev 2018).
const SCATTER_SAMPLES: usize = 21;
/// The diffuse irradiance target live subsurface gathers.
pub(crate) const DIFFUSE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// When live subsurface runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SubsurfaceMode {
    /// On, except on an integrated GPU (tier B), where it costs about as much as the rest of
    /// the frame.
    #[default]
    Auto,
    On,
    Off,
}

/// Live subsurface (decision 0023, `material.wax-preview`): in Lit mode, a look that scatters
/// has its diffuse light spread under its surface by Burley's profile, as its MatCap bake does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subsurface {
    pub mode: SubsurfaceMode,
    /// Multiplies the look's subsurface weight (0: the look without scattering).
    pub strength: f32,
    /// Multiplies the look's mean free paths.
    pub radius: f32,
    /// The length of one scene unit in millimeters, the unit of mean free paths. 50 makes the
    /// default sphere (radius 1) the 100 mm ball the MatCap bakes picture.
    pub unit_mm: f32,
}

impl SubsurfaceMode {
    /// Whether live subsurface runs in this mode on a device that is, or is not, an integrated
    /// GPU (decision 0023). `Auto` is off on an integrated GPU (tier B), where it adds about
    /// 9 ms to a moving frame, and on everywhere else: on a discrete GPU, and on lavapipe,
    /// which draws at tier A's quality as it does for occlusion (decision 0019).
    pub fn runs(self, integrated: bool) -> bool {
        match self {
            SubsurfaceMode::On => true,
            SubsurfaceMode::Off => false,
            SubsurfaceMode::Auto => !integrated,
        }
    }
}

impl Subsurface {
    pub const DEFAULT: Subsurface = Subsurface {
        mode: SubsurfaceMode::Auto,
        strength: 1.0,
        radius: 1.0,
        unit_mm: 50.0,
    };
}

impl Default for Subsurface {
    fn default() -> Self {
        Subsurface::DEFAULT
    }
}

/// An environment on the GPU, ready for Lit mode: everything but light 1 as a GGX-prefiltered
/// cube and as its SH9 fit, and light 1 as the extraction found it.
pub struct GpuEnvironment {
    pub(crate) cube: wgpu::TextureView,
    /// The SH9 fit as polynomial coefficients (`Sh9::polynomial`).
    sh: [[f32; 3]; 9],
    key: Option<DiscLight>,
    name: String,
    pub(crate) id: u64,
}

impl GpuEnvironment {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Light 1 as the environment gives it, before any setting; `None` when nothing in it is
    /// bright enough to be a light.
    pub fn key(&self) -> Option<DiscLight> {
        self.key
    }
}

/// Clay mode's material: the shipped look [`CLAY_LOOK`].
pub fn clay_material() -> &'static Material {
    static CLAY: OnceLock<Material> = OnceLock::new();
    CLAY.get_or_init(|| {
        (Library::shipped().get(CLAY_LOOK))
            .map(|look| look.recipe.material.clone())
            .unwrap_or_default()
    })
}

/// Matches `Lit` in `shaders/lit.wgsl`; see it for what each field holds.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct LitUniforms {
    f0: [[f32; 4]; 3],
    f90: [[f32; 4]; 3],
    dip: [[f32; 4]; 3],
    base: [f32; 4],
    coat: [f32; 4],
    sh: [[f32; 4]; 9],
    toward: [[f32; 4]; MAX_LIGHTS],
    illuminance: [[f32; 4]; MAX_LIGHTS],
    counts: [u32; 4],
    params: [f32; 4],
    scatter: [f32; 4],
    radii: [[f32; 4]; 6],
}

/// `rgb` and `w` as one vector.
fn four(rgb: [f32; 3], w: f32) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], w]
}

impl LitUniforms {
    /// Material `m` under `environment` with `lights`.
    pub fn new(m: &Material, environment: &GpuEnvironment, lights: &Lights) -> LitUniforms {
        let fresnels = [m.dielectric_fresnel(), m.metal_fresnel(), m.coat_fresnel()];
        let roughness = [m.specular_roughness, m.specular_roughness, m.coat_roughness];
        let discs = lights.discs(environment.key.as_ref());
        let mut uniforms = LitUniforms {
            f0: std::array::from_fn(|i| four(fresnels[i].f0, roughness[i])),
            f90: std::array::from_fn(|i| four(fresnels[i].f90, 0.0)),
            dip: std::array::from_fn(|i| four(fresnels[i].b, 0.0)),
            base: four(m.base_color.0, m.base_metalness),
            coat: four(m.coat_color.0, m.coat_weight),
            sh: environment.sh.map(|k| four(k, 0.0)),
            counts: [discs.len().min(MAX_LIGHTS) as u32, 0, 0, 0],
            params: [lights.ambient.max(0.0), 0.0, 0.0, 0.0],
            ..LitUniforms::zeroed()
        };
        for (i, disc) in discs.iter().take(MAX_LIGHTS).enumerate() {
            uniforms.toward[i] = four(disc.direction, disc.sin_radius().0);
            uniforms.illuminance[i] = four(disc.illuminance, 0.0);
        }
        uniforms
    }

    /// With `m`'s diffuse light spread under its surface as `s` sets: each channel's profile
    /// shape in scene units, the share that scatters, and the gather's radii. Unchanged when
    /// nothing would scatter.
    fn scattering(mut self, m: &Material, s: &Subsurface) -> LitUniforms {
        static RADII: OnceLock<Vec<f32>> = OnceLock::new();
        let weight = (m.subsurface_weight * s.strength).clamp(0.0, 1.0);
        let shapes: [f32; 3] = std::array::from_fn(|k| {
            let mfp = m.subsurface_radius * m.subsurface_radius_scale[k] * s.radius;
            (burley::shape(mfp, m.base_color.0[k]) / s.unit_mm).max(0.0)
        });
        let far = shapes[0].max(shapes[1]).max(shapes[2]);
        if weight <= 0.0 || !(far > 0.0 && far.is_finite()) {
            return self;
        }
        let radii = RADII.get_or_init(|| burley::split(SCATTER_SAMPLES));
        self.scatter = four(shapes, weight);
        for (i, r) in radii.iter().enumerate() {
            self.radii[i / 4][i % 4] = *r;
        }
        self
    }

    /// The uniforms as words, for the accumulation key: two frames with the same words and
    /// environment light the same way.
    pub fn words(&self) -> Vec<u32> {
        bytemuck::cast_slice(bytemuck::bytes_of(self)).to_vec()
    }
}

/// What the Lit branch reads for one frame.
pub(crate) struct LitFrame<'r> {
    pub uniforms: LitUniforms,
    pub environment: &'r GpuEnvironment,
    pub dfg: &'r wgpu::TextureView,
}

impl LitFrame<'_> {
    /// Whether live subsurface runs: the diffuse pass, and the gather in the display pass.
    pub fn scatters(&self) -> bool {
        self.uniforms.scatter[3] > 0.0
    }
}

/// The textures and sampler of Lit mode, with stand-ins for frames that do not use them.
pub(crate) struct LitTextures {
    /// Trilinear, for the prefiltered cube.
    pub sampler: wgpu::Sampler,
    pub blank_cube: wgpu::TextureView,
    pub blank_dfg: wgpu::TextureView,
    pub blank_diffuse: wgpu::TextureView,
    /// The DFG table and the default studio, made on the first Lit frame: preparing the studio
    /// takes the CPU a moment.
    defaults: OnceLock<(wgpu::TextureView, GpuEnvironment)>,
}

impl LitTextures {
    pub fn new(gpu: &Gpu) -> LitTextures {
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("prefiltered environment"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let black = Face {
            size: 1,
            texels: vec![[0.0; 3]],
        };
        let blank = Cube {
            levels: vec![std::array::from_fn(|_| black.clone())],
        };
        LitTextures {
            sampler,
            blank_cube: cube_texture(gpu, &blank),
            blank_dfg: float_texture(gpu, 1, &[[0.0; 4]]),
            blank_diffuse: diffuse_texture(gpu, [1, 1]),
            defaults: OnceLock::new(),
        }
    }

    /// The DFG table and the default studio, uploaded on first use.
    fn defaults(&self, gpu: &Gpu) -> Result<&(wgpu::TextureView, GpuEnvironment), RenderError> {
        if let Some(made) = self.defaults.get() {
            return Ok(made);
        }
        let studio = LitEnvironment::shipped(DEFAULT_ENVIRONMENT)?;
        Ok(self.defaults.get_or_init(|| {
            let dfg = float_texture(gpu, dfg::SIZE as u32, &Dfg::get().texels());
            (dfg, upload_environment(gpu, &studio))
        }))
    }
}

impl Renderer {
    /// Copies an environment prepared for Lit mode to the GPU.
    pub fn upload_environment(&self, environment: &LitEnvironment) -> GpuEnvironment {
        upload_environment(&self.gpu, environment)
    }

    /// What the Lit branch reads to draw `view`; `None` when its shading is not lit (MatCap,
    /// normals). Subsurface runs as [`SubsurfaceMode::runs`] says for this device.
    pub(crate) fn lit_frame<'r>(
        &'r self,
        view: &View<'r>,
    ) -> Result<Option<LitFrame<'r>>, RenderError> {
        let material = match view.shading {
            Shading::Clay | Shading::Zebra => clay_material(),
            Shading::Lit(m) => m,
            Shading::MatCap(_) | Shading::Normals | Shading::Flat(_) => return Ok(None),
        };
        let (dfg, studio) = self.lit.defaults(&self.gpu)?;
        let environment = view.light.environment.unwrap_or(studio);
        let mut uniforms = LitUniforms::new(material, environment, view.light.lights);
        if view.subsurface.mode.runs(self.gpu.integrated()) {
            uniforms = uniforms.scattering(material, &view.subsurface);
        }
        Ok(Some(LitFrame {
            uniforms,
            environment,
            dfg,
        }))
    }
}

fn upload_environment(gpu: &Gpu, environment: &LitEnvironment) -> GpuEnvironment {
    GpuEnvironment {
        cube: cube_texture(gpu, &environment.prefiltered),
        sh: environment.rest.sh9.polynomial(),
        key: environment.key,
        name: environment.name.clone(),
        id: next_id(),
    }
}

/// A cube texture of half floats with every level of `cube` as a mip level; faces in the GPU's
/// order and orientation, which `shading::cube` follows.
fn cube_texture(gpu: &Gpu, cube: &Cube) -> wgpu::TextureView {
    let size = cube.size() as u32;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        mip_level_count: cube.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST),
        view_formats: &[],
    });
    for (mip, level) in cube.levels.iter().enumerate() {
        let side = level[0].size as u32;
        let texels: Vec<[u16; 4]> = (level.iter().flat_map(|face| &face.texels))
            .map(|c| half_rgba(*c))
            .collect();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 8),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 6,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

/// A target for live subsurface's diffuse irradiance, `size` texels.
pub(crate) fn diffuse_texture(gpu: &Gpu, [width, height]: [u32; 2]) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diffuse irradiance"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DIFFUSE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING),
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// A square RGBA32F texture of `size` texels a side, read with `textureLoad`.
fn float_texture(gpu: &Gpu, size: u32, texels: &[[f32; 4]]) -> wgpu::TextureView {
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dfg"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST),
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 16),
            rows_per_image: None,
        },
        extent,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
