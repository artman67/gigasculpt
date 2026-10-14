//! The session's display (decision 0023): how the window and every look draw. The shading mode,
//! exposure, view transform, live subsurface, ambient occlusion, the floor, the mask's tint and
//! the active part's outline (`view.display`), the environment Lit mode lights with (`view.environment`) and the lights
//! of `light.palette` (`light.set`).
//!
//! Display state, not document data: never undone and, but for the background, never saved with
//! the scene, like the selected MatCap. Like the MatCap it is logged, so a replay draws the same pictures; unlike
//! it, it lives here rather than in the render host, so a session that cannot render still
//! keeps what a log sets. The window draws from it through the session's state (decision 0013); the
//! looks get it with their render request.

mod background;
pub(crate) mod color;
mod environment;
mod floor;
mod lights;
mod settings;
mod wire;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::view::Shading;

pub use background::Background;
pub use environment::{EnvironmentInfo, KeyLight};
pub use floor::{FloorGrid, FloorPreset};
pub use wire::WireStyle;

/// How light becomes a display color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    /// Khronos PBR Neutral, the default: base colors show as set under unit light and
    /// highlights compress toward white.
    #[default]
    PbrNeutral,
    /// `AgX`, Blender's default: brights go toward white as on film.
    Agx,
    /// Clamps to 0..1 and nothing else.
    Standard,
}

/// When live subsurface runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubsurfaceMode {
    /// On, except on an integrated GPU, where it costs about as much as the rest of the frame.
    #[default]
    Auto,
    On,
    Off,
}

/// Live subsurface (`material.wax-preview`): in Lit mode, a look that scatters (skin, wax) has
/// its diffuse light spread under its surface, as its MatCap's bake does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Subsurface {
    pub mode: SubsurfaceMode,
    /// Multiplies the look's subsurface weight: 0 draws it without scattering.
    pub strength: f32,
    /// Multiplies how far light travels under the surface (the look's mean free paths).
    pub radius: f32,
    /// The length of one scene unit in millimeters, which the looks' mean free paths are in.
    /// 50 makes the default sphere the 100 mm ball the MatCaps picture.
    pub unit_mm: f32,
}

impl Default for Subsurface {
    /// Auto, strength 1, radius 1, 50 mm per scene unit.
    fn default() -> Self {
        Subsurface {
            mode: SubsurfaceMode::Auto,
            strength: 1.0,
            radius: 1.0,
            unit_mm: 50.0,
        }
    }
}

/// Ambient occlusion (`render.live-ambient-occlusion`, decision 0019 as amended): creases
/// darkened where the surrounding surface blocks the environment's light. Every physical look
/// takes it (MatCap, Lit, Clay and zebra over Clay); normals never.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Occlusion {
    /// 0 (none: the passes are skipped) to 1 (full).
    pub strength: f32,
    /// How far a surface looks for what blocks its light.
    pub radius: OcclusionRadius,
}

impl Default for Occlusion {
    /// Full strength, the automatic radius.
    fn default() -> Self {
        Occlusion {
            strength: 1.0,
            radius: OcclusionRadius::AUTO,
        }
    }
}

/// How the mask shows (`masking.view-mask`, decision 0023 as amended for Q2): a fully masked
/// surface is darkened by `opacity`. Only the picture: hidden or not, masked points are held
/// back as much.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MaskTint {
    pub shown: bool,
    /// 0 (no darkening) to 1 (masked surface drawn black).
    pub opacity: f32,
}

impl MaskTint {
    /// How much a fully masked surface is darkened as drawn: 0 while the mask is hidden.
    pub fn drawn(self) -> f32 {
        if self.shown { self.opacity } else { 0.0 }
    }
}

impl Default for MaskTint {
    /// Shown, at Blender's default opacity.
    fn default() -> Self {
        MaskTint {
            shown: true,
            opacity: 0.75,
        }
    }
}

/// How far ambient occlusion reaches: "auto", or a distance in scene units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OcclusionRadius {
    /// "auto": a tenth of the radius of the sphere around everything drawn, so it follows the
    /// model's size.
    Auto(AutoWord),
    /// A distance in scene units, 0 to a million: a crease shades the same at any zoom.
    Units(#[schemars(range(min = 0.0, max = 1.0e6))] f32),
}

impl OcclusionRadius {
    /// The automatic radius.
    pub const AUTO: OcclusionRadius = OcclusionRadius::Auto(AutoWord::Auto);

    /// The distance in scene units; `None` while it is automatic.
    pub fn units(self) -> Option<f32> {
        match self {
            OcclusionRadius::Auto(_) => None,
            OcclusionRadius::Units(r) => Some(r),
        }
    }
}

/// The word "auto".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutoWord {
    Auto,
}

/// What Lit mode lights with: an environment fixed to the camera.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Each look's own (the environment its recipe names; `studio` for most).
    #[default]
    Own,
    /// A shipped environment, by name.
    Shipped(String),
    /// A Radiance `.hdr` file (an equirectangular light probe).
    File {
        path: String,
        /// The BLAKE3 hash of the file's bytes when it was read.
        hash: String,
        /// Degrees about the vertical axis, positive turning the light to the viewer's right;
        /// `None`: the measured turn that puts its dominant light on the key side.
        turn: Option<f32>,
    },
}

/// One light as `light.set` left it. A value left unset follows the environment (light 1 is the
/// environment's key light) or the default (lights 2 to 8: white, intensity 1, from the upper
/// left).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LightSetting {
    pub on: bool,
    /// Linear RGB, scaled to luminance 1.
    pub color: Option<[f32; 3]>,
    /// 1 lights a white card facing the light to reflect 1.
    pub intensity: Option<f32>,
    /// Toward the light in view space: x right, y up, z toward the viewer. Length 1.
    pub direction: Option<[f32; 3]>,
    /// The light's angular radius in degrees.
    pub size_deg: Option<f32>,
}

/// Lights in the palette.
pub const LIGHTS: usize = 8;

/// The session's display: what `view.display`, `view.environment` and `light.set` set.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent display switches, each one view.display sets on its own"
)]
pub struct DisplaySettings {
    /// The window's shading, and every look's that does not name one.
    pub shading: Shading,
    /// In stops: +1 doubles the light.
    pub exposure: f32,
    pub transform: Transform,
    pub subsurface: Subsurface,
    pub occlusion: Occlusion,
    /// The floor grid under everything the window's views draw (`canvas.floor-grid`).
    /// Snapshots take their own `grid` instead.
    pub floor: bool,
    pub mask: MaskTint,
    /// The active part outlined in the window's views of the scene while they draw two parts
    /// or more (decision 0033, section 5). Snapshots take their own `active_outline` instead.
    pub active_outline: bool,
    /// Every part shows its face groups in place of its paint (decision 0039, section 9).
    /// Snapshots take their own `face_groups` instead.
    pub face_groups: bool,
    /// MatCaps read mirrored left to right (catalog item `material.matcap-shading`).
    pub matcap_flip: bool,
    /// Live Boolean's live mode: each group of parts that show is cut again off the engine
    /// thread after every change to it (decision 0057, section 8).
    pub live_boolean: bool,
    /// The live mode's issue marks and coplanar marks (decision 0057, section 9).
    pub show_issues: bool,
    pub show_coplanar: bool,
    pub environment: Environment,
    /// Scales the environment's light (all of it but light 1).
    pub ambient: f32,
    /// Lights 1 to 8.
    pub lights: [LightSetting; LIGHTS],
    // The canvas (catalog items canvas.floor-grid, canvas.wireframe-view, canvas.background).
    /// How the floor grid is drawn while `floor` shows it (`view.floor`).
    pub floor_grid: FloorGrid,
    /// How every wire is drawn, the density view and dots (`view.wire`).
    pub wire: WireStyle,
    /// What shows where no part does (`view.background`); unlike the rest, saved with the scene.
    pub background: Background,
}

impl Default for DisplaySettings {
    /// MatCap shading, exposure 0, PBR Neutral, subsurface and occlusion on their defaults, no
    /// floor, the mask shown at 0.75, the active part outlined, no face group colors, MatCaps
    /// unflipped, no live Boolean, each look's own environment, Ambient 1, light 1 on and
    /// lights 2 to 8 off.
    fn default() -> Self {
        let mut lights = [LightSetting::default(); LIGHTS];
        lights[0].on = true;
        DisplaySettings {
            shading: Shading::Matcap,
            exposure: 0.0,
            transform: Transform::PbrNeutral,
            subsurface: Subsurface::default(),
            occlusion: Occlusion::default(),
            floor: false,
            mask: MaskTint::default(),
            active_outline: true,
            face_groups: false,
            matcap_flip: false,
            live_boolean: false,
            show_issues: false,
            show_coplanar: false,
            environment: Environment::Own,
            ambient: 1.0,
            lights,
            floor_grid: FloorGrid::default(),
            wire: WireStyle::default(),
            background: Background::default(),
        }
    }
}

impl DisplaySettings {
    /// The canvas's values by the names `session.state` gives them.
    pub(crate) fn canvas_values(&self) -> impl Iterator<Item = (String, serde_json::Value)> {
        let floor = self.floor_grid.values().into_iter();
        let (wire, background) = (self.wire.values(), self.background.values());
        (floor.chain(wire).chain(background)).map(|(k, v)| (k.to_string(), v))
    }

    /// What `session.state` names the environment: a shipped environment's name, a file's
    /// path, or `None` for each look's own.
    pub fn environment_name(&self) -> Option<&str> {
        match &self.environment {
            Environment::Own => None,
            Environment::Shipped(name) => Some(name),
            Environment::File { path, .. } => Some(path),
        }
    }
}
