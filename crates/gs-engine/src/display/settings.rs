//! `view.display`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{DisplaySettings, OcclusionRadius, SubsurfaceMode, Transform};
use crate::view::Shading;
use crate::{Command, Ctx, Example, register_command};

/// Set how the window and every look draw: the shading mode, the exposure, the view transform,
/// live subsurface, ambient occlusion, the floor, the mask's tint, the active part's outline,
/// face group colors, the MatCap flip and the live Boolean. Arguments left out keep their value.
///
/// `lit` draws the selected look as a material lit live by the session's environment and
/// lights (`view.environment`, `light.set`); `clay` and `zebra` draw the clay look the same way.
/// `view.snapshot` and `view.contact_sheet` draw in this shading unless they name one, and
/// always with this exposure, transform, subsurface, occlusion, mask tint and flip; the floor only
/// when they pass `grid`, the active part's outline only when they pass `active_outline` and
/// face group colors only when they pass `face_groups`. A display setting of the session, like
/// the selected look: never undone, never saved with the scene.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewDisplay {
    /// How surfaces are shaded: matcap (the default), lit, clay, zebra or normals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// In stops, -16 to 16: +1 doubles the light. Applies to every mode but normals. Starts at 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -16.0, max = 16.0))]
    pub exposure: Option<f32>,
    /// How light becomes a display color: pbr-neutral (the default), agx or standard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// Live subsurface, for looks that scatter (skin, wax) in Lit mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsurface: Option<SubsurfaceArgs>,
    /// Ambient occlusion's strength, 0 to 1: how dark creases get. 0 draws none and skips its
    /// passes. Applies to every mode but normals. Starts at 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub occlusion_strength: Option<f32>,
    /// How far ambient occlusion reaches, in scene units, so a crease shades the same at any
    /// zoom; or "auto" (the default): a tenth of the radius of the sphere around everything
    /// drawn, which follows the model's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_radius: Option<OcclusionRadius>,
    /// The floor grid in the window: a plane just below everything a view draws, lines at
    /// powers of ten of the scene unit. Starts off. Snapshots draw it only when they pass
    /// `grid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<bool>,
    /// Whether the mask shows as darkening. Starts shown. Hiding changes only the picture:
    /// masked points are still held back from every edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<bool>,
    /// How much a fully masked surface is darkened while the mask shows, 0 to 1. Starts at
    /// 0.75.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub mask_opacity: Option<f32>,
    /// An orange line just outside the active part in the window's views, while they draw two
    /// parts or more, so you see which part a stroke will touch. Starts on. Snapshots draw it
    /// only when they pass `active_outline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_outline: Option<bool>,
    /// Every part shows its face groups, one flat color each, in place of its paint; group 0
    /// is white, so a part with no groups shows its material. Starts off. Snapshots draw them
    /// only when they pass `face_groups`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_groups: Option<bool>,
    /// MatCap shading reads every MatCap mirrored left to right, so its light comes from the
    /// other side. Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcap_flip: Option<bool>,
    /// Live Boolean's live mode: each group of parts that show is cut again, off the engine
    /// thread, after every change to it, and `session.state`'s `boolean.live` reports each
    /// result; `boolean.apply` then takes a group's settled result instead of cutting it. The
    /// results are never saved, undone or logged. Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_boolean: Option<bool>,
    /// With Live Boolean's live mode on, the window marks each group's open and non-manifold
    /// edges, inside-out shells and self-crossings, as `boolean.check` finds them, on the faces
    /// they touch (decision 0057, section 9). Never saved or undone. Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_issues: Option<bool>,
    /// With Live Boolean's live mode on, the window marks the faces of each group's parts that
    /// lie on another part's, as `boolean.check` finds them. Never saved or undone. Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_coplanar: Option<bool>,
}

/// Live subsurface's settings to change.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubsurfaceArgs {
    /// auto (on except on an integrated GPU, the default), on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SubsurfaceMode>,
    /// 0 to 1: multiplies the look's subsurface weight. Starts at 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub strength: Option<f32>,
    /// 0 to 10: multiplies how far light travels under the surface. Starts at 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 10.0))]
    pub radius: Option<f32>,
    /// One scene unit in millimeters, 0.01 to 10000, the unit the looks' scattering distances
    /// are in. Starts at 50 (the default sphere is a 100 mm ball).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 10000.0))]
    pub unit_mm: Option<f32>,
}

impl Command for ViewDisplay {
    const NAME: &'static str = "view.display";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = DisplaySettings;

    fn examples() -> Vec<Example<Self>> {
        let lit = Self {
            shading: Some(Shading::Lit),
            ..Self::default()
        };
        let darker = Self {
            exposure: Some(-1.0),
            transform: Some(Transform::Agx),
            ..Self::default()
        };
        let wax = Self {
            subsurface: Some(SubsurfaceArgs {
                radius: Some(2.0),
                ..SubsurfaceArgs::default()
            }),
            ..Self::default()
        };
        let creases = Self {
            occlusion_strength: Some(0.5),
            occlusion_radius: Some(OcclusionRadius::Units(0.3)),
            ..Self::default()
        };
        let auto = Self {
            occlusion_radius: Some(OcclusionRadius::AUTO),
            ..Self::default()
        };
        let floor = Self {
            floor: Some(true),
            ..Self::default()
        };
        let unmasked = Self {
            mask: Some(false),
            ..Self::default()
        };
        let faint = Self {
            mask: Some(true),
            mask_opacity: Some(0.4),
            ..Self::default()
        };
        let unoutlined = Self {
            active_outline: Some(false),
            ..Self::default()
        };
        let groups = Self {
            face_groups: Some(true),
            ..Self::default()
        };
        let flipped = Self {
            matcap_flip: Some(true),
            ..Self::default()
        };
        vec![
            Example::new("draw the selected look lit live", lit),
            Example::new("one stop darker, through AgX", darker),
            Example::new("light travels twice as far under the surface", wax),
            Example::new("half-strength occlusion reaching 0.3 scene units", creases),
            Example::new("occlusion's radius back to automatic", auto),
            Example::new("show the floor grid", floor),
            Example::new("hide the mask's darkening; the mask still holds", unmasked),
            Example::new("show the mask, darkening it less", faint),
            Example::new("stop outlining the active part", unoutlined),
            Example::new("color every face group", groups),
            Example::new("mirror the MatCap, lighting from the other side", flipped),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<DisplaySettings, CommandError> {
        let d = &mut *cx.display;
        if let Some(shading) = self.shading {
            d.shading = shading;
        }
        if let Some(exposure) = self.exposure {
            d.exposure = exposure;
        }
        if let Some(transform) = self.transform {
            d.transform = transform;
        }
        if let Some(s) = self.subsurface {
            let ss = &mut d.subsurface;
            ss.mode = s.mode.unwrap_or(ss.mode);
            ss.strength = s.strength.unwrap_or(ss.strength);
            ss.radius = s.radius.unwrap_or(ss.radius);
            ss.unit_mm = s.unit_mm.unwrap_or(ss.unit_mm);
        }
        if let Some(strength) = self.occlusion_strength {
            d.occlusion.strength = strength;
        }
        if let Some(radius) = self.occlusion_radius {
            d.occlusion.radius = radius;
        }
        d.floor = self.floor.unwrap_or(d.floor);
        d.mask.shown = self.mask.unwrap_or(d.mask.shown);
        d.mask.opacity = self.mask_opacity.unwrap_or(d.mask.opacity);
        d.active_outline = self.active_outline.unwrap_or(d.active_outline);
        d.face_groups = self.face_groups.unwrap_or(d.face_groups);
        d.matcap_flip = self.matcap_flip.unwrap_or(d.matcap_flip);
        d.live_boolean = self.live_boolean.unwrap_or(d.live_boolean);
        d.show_issues = self.show_issues.unwrap_or(d.show_issues);
        d.show_coplanar = self.show_coplanar.unwrap_or(d.show_coplanar);
        Ok(d.clone())
    }
}
register_command!(ViewDisplay, "Set display");
