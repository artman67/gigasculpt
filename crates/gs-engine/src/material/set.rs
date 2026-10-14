//! `material.set` (decision 0043, section 8, step (b)): any of a material's physical keys and
//! the environment that lights it, or a built-in's shipped settings back; one palette step a
//! call that changes something.

use std::sync::Arc;

use gs_core::material::{Color, ENVIRONMENTS, MaterialKind, Recipe, Rgb};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::edit::{MaterialChanged, changed};
use super::{MaterialInfo, MaterialPalette, PaletteEntry};
use crate::errors::{args_invalid, internal};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// A color as a look file writes it, for the schema: `"#rrggbb"` (8-bit sRGB) or `[r, g, b]`
/// (linear, each 0 to 1). gs-core's `Color` reads both.
#[derive(JsonSchema)]
#[serde(untagged)]
#[expect(
    dead_code,
    reason = "only its schema is used; gs-core's Color reads the value"
)]
enum ColorArg {
    Hex(#[schemars(regex(pattern = crate::display::color::PATTERN))] String),
    Linear([f32; 3]),
}

/// Change a material: any of its physical keys, named as in a look file's `[material]` table
/// (the names of the `OpenPBR` surface standard), and the environment that lights it. Built-in
/// materials may be changed (their shipped file never is), and `shipped` puts a built-in's
/// shipped settings back. The changed material is checked as a look file is. One step of the
/// palette's own undo (`material.undo`); the window bakes the changed look again.
/// `material.list` reports every material's `values` in this form, colors linear. A standard
/// material (kind `standard`) also takes the `cavity_` keys, its own scale on the session's
/// occlusion (`view.display`); any other kind refuses them. A flat material (Flat Color, kind
/// `flat`) has no settings and is refused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialSet {
    /// The material: `#id`, `@place` (from 0) or its name. Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub material: Option<String>,
    /// Start from the built-in material's shipped settings (its keys and environment; its name
    /// and title stay), then apply any key given. Refused on a user material.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shipped: bool,
    /// The shipped environment that lights it, for example `soft-studio`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Diffuse albedo; for a metal, its reflectance at normal incidence. Each channel 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ColorArg>")]
    pub base_color: Option<Color>,
    /// 0 dielectric, 1 metal; in between only for dusty or dirty metal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_metalness: Option<f32>,
    /// Scales the specular reflectance, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_weight: Option<f32>,
    /// Tints a dielectric's reflectance at normal incidence; for a metal, its edge tint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ColorArg>")]
    pub specular_color: Option<Color>,
    /// Perceptual roughness, 0 (mirror) to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_roughness: Option<f32>,
    /// A dielectric's index of refraction, 1 to 3: 1.5 reflects 4% at normal incidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specular_ior: Option<f32>,
    /// How much of a clear coat covers the base, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coat_weight: Option<f32>,
    /// What the coat absorbs on the way to the base and back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ColorArg>")]
    pub coat_color: Option<Color>,
    /// The coat's roughness, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coat_roughness: Option<f32>,
    /// The coat's index of refraction, 1 to 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coat_ior: Option<f32>,
    /// How much of the dielectric body scatters under the surface, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsurface_weight: Option<f32>,
    /// Mean free path of light under the surface, 0 or more, in the units of the bake's ball
    /// (shipped looks: millimeters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsurface_radius: Option<f32>,
    /// Per-channel multiplier on the radius, each 0 or more (red travels furthest in skin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-rgb" = true))]
    pub subsurface_radius_scale: Option<Rgb>,
    /// A standard material's cavity Intensity, 0 to 2: scales the darkening the session's
    /// occlusion leaves (1 as it is, 0 none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavity_intensity: Option<f32>,
    /// A standard material's cavity Radius, 0.25 to 4: multiplies the session's occlusion
    /// radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavity_radius: Option<f32>,
    /// A standard material's cavity Colorize, 0 to 1: gives back that share of the light
    /// occlusion takes, in `cavity_color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavity_colorize: Option<f32>,
    /// The color Colorize gives back (black by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<ColorArg>")]
    pub cavity_color: Option<Color>,
}

impl MaterialSet {
    /// Every key of `r`, as `material.list` reports them.
    pub(super) fn values(r: &Recipe) -> MaterialSet {
        let (m, c) = (&r.material, r.cavity);
        MaterialSet {
            base_color: Some(m.base_color),
            base_metalness: Some(m.base_metalness),
            specular_weight: Some(m.specular_weight),
            specular_color: Some(m.specular_color),
            specular_roughness: Some(m.specular_roughness),
            specular_ior: Some(m.specular_ior),
            coat_weight: Some(m.coat_weight),
            coat_color: Some(m.coat_color),
            coat_roughness: Some(m.coat_roughness),
            coat_ior: Some(m.coat_ior),
            subsurface_weight: Some(m.subsurface_weight),
            subsurface_radius: Some(m.subsurface_radius),
            subsurface_radius_scale: Some(m.subsurface_radius_scale),
            cavity_intensity: Some(c.intensity),
            cavity_radius: Some(c.radius),
            cavity_colorize: Some(c.colorize),
            cavity_color: Some(c.color),
            ..MaterialSet::default()
        }
    }

    /// Puts each key given on `r`.
    fn apply(&self, r: &mut Recipe) {
        let put = |to: &mut f32, v: Option<f32>| *to = v.unwrap_or(*to);
        let tint = |to: &mut Color, v: Option<Color>| *to = v.unwrap_or(*to);
        let c = &mut r.cavity;
        put(&mut c.intensity, self.cavity_intensity);
        put(&mut c.radius, self.cavity_radius);
        put(&mut c.colorize, self.cavity_colorize);
        tint(&mut c.color, self.cavity_color);
        let m = &mut r.material;
        tint(&mut m.base_color, self.base_color);
        put(&mut m.base_metalness, self.base_metalness);
        put(&mut m.specular_weight, self.specular_weight);
        tint(&mut m.specular_color, self.specular_color);
        put(&mut m.specular_roughness, self.specular_roughness);
        put(&mut m.specular_ior, self.specular_ior);
        put(&mut m.coat_weight, self.coat_weight);
        tint(&mut m.coat_color, self.coat_color);
        put(&mut m.coat_roughness, self.coat_roughness);
        put(&mut m.coat_ior, self.coat_ior);
        put(&mut m.subsurface_weight, self.subsurface_weight);
        put(&mut m.subsurface_radius, self.subsurface_radius);
        let scale = &mut m.subsurface_radius_scale;
        *scale = self.subsurface_radius_scale.unwrap_or(*scale);
    }

    /// No key and no environment given.
    fn changes_nothing(&self) -> bool {
        let keys = MaterialSet {
            material: None,
            shipped: false,
            ..self.clone()
        };
        keys == MaterialSet::default()
    }
}

/// The recipe built-in material `entry` shipped with, found by its id: a built-in keeps its id
/// (`#1` to `#13`, then `#1001` on).
fn shipped(entry: &PaletteEntry) -> Result<Arc<Recipe>, CommandError> {
    let shipped = MaterialPalette::shipped().materials.into_iter();
    let found = shipped
        .filter(|_| entry.built_in)
        .find(|m| m.id == entry.id);
    found
        .map(|m| m.recipe)
        .ok_or_else(|| internal("a built-in material has no shipped recipe"))
}

/// A look file's refusal (`material.specular_ior: 0.5 is outside 1 to 3`) as an argument
/// refused at its key (a `[cavity]` table's key with `cavity_` before it).
fn refused(e: &str) -> CommandError {
    let message = e.strip_prefix("material.").unwrap_or(e);
    let message = message.replacen("cavity.", "cavity_", 1);
    let key = message.split(':').next().unwrap_or_default();
    let mut refused = args_invalid(&format!("/{key}"), &message);
    if key == "environment" {
        refused.hint = Some(format!("the shipped environments: {}", environments()));
    }
    refused
}

fn environments() -> String {
    let names: Vec<&str> = ENVIRONMENTS.iter().map(|(name, _)| *name).collect();
    names.join(", ")
}

impl Command for MaterialSet {
    const NAME: &'static str = "material.set";
    const KIND: Kind = Kind::View;
    /// The window bakes the changed look at once, as `material.select` bakes a new one.
    const COST: Cost = Cost::Short;
    type Output = MaterialChanged;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "make red wax glossier",
                json!({ "material": "red-wax", "specular_roughness": 0.2 }),
            ),
            Example::raw(
                "the active material in a darker gray, as a look file writes it",
                json!({ "base_color": "#404040" }),
            ),
            Example::raw(
                "red wax back as it shipped",
                json!({ "material": "red-wax", "shipped": true }),
            )
            .after(
                "material.set",
                json!({ "material": "red-wax", "coat_weight": 0.5 }),
            ),
            Example::raw(
                "Basic Material's creases half as dark, tinted toward red",
                json!({
                    "material": "basic-material",
                    "cavity_intensity": 0.5,
                    "cavity_colorize": 0.25,
                    "cavity_color": "#ff0000"
                }),
            ),
            Example::raw(
                "an index of refraction below 1",
                json!({ "material": "red-wax", "specular_ior": 0.5 }),
            )
            .fails_with(ErrorCode::ArgsInvalid),
            Example::raw(
                "a copy has no shipped settings",
                json!({ "material": "#14", "shipped": true }),
            )
            .after("material.copy", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::raw(
                "Flat Color has no settings",
                json!({ "material": "flat-color", "base_color": "#ff0000" }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    /// `shipped` on a user material cannot run, so the window greys its Reset there.
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        let palette = &cx.palette.now;
        let named = match &self.material {
            Some(key) => palette.find(key).ok(),
            None => Some(palette.active()),
        };
        match named {
            Some(m) if m.recipe.kind == MaterialKind::Flat => Err(Unavailable::new(format!(
                "{} is flat: the paint times its base color, with no settings to edit",
                m.name
            ))
            .fix("copy a MatCap or standard material and edit the copy")),
            Some(m) if self.shipped && !m.built_in => Err(Unavailable::new(format!(
                "{} is not built in, so it has no shipped settings",
                m.name
            ))
            .fix("only built-in materials have shipped settings to go back to")),
            _ => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<MaterialChanged, CommandError> {
        if !self.shipped && self.changes_nothing() {
            return Err(args_invalid(
                "",
                "give a key to change, an environment, or shipped",
            ));
        }
        let palette = &cx.palette.now;
        let old = match &self.material {
            Some(key) => palette.find(key)?,
            None => palette.active(),
        };
        let mut recipe = (*old.recipe).clone();
        if self.shipped {
            let shipped = shipped(old)?;
            recipe.material = shipped.material.clone();
            recipe.environment.clone_from(&shipped.environment);
            recipe.cavity = shipped.cavity;
        }
        self.apply(&mut recipe);
        if let Some(environment) = &self.environment {
            recipe.environment.clone_from(environment);
        }
        recipe.check().map_err(|e| refused(&e))?;
        if recipe == *old.recipe {
            let unchanged = MaterialChanged {
                material: MaterialInfo::of(old),
                active: palette.active().name.clone(),
                references: Vec::new(),
            };
            cx.noop_reason("the material already has those settings, so no step is made");
            return Ok(unchanged);
        }
        let edited = PaletteEntry {
            recipe: Arc::new(recipe),
            ..old.clone()
        };
        let mut materials = palette.materials.clone();
        for m in materials.iter_mut().filter(|m| m.id == edited.id) {
            m.clone_from(&edited);
        }
        changed(cx, materials, Self::NAME, &edited)
    }
}
register_command!(MaterialSet, "Set material");
