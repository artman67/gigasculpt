//! `pen.set`: the pen's feel, how this pen in this hand reaches every brush (decision 0036,
//! section 3 as amended): Size, Strength and Color sensitivity and lazy pressure. They last the
//! session, as the global pressure curves do, and `brush.stroke` never reads them: a stroke
//! whose session feel is not the default carries it as its `pen`, so the log replays without
//! them.

use gs_core::stroke::PenFeel;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Command, Ctx, Example, errors, register_command};

/// The pen's feel. The sensitivities are percentages: 100 is the pressure curve as it is, 0
/// leaves pressure out, and below 0 the curve runs backward.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PenSettings {
    /// How much of the size pressure curve strokes keep, -100 to 100. With no size curve on,
    /// it has nothing to scale.
    #[schemars(range(min = -100.0, max = 100.0))]
    pub size_sensitivity: f32,
    /// How much of the strength pressure curve strokes keep, -100 to 100.
    #[schemars(range(min = -100.0, max = 100.0))]
    pub strength_sensitivity: f32,
    /// How much of the strength pressure curve paint keeps, -100 to 100: at 0 a brush paints
    /// the same color at every pressure.
    #[schemars(range(min = -100.0, max = 100.0))]
    pub color_sensitivity: f32,
    /// How slowly pressure follows the pen, 0 to 1: each sample's pressure moves `1 -
    /// lazy_pressure` of the way from the last one's toward the pen's.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub lazy_pressure: f32,
}

impl Default for PenSettings {
    /// Every sensitivity 100 and lazy pressure 0: pressure as the curves give it.
    fn default() -> Self {
        Self {
            size_sensitivity: 100.0,
            strength_sensitivity: 100.0,
            color_sensitivity: 100.0,
            lazy_pressure: 0.0,
        }
    }
}

impl PenSettings {
    /// The feel as gs-core takes it; `E_ARGS_INVALID` under `path` for a value out of range.
    pub(crate) fn core(&self, path: &str) -> Result<PenFeel, CommandError> {
        let sensitivities = [
            ("size_sensitivity", self.size_sensitivity),
            ("strength_sensitivity", self.strength_sensitivity),
            ("color_sensitivity", self.color_sensitivity),
        ];
        for (name, k) in sensitivities {
            if !(-100.0..=100.0).contains(&k) {
                let message = format!("{name} must be from -100 to 100, not {k}");
                return Err(errors::args_invalid(&format!("{path}/{name}"), message));
            }
        }
        if !(0.0..=1.0).contains(&self.lazy_pressure) {
            let message = format!(
                "lazy_pressure must be from 0 to 1, not {}",
                self.lazy_pressure
            );
            return Err(errors::args_invalid(
                &format!("{path}/lazy_pressure"),
                message,
            ));
        }
        Ok(PenFeel {
            size: self.size_sensitivity / 100.0,
            strength: self.strength_sensitivity / 100.0,
            color: self.color_sensitivity / 100.0,
            lazy: self.lazy_pressure,
        })
    }

    /// True at the defaults, where strokes are as their curves give them.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Change the pen's feel for every brush: Size, Strength and Color sensitivity (-100 to 100,
/// each 100 at first) and lazy pressure (0 to 1, 0 at first). A sensitivity scales how far its
/// pressure curve falls below full: 100 is the curve as it is, 50 half as far, 0 leaves
/// pressure out, and below 0 the curve runs backward, so a firmer touch shrinks what it grew.
/// Leave a setting out to keep it. Strokes carry the feel as their `pen`, so the log replays
/// without it. Answers with all four.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PenSet {
    /// How much of the size pressure curve strokes keep, -100 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub size_sensitivity: Option<f32>,
    /// How much of the strength pressure curve strokes keep, -100 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub strength_sensitivity: Option<f32>,
    /// How much of the strength pressure curve paint keeps, -100 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub color_sensitivity: Option<f32>,
    /// How slowly pressure follows the pen, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub lazy_pressure: Option<f32>,
}

impl Command for PenSet {
    const NAME: &'static str = "pen.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = PenSettings;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "pressure changes the sculpt half as much",
                json!({ "strength_sensitivity": 50 }),
            ),
            Example::raw(
                "paint the same color at any pressure",
                json!({ "color_sensitivity": 0 }),
            ),
            Example::raw(
                "the brush shrinks to half as pressure rises",
                json!({ "size_sensitivity": -50 }),
            ),
            Example::raw("steady a jittery pen", json!({ "lazy_pressure": 0.5 })),
            Example::raw("more than full", json!({ "size_sensitivity": 150 }))
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<PenSettings, CommandError> {
        let mut next = cx.sculpt.settings.pen;
        let given = [
            (&mut next.size_sensitivity, self.size_sensitivity),
            (&mut next.strength_sensitivity, self.strength_sensitivity),
            (&mut next.color_sensitivity, self.color_sensitivity),
            (&mut next.lazy_pressure, self.lazy_pressure),
        ];
        for (field, value) in given {
            *field = value.unwrap_or(*field);
        }
        next.core("")?;
        cx.sculpt.settings.pen = next;
        Ok(next)
    }
}
register_command!(PenSet, "Set pen feel");
