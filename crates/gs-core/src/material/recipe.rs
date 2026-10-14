//! The look file format (catalog item `material.look-recipe`, research 11, 12.1): a physical
//! material, the environment that lights it, and how the MatCap is baked. Nothing in it is a
//! look-shaping knob; the look follows from the material and the light. A recipe writes back
//! out in the same format, every key given and colors linear, so a scene file keeps it whole
//! (decision 0043, section 7).

use serde::{Deserialize, Serialize};

use super::{Color, ENVIRONMENTS, Material};

/// A look: what the baker turns into a MatCap.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// What the user reads, for example `MatCap Red Wax`.
    pub name: String,
    /// The catalog item this look implements, for example `material.matcap-red-wax`.
    #[serde(default)]
    pub catalog_id: String,
    /// What kind of material it is; left out, a MatCap look.
    #[serde(default)]
    pub kind: MaterialKind,
    /// The shipped environment that lights it (`assets/environments/<name>.toml`).
    #[serde(default = "studio")]
    pub environment: String,
    pub material: Material,
    #[serde(default)]
    pub bake: Bake,
    /// How it takes the session's occlusion: a standard material's own, neutral on any other.
    #[serde(default)]
    pub cavity: Cavity,
}

/// A standard material's cavity settings (decision 0043, step (c)): factors on the session's
/// occlusion, so 1, 1 and 0 draw it as it is. The renderer applies them in MatCap and Lit
/// shading.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Cavity {
    /// Scales the darkening occlusion leaves (one minus visibility), 0 to 2.
    pub intensity: f32,
    /// Multiplies the session's occlusion radius, 0.25 to 4.
    pub radius: f32,
    /// Gives back this share of the light occlusion takes, in `color`, 0 to 1.
    pub colorize: f32,
    /// The color Colorize gives back, each channel 0 to 1.
    pub color: Color,
}

impl Cavity {
    /// The session's occlusion as it is: what every material but a standard one has.
    pub const NEUTRAL: Cavity = Cavity {
        intensity: 1.0,
        radius: 1.0,
        colorize: 0.0,
        color: Color([0.0; 3]),
    };

    /// The ranges, each error naming its key. A material of any kind but standard has only the
    /// neutral settings.
    #[expect(
        clippy::float_cmp,
        reason = "a neutral setting is the exact default, written or not"
    )]
    fn check(&self, kind: MaterialKind) -> Result<(), String> {
        let n = Cavity::NEUTRAL;
        let keys = [
            ("intensity", self.intensity, n.intensity, [0.0, 2.0]),
            ("radius", self.radius, n.radius, [0.25, 4.0]),
            ("colorize", self.colorize, n.colorize, [0.0, 1.0]),
        ];
        let color = (self.color.0).map(|c| ("color", c, 0.0, [0.0, 1.0]));
        for (key, v, neutral, [lo, hi]) in keys.into_iter().chain(color) {
            if kind != MaterialKind::Standard && v != neutral {
                return Err(format!(
                    "{key}: only a standard material has cavity settings, not a {} one",
                    kind.name()
                ));
            }
            if !(lo..=hi).contains(&v) {
                return Err(format!("{key}: {v} is outside {lo} to {hi}"));
            }
        }
        Ok(())
    }
}

impl Default for Cavity {
    fn default() -> Self {
        Cavity::NEUTRAL
    }
}

/// What a material is (decision 0043, step (c)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaterialKind {
    /// A look baked into a MatCap under its environment, live in Lit shading.
    #[default]
    Matcap,
    /// A standard material: a preset of the shared model, drawn as a look in phase 1, the only
    /// kind with cavity settings.
    Standard,
    /// Paint times the base color, unlit and never baked: Flat Color. It has no settings.
    Flat,
}

impl MaterialKind {
    /// Its name in a look file and in `material.list`.
    pub fn name(self) -> &'static str {
        match self {
            MaterialKind::Matcap => "matcap",
            MaterialKind::Standard => "standard",
            MaterialKind::Flat => "flat",
        }
    }
}

impl Default for Recipe {
    /// An untitled MatCap look of `OpenPBR`'s default material under the studio.
    fn default() -> Self {
        Recipe {
            name: String::new(),
            catalog_id: String::new(),
            kind: MaterialKind::default(),
            environment: studio(),
            material: Material::default(),
            bake: Bake::default(),
            cavity: Cavity::NEUTRAL,
        }
    }
}

/// How a look becomes a MatCap.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Bake {
    /// Image width and height in pixels (512 by default, 16 to 4096).
    pub size: u32,
    /// The radius of the ball the MatCap pictures, in the units of `subsurface_radius`
    /// (shipped looks: millimeters). Only scattering materials depend on it: a MatCap is
    /// right for one size of subject.
    pub sphere_radius: f32,
}

impl Default for Bake {
    /// 512 pixels, a 50 mm ball (a hand-sized maquette).
    fn default() -> Self {
        Bake {
            size: 512,
            sphere_radius: 50.0,
        }
    }
}

fn studio() -> String {
    "studio".into()
}

impl Recipe {
    /// Reads and checks a look. An error starts with the key it is about, for example
    /// `material.specular_ior: 0.5 is outside 1 to 3`.
    pub fn parse(text: &str) -> Result<Recipe, String> {
        let recipe: Recipe = parse_toml(text)?;
        recipe.check()?;
        Ok(recipe)
    }

    /// The limits of `material.look-recipe`, each error naming its key.
    pub fn check(&self) -> Result<(), String> {
        if let Err(e) = self.material.check() {
            return Err(format!("material.{e}"));
        }
        if let Err(e) = self.cavity.check(self.kind) {
            return Err(format!("cavity.{e}"));
        }
        if !ENVIRONMENTS
            .iter()
            .any(|(name, _)| *name == self.environment)
        {
            let names: Vec<&str> = ENVIRONMENTS.iter().map(|(n, _)| *n).collect();
            return Err(format!(
                "environment: no environment named {:?} (shipped: {})",
                self.environment,
                names.join(", ")
            ));
        }
        if !(16..=4096).contains(&self.bake.size) {
            return Err(format!(
                "bake.size: {} is outside 16 to 4096",
                self.bake.size
            ));
        }
        if !(self.bake.sphere_radius > 0.0 && self.bake.sphere_radius.is_finite()) {
            return Err(format!(
                "bake.sphere_radius: {} must be above 0",
                self.bake.sphere_radius
            ));
        }
        Ok(())
    }

    /// What a bake of this look depends on: its material, environment and bake settings, not
    /// its title or catalog item. Two looks with the same key draw the same MatCap.
    pub fn bake_key(&self) -> (&Material, &str, &Bake) {
        (&self.material, &self.environment, &self.bake)
    }
}

/// Reads a TOML file into `T`. An error starts with the key it is about, for example
/// `material.specular_ior: invalid type: string "x", expected f32`.
pub fn parse_toml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    let de = toml::Deserializer::parse(text).map_err(|e| e.message().trim().to_string())?;
    serde_path_to_error::deserialize(de).map_err(|e| {
        let key = e.path().to_string();
        format!("{key}: {}", e.inner().message().trim())
    })
}
