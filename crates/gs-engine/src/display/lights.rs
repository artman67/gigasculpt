//! `light.set`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{DisplaySettings, LIGHTS, LightSetting};
use crate::errors::args_invalid;
use crate::{Command, Ctx, Example, register_command};

/// Change a light of Lit mode, or Ambient: what lights `lit`, `clay` and `zebra` shading.
///
/// Light 1 is the environment's key light, taken out of it so it can be moved, recolored or
/// turned off; lights 2 to 8 are extra lights, off until turned on. Lights are fixed to the
/// camera, as the environment is: a direction is in view space (x right, y up, z toward the
/// viewer). Ambient scales the rest of the environment. Values left out keep theirs; a value
/// never set follows the environment (light 1) or the default (white, intensity 1, from the
/// upper left). A display setting of the session: never undone, never saved with the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LightSet {
    /// Which light, 1 to 8. Left out: only `ambient` changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 8))]
    pub light: Option<u32>,
    /// Turn it on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Linear RGB, each 0 to 1000 and not all 0; used at luminance 1 (intensity sets the
    /// brightness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-rgb" = true))]
    pub color: Option<[f32; 3]>,
    /// 0 to 1000: 1 lights a white card facing the light to reflect 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1000.0))]
    pub intensity: Option<f32>,
    /// Toward the light in view space (x right, y up, z toward the viewer), any length but 0;
    /// kept at length 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f32; 3]>,
    /// The light's angular radius in degrees, 0.01 to 90: larger is softer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 90.0))]
    pub size_deg: Option<f32>,
    /// Back to the light's start (light 1: on, as the environment has it; others: off), before
    /// any other value given here.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    /// 0 to 16: scales the environment's light besides light 1. Starts at 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 16.0))]
    pub ambient: Option<f32>,
}

impl LightSet {
    fn light_only(&self) -> bool {
        self.on.is_some()
            || self.color.is_some()
            || self.intensity.is_some()
            || self.direction.is_some()
            || self.size_deg.is_some()
            || self.reset
    }

    /// The values that need more than the schema's ranges.
    fn checked(&self) -> Result<(), CommandError> {
        if self.light.is_none() && self.light_only() {
            return Err(args_invalid("/light", "say which light, 1 to 8"));
        }
        if let Some(c) = self.color {
            let valid = c.iter().all(|v| (0.0..=1000.0).contains(v));
            if !valid || c.iter().all(|v| *v == 0.0) {
                let message = "each channel 0 to 1000, not all 0 (intensity 0 turns it dark)";
                return Err(args_invalid("/color", message));
            }
        }
        if self.direction.is_some_and(|d| unit(d).is_none()) {
            return Err(args_invalid(
                "/direction",
                "a direction needs a length, not 0",
            ));
        }
        Ok(())
    }
}

/// `d` at length 1, or `None` when it has no length. In f64, where no f32 vector's squared
/// length overflows or underflows, so a direction of any length but 0 points where it says
/// (review 0005, m5; `mesh.translate` measures its direction the same way).
fn unit(d: [f32; 3]) -> Option<[f32; 3]> {
    let d = d.map(f64::from);
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (length.is_finite() && length > 0.0).then(|| d.map(|c| (c / length) as f32))
}

impl Command for LightSet {
    const NAME: &'static str = "light.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = DisplaySettings;

    fn examples() -> Vec<Example<Self>> {
        let call = |light: Option<u32>| Self {
            light,
            on: None,
            color: None,
            intensity: None,
            direction: None,
            size_deg: None,
            reset: false,
            ambient: None,
        };
        let rim = Self {
            on: Some(true),
            color: Some([0.6, 0.8, 1.0]),
            intensity: Some(2.0),
            direction: Some([0.3, 0.4, -1.0]),
            ..call(Some(2))
        };
        let off = Self {
            on: Some(false),
            ..call(Some(1))
        };
        let dim = Self {
            ambient: Some(0.25),
            ..call(None)
        };
        let unnamed = Self {
            on: Some(true),
            ..call(None)
        };
        vec![
            Example::new("a cool rim light from behind on the right", rim),
            Example::new("the key light off: only the environment lights", off),
            Example::new("a quarter of the environment's light", dim),
            Example::new("a light value needs a light", unnamed)
                .fails_with(gs_protocol::ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<DisplaySettings, CommandError> {
        self.checked()?;
        let d = &mut *cx.display;
        if let Some(n) = self.light {
            let i = (n as usize)
                .checked_sub(1)
                .filter(|i| *i < LIGHTS)
                .ok_or_else(|| args_invalid("/light", "lights are 1 to 8"))?;
            let l = &mut d.lights[i];
            if self.reset {
                *l = LightSetting {
                    on: i == 0,
                    ..LightSetting::default()
                };
            }
            l.on = self.on.unwrap_or(l.on);
            l.color = self.color.or(l.color);
            l.intensity = self.intensity.or(l.intensity);
            l.direction = self.direction.and_then(unit).or(l.direction);
            l.size_deg = self.size_deg.or(l.size_deg);
        }
        if let Some(ambient) = self.ambient {
            d.ambient = ambient;
        }
        Ok(d.clone())
    }
}
register_command!(LightSet, "Set light");
