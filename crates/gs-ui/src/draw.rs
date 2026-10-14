//! The one way a 3D view is drawn (decision 0024): the window's main view, every Reference
//! View, `view.snapshot` and `view.contact_sheet`. Parts reach the GPU from their mesh stores
//! (`Renderer::upload_store`, decision 0012), so hidden faces stay hidden and the mask shows; a
//! view's look is described here once ([`Look`]); gs-render's frame draws it. The window draws
//! one jittered sample per paint into its frame; a look draws all of its samples at once.

use gs_engine::{DisplaySettings, RenderRequest, Screenshot, Shading as Shade};
use gs_engine::{SubsurfaceMode as Mode, Transform};
use gs_render::{
    Camera, Display, GpuMesh, Light, MeshStore, RenderSize, Renderer, Shading, Subsurface,
    SubsurfaceMode, View, ViewTransform, Wire,
};

use crate::lighting::Lighting;
use crate::matcaps::MatCaps;

/// What a 3D view looks like, besides what it shows.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per thing a view may draw"
)]
pub(crate) struct Look<'a> {
    pub camera: Camera,
    pub shading: Shade,
    /// The MatCap `shading` draws, by name; `None`: the selected one. Lit shading draws its
    /// recipe's material.
    pub matcap: Option<&'a str>,
    pub wireframe: bool,
    /// The floor grid: the session's floor in the window's views, a look's own `grid`.
    pub grid: bool,
    /// Outline the marked part, the active one (decision 0033, section 5): the session's
    /// `active_outline` in the window's views, a look's own `active_outline`.
    pub outline: bool,
    /// Every part shows its face groups in place of its paint (decision 0039, section 9): the
    /// session's `face_groups` in the window's views, a look's own `face_groups`.
    pub face_groups: bool,
    /// Every part colored by its polygons' size on screen: the session's in the window's
    /// views, a look's own `density`.
    pub density: bool,
    /// The session's display (decision 0023): exposure, view transform, environment, lights,
    /// subsurface, occlusion and the mask's tint. Its floor reaches the view through `grid`,
    /// its outline through `outline` and its face groups through `face_groups`.
    pub display: &'a DisplaySettings,
}

impl<'a> Look<'a> {
    /// `camera` in the session's shading and the selected MatCap, with the session's floor,
    /// outline and face groups and no wireframe: the main view.
    pub fn main(camera: Camera, display: &'a DisplaySettings) -> Self {
        Self {
            camera,
            shading: display.shading,
            matcap: None,
            wireframe: false,
            grid: display.floor,
            outline: display.active_outline,
            face_groups: display.face_groups,
            density: display.wire.density,
            display,
        }
    }

    /// gs-render's view of this look, its MatCap baked and its environment uploaded for
    /// `renderer` the first time.
    pub fn view<'m>(
        &self,
        renderer: &Renderer,
        (matcaps, lighting): (&'m mut MatCaps, &'m mut Lighting),
    ) -> Result<View<'m>, String> {
        // Clay and zebra draw the clay look; each lit look lights with its own environment
        // unless the session chose one.
        let lit = match self.shading {
            Shade::Lit => Some(matcaps.recipe(self.matcap)?),
            Shade::Clay | Shade::Zebra => Some(matcaps.clay()?),
            Shade::Matcap | Shade::Normals => None,
        };
        let light = match lit {
            Some(recipe) => lighting.light(renderer, self.display, &recipe.environment)?,
            None => Light::default(),
        };
        let cavity = matcaps
            .recipe(self.matcap)
            .map(|r| r.cavity)
            .unwrap_or_default();
        let shading = match (self.shading, self.matcap, matcaps.flat(self.matcap)) {
            (Shade::Matcap | Shade::Lit, _, Some(base)) => Shading::Flat(base),
            (Shade::Matcap, Some(name), _) => Shading::MatCap(matcaps.gpu_of(renderer, name)?),
            (Shade::Matcap, None, _) => Shading::MatCap(matcaps.gpu(renderer)?),
            (Shade::Lit, _, _) => Shading::Lit(&matcaps.recipe(self.matcap)?.material),
            (Shade::Clay, _, _) => Shading::Clay,
            (Shade::Zebra, _, _) => Shading::Zebra,
            (Shade::Normals, _, _) => Shading::Normals,
        };
        let base = View::new(self.camera, shading);
        Ok(crate::canvas::canvas(
            self.display,
            View {
                wire: if self.wireframe {
                    Wire::Auto
                } else {
                    Wire::Off
                },
                grid: self.grid,
                outline: self.outline,
                face_groups: self.face_groups,
                density: self.density,
                mask_opacity: self.display.mask.drawn(),
                display: Display {
                    exposure: self.display.exposure,
                    transform: match self.display.transform {
                        Transform::PbrNeutral => ViewTransform::PbrNeutral,
                        Transform::Agx => ViewTransform::Agx,
                        Transform::Standard => ViewTransform::Standard,
                    },
                    ..base.display
                },
                occlusion: gs_render::Occlusion {
                    strength: self.display.occlusion.strength,
                    radius: self.display.occlusion.radius.units(),
                    cavity,
                },
                light,
                subsurface: subsurface(self.display),
                matcap_flip: self.display.matcap_flip,
                ..base
            },
        ))
    }
}

/// `display`'s live subsurface as gs-render takes it.
pub(crate) fn subsurface(display: &DisplaySettings) -> Subsurface {
    let s = &display.subsurface;
    Subsurface {
        mode: match s.mode {
            Mode::Auto => SubsurfaceMode::Auto,
            Mode::On => SubsurfaceMode::On,
            Mode::Off => SubsurfaceMode::Off,
        },
        strength: s.strength,
        radius: s.radius,
        unit_mm: s.unit_mm,
    }
}

/// Whether a part draws anything: a store with no vertices has nothing to upload.
pub(crate) fn drawn(store: &MeshStore) -> bool {
    store.vertex_count() > 0
}

/// Jittered samples averaged for anti-aliased looks (the default; decision 0019). Golden images
/// render through the test kit at one sample instead, which keeps references fixed.
const LOOK_SAMPLES: u32 = 4;

/// A look (`view.snapshot`, `view.contact_sheet`): `parts` uploaded once, then drawn through
/// every camera of `request` as one PNG. The uploads are dropped when it returns (decision
/// 0024, section 2).
pub(crate) fn look(
    renderer: &Renderer,
    looks: (&mut MatCaps, &mut Lighting),
    parts: &[&MeshStore],
    request: &RenderRequest,
) -> Result<Screenshot, String> {
    let text = |e: gs_render::RenderError| e.to_string();
    let uploaded: Vec<GpuMesh> = (parts.iter().zip(&request.places).enumerate())
        .filter(|(_, (s, _))| drawn(s))
        .map(|(i, (s, t))| {
            let marked = request.outlined == Some(i);
            let painted = !request.unpainted.contains(&i);
            renderer
                .upload_store(s)
                .placed(*t, true)
                .marked(marked)
                .painted(painted)
        })
        .collect();
    let meshes: Vec<&GpuMesh> = uploaded.iter().collect();
    let size = RenderSize {
        width: request.tile[0],
        height: request.tile[1],
        samples: if request.draws.antialias {
            LOOK_SAMPLES
        } else {
            1
        },
    };
    let first = request.cameras.first().ok_or("a look needs a camera")?;
    let look = Look {
        camera: first.camera,
        shading: request.shading,
        matcap: request.matcap.as_deref(),
        wireframe: request.draws.wireframe,
        grid: request.draws.grid,
        outline: request.outlined.is_some(),
        face_groups: request.draws.face_groups,
        density: request.draws.density,
        display: &request.display,
    };
    // Every camera of a request has the same look.
    let base = look.view(renderer, looks)?;
    let views: Vec<(&str, View)> = (request.cameras.iter())
        .map(|c| {
            (
                c.label.as_str(),
                View {
                    camera: c.camera,
                    ..base
                },
            )
        })
        .collect();
    let image = match views.as_slice() {
        [(_, view)] => renderer.snapshot(meshes.as_slice(), view, size),
        several => renderer.contact_sheet(meshes.as_slice(), several, size, request.columns),
    }
    .map_err(text)?;
    Ok(Screenshot {
        width: image.width,
        height: image.height,
        png: image.png().map_err(text)?,
    })
}
