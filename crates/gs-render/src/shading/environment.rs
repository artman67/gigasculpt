//! Environments: the light around a look, as HDR radiance in every direction (research 11,
//! section 4). One environment lights both the diffuse and the specular response, so the two
//! can never disagree. It is locked to the camera, like every MatCap: x right, y up, z toward
//! the viewer.
//!
//! An environment file (`assets/environments/<name>.toml`) holds one of two kinds: a procedural
//! studio (a room with a floor and soft boxes, in cd/m^2) or a captured light probe (an
//! equirectangular Radiance `.hdr` image beside the file, turned about the vertical axis;
//! decision 0019). A user's `.hdr` file is a probe too. Every kind becomes the same
//! [`Environment`]: a radiance cube with mip levels, an irradiance map and a spherical-harmonic
//! fit, metered so a white diffuser facing the camera reflects exactly 1. Exposure 0 then means
//! what an incident-light meter at the subject would set, whatever units the source used.

use super::Rgb;
use super::cube::Cube;
use super::irradiance::{Sh9, irradiance, irradiance_map};
use crate::error::RenderError;
use crate::math::{V3, cross, dot, normalize_or};
use serde::Deserialize;
use std::f32::consts::PI;
use std::sync::{Arc, OnceLock};

/// Texels per face of the radiance cube: about 0.7 degrees each, as sharp as a 512 MatCap of
/// a mirror ball resolves at its center.
pub const CUBE_SIZE: usize = 128;
/// Samples per texel side when rasterizing a studio, so soft box edges are antialiased.
const SUPERSAMPLE: usize = 4;
/// The brightest any radiance may be, read or metered: 2^64, ten orders of magnitude above the
/// sun's disc (about 1.6e9 cd/m^2), so no capture comes near it in any unit. Below it no sum
/// taken in preparing an environment (supersampling, mip levels, metering, the spherical-harmonic
/// fit, the GGX prefilter) leaves f32. RGBE reaches 1.7e38, where they did, and the picture
/// went black without a word: a file brighter than this is refused, and metering holds a texel
/// at it (review 0005, m4; decision 0023).
pub const BRIGHTEST: f32 = 18_446_744_073_709_551_616.0;

/// A studio: a room and its lights.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Studio {
    pub name: String,
    pub room: Room,
    #[serde(default, rename = "softbox")]
    pub softboxes: Vec<SoftBox>,
}

/// The room around the subject. Above the horizon its radiance runs from `walls` at the
/// horizon to `ceiling` straight up. Below it is the floor, a diffuse surface of albedo
/// `floor_albedo`, whose radiance is computed from the light falling on it.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Room {
    pub walls: f32,
    pub ceiling: f32,
    pub floor_albedo: f32,
}

/// A rectangular area light of uniform radiance, kept upright.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftBox {
    pub name: String,
    /// Toward the light's center, from the subject.
    pub direction: V3,
    /// Half-width and half-height, as tangents of the half angles the light covers.
    pub width: f32,
    pub height: f32,
    /// In cd/m^2.
    pub radiance: f32,
}

impl SoftBox {
    /// True when direction `r` (unit) sees this light.
    fn covers(&self, r: V3) -> bool {
        let d = normalize_or(self.direction, [0.0, 0.0, 1.0]);
        let facing = dot(r, d);
        if facing <= 0.0 {
            return false;
        }
        let side = normalize_or(cross([0.0, 1.0, 0.0], d), [1.0, 0.0, 0.0]);
        let up = cross(d, side);
        (dot(r, side) / facing).abs() <= self.width && (dot(r, up) / facing).abs() <= self.height
    }
}

/// A captured light probe: an equirectangular Radiance image, turned about the vertical axis.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// The image file, beside the environment file.
    pub image: String,
    /// Degrees about the vertical axis, positive turning the light to the viewer's right. Left
    /// out, it is measured: the turn that puts the probe's dominant light (the direction of its
    /// spherical-harmonic fit's first band) at the azimuth of the default studio's key light,
    /// so every environment lights a look from the same side.
    #[serde(default)]
    pub turn: Option<f32>,
    /// Who made it, its license and where it comes from.
    pub source: String,
}

/// What an environment file holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Studio(Studio),
    Probe { name: String, probe: Probe },
}

/// An environment file as written: a studio's `[room]` and `[[softbox]]`, or a `[probe]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: String,
    room: Option<Room>,
    #[serde(default, rename = "softbox")]
    softboxes: Vec<SoftBox>,
    probe: Option<Probe>,
}

impl Source {
    /// Reads and checks an environment file. An error starts with the key it is about.
    pub fn parse(text: &str) -> Result<Source, RenderError> {
        let file: File = super::parse_toml(text)?;
        match (file.room, file.probe) {
            (Some(room), None) => Studio {
                name: file.name,
                room,
                softboxes: file.softboxes,
            }
            .checked()
            .map(Source::Studio),
            (None, Some(probe)) if file.softboxes.is_empty() => Ok(Source::Probe {
                name: file.name,
                probe,
            }),
            _ => Err(RenderError::Recipe(
                "an environment is a studio ([room] and [[softbox]]) or a [probe], not both".into(),
            )),
        }
    }
}

impl Studio {
    /// Reads and checks a studio. An error starts with the key it is about.
    pub fn parse(text: &str) -> Result<Studio, RenderError> {
        match Source::parse(text)? {
            Source::Studio(studio) => Ok(studio),
            Source::Probe { .. } => Err(RenderError::Recipe("probe: not a studio".into())),
        }
    }

    /// The studio if its values are in range.
    fn checked(self) -> Result<Studio, RenderError> {
        let studio = self;
        let fail = |what: String| Err(RenderError::Recipe(what));
        let room = studio.room;
        if !(room.walls >= 0.0 && room.ceiling >= 0.0) {
            return fail("room: radiance must be 0 or more".into());
        }
        if !(0.0..=1.0).contains(&room.floor_albedo) {
            return fail(format!(
                "room.floor_albedo: {} is outside 0 to 1",
                room.floor_albedo
            ));
        }
        for b in &studio.softboxes {
            if !(b.radiance >= 0.0 && b.width > 0.0 && b.height > 0.0) {
                return fail(format!(
                    "softbox {}: radiance 0 or more, sizes above 0",
                    b.name
                ));
            }
        }
        Ok(studio)
    }

    /// Radiance along unit direction `r`, with the floor at radiance `floor`.
    fn radiance(&self, r: V3, floor: f32) -> f32 {
        if let Some(b) = self.softboxes.iter().find(|b| b.covers(r)) {
            return b.radiance;
        }
        if r[1] < 0.0 {
            return floor;
        }
        self.room.walls + (self.room.ceiling - self.room.walls) * r[1]
    }

    /// The studio as a cube, its floor lit by everything above the horizon.
    fn cube(&self) -> Cube {
        let unlit = Cube::build(32, 1, |r| [self.radiance(r, 0.0); 3]);
        let up = irradiance(&unlit.texels(0), [0.0, 1.0, 0.0]);
        let floor = self.room.floor_albedo * up[1] / PI;
        Cube::build(CUBE_SIZE, SUPERSAMPLE, |r| [self.radiance(r, floor); 3])
    }
}

/// An environment ready to light with: radiance with mips, irradiance, its SH fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    pub name: String,
    pub radiance: Cube,
    pub irradiance: Cube,
    pub sh9: Sh9,
}

impl Environment {
    /// Meters `cube` (a white diffuser facing the camera reflects 1) and derives the rest. The
    /// scale is f32's when it fits and f64's when the camera's side is near RGBE's darkest
    /// (about 2^-128) and it does not; no texel is lifted past [`BRIGHTEST`].
    pub fn from_cube(name: &str, cube: Cube) -> Environment {
        let facing = irradiance(&cube.texels(2.min(cube.levels.len() - 1)), [0.0, 0.0, 1.0]);
        let luminance = 0.2126 * facing[0] + 0.7152 * facing[1] + 0.0722 * facing[2];
        let radiance = if luminance > 0.0 {
            let k = PI / luminance;
            cube.scaled(if k.is_finite() {
                f64::from(k)
            } else {
                f64::from(PI) / f64::from(luminance)
            })
        } else {
            cube
        };
        Environment::derived(name, radiance)
    }

    /// The environment of already metered `radiance`: its irradiance map and SH fit.
    pub fn derived(name: &str, radiance: Cube) -> Environment {
        Environment {
            name: name.into(),
            irradiance: irradiance_map(&radiance),
            sh9: Sh9::project(&radiance),
            radiance,
        }
    }

    /// The environment whose radiance along each direction is `light(direction)`.
    pub fn from_light(name: &str, light: impl Fn(V3) -> Rgb + Sync) -> Environment {
        Environment::from_cube(name, Cube::build(CUBE_SIZE, 1, light))
    }

    /// A procedural studio.
    pub fn from_studio(studio: &Studio) -> Environment {
        Environment::from_cube(&studio.name, studio.cube())
    }

    /// Radiance along unit direction `d`, at mip `lod`.
    pub fn radiance(&self, d: V3, lod: f32) -> Rgb {
        self.radiance.sample(d, lod)
    }

    /// Irradiance on a surface with unit normal `n`, from the integrated map.
    pub fn irradiance(&self, n: V3) -> Rgb {
        self.irradiance.sample(n, 0.0)
    }

    /// A shipped environment by name, built once per process.
    pub fn shipped(name: &str) -> Result<Arc<Environment>, RenderError> {
        static BUILT: [OnceLock<Result<Arc<Environment>, RenderError>>; SHIPPED.len()] =
            [const { OnceLock::new() }; SHIPPED.len()];
        let at = SHIPPED
            .iter()
            .position(|(n, _)| *n == name)
            .ok_or_else(|| {
                let names: Vec<&str> = SHIPPED.iter().map(|(n, _)| *n).collect();
                RenderError::Recipe(format!(
                    "environment: there is no environment named {name} (shipped: {})",
                    names.join(", ")
                ))
            })?;
        BUILT[at]
            .get_or_init(|| {
                let built = Environment::from_source(&Source::parse(SHIPPED[at].1)?, |file| {
                    (IMAGES.iter().find(|(n, _)| *n == file)).map(|(_, bytes)| bytes.to_vec())
                });
                built.map(Arc::new)
            })
            .clone()
    }

    /// The environment `source` describes; a probe's image comes from `image(file name)`.
    pub fn from_source(
        source: &Source,
        image: impl Fn(&str) -> Option<Vec<u8>>,
    ) -> Result<Environment, RenderError> {
        match source {
            Source::Studio(studio) => Ok(Environment::from_studio(studio)),
            Source::Probe { name, probe } => {
                let bytes = image(&probe.image).ok_or_else(|| {
                    RenderError::Recipe(format!("probe.image: no image {}", probe.image))
                })?;
                let picture = super::hdr::Equirect::read_hdr(&bytes)?;
                let turn = probe
                    .turn
                    .map_or_else(|| picture.key_turn(), f32::to_radians);
                Ok(picture.environment(name, turn))
            }
        }
    }
}

/// Every shipped environment (`assets/environments/`), built in: `(name, file text)`. gs-core
/// holds the list, which a look recipe's `environment` is checked against (decision 0043).
pub use gs_core::material::ENVIRONMENTS as SHIPPED;

/// The shipped probes' images, built in: `(file name, Radiance file)`. CC0, from Poly Haven
/// (`assets/environments/CREDITS`).
const IMAGES: [(&str, &[u8]); 3] = [
    (
        "studio_small_09_1k.hdr",
        include_bytes!("../../../../assets/environments/studio_small_09_1k.hdr"),
    ),
    (
        "studio_small_08_1k.hdr",
        include_bytes!("../../../../assets/environments/studio_small_08_1k.hdr"),
    ),
    (
        "photo_studio_01_1k.hdr",
        include_bytes!("../../../../assets/environments/photo_studio_01_1k.hdr"),
    ),
];
