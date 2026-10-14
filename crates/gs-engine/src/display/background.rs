//! The viewport's background (catalog item `canvas.background`): `view.background` sets it,
//! and a scene file keeps it, the one display setting saved with the scene (decision 0023 as
//! amended for F18).

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::color;
use crate::{Command, Ctx, Example, register_command};

/// The color behind the parts, the gradient over it, and the border colors.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Background {
    /// The color behind the parts, `"#rrggbb"`.
    #[serde(with = "color::as_hex")]
    #[schemars(with = "String")]
    pub color: [u8; 3],
    /// How much of a black-to-white gradient, white at the top, lies over the color: 0 none
    /// (the flat color) to 1 (the gradient alone).
    pub gradient: f32,
    /// How far down the view the gradient is half white: 0 the top, 1 the bottom.
    pub center: f32,
    /// How soft the gradient is: 0 a hard edge, 1 spread over the view's whole height.
    pub softness: f32,
    /// The color around a fixed-size canvas, `"#rrggbb"`. Kept, but drawn nowhere yet: no view
    /// has a fixed-size canvas until catalog item `canvas.document`.
    #[serde(with = "color::as_hex")]
    #[schemars(with = "String")]
    pub border: [u8; 3],
    /// The second border color, blended into the first for a two-tone frame; the same color
    /// makes a flat one.
    #[serde(with = "color::as_hex")]
    #[schemars(with = "String")]
    pub border2: [u8; 3],
}

/// Today's background color, gs-render's `BACKGROUND` (catalog item `canvas.background`: the
/// spec's #3a3a3a waits for the maintainer).
const GRAY: [u8; 3] = [40, 42, 46];

impl Default for Background {
    /// The flat dark gray every view drew before the setting, with no gradient (the spec's 0.3
    /// would change that look, so it waits with the color); the gradient's center and
    /// softness at the spec's 0.5; both borders the same gray.
    fn default() -> Self {
        Background {
            color: GRAY,
            gradient: 0.0,
            center: 0.5,
            softness: 0.5,
            border: GRAY,
            border2: GRAY,
        }
    }
}

impl Background {
    /// Its values by the names `session.state` gives them.
    pub(crate) fn values(&self) -> [(&'static str, Value); 6] {
        [
            ("view.background.color", json!(color::hex(self.color))),
            ("view.background.gradient", json!(self.gradient)),
            ("view.background.center", json!(self.center)),
            ("view.background.softness", json!(self.softness)),
            ("view.background.border", json!(color::hex(self.border))),
            ("view.background.border2", json!(color::hex(self.border2))),
        ]
    }

    /// The background a scene file keeps, if it keeps one this version reads: read apart, so
    /// one it cannot read never costs the rest of the file's scene section.
    pub(crate) fn saved(value: Option<Value>) -> Option<Background> {
        value.and_then(|v| serde_json::from_value(v).ok())
    }
}

/// Set the viewport's background. Arguments left out keep their value.
///
/// The color behind the parts, with a vertical gradient over it: black to white, white at the
/// top, as strong as `gradient`, half white at `center`, as soft as `softness`. Every view
/// draws it, the window's and snapshots alike. The border colors frame a fixed-size canvas,
/// which no view has yet: they are kept and saved, and draw nothing. Unlike the other display
/// settings, the background is saved with the scene and comes back when the scene opens (a
/// file saved without one keeps the session's). Never undone; changing it is no unsaved
/// change.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewBackground {
    /// The color behind the parts, `"#rrggbb"`. Starts at `"#282a2e"`, dark gray.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = color::PATTERN))]
    pub color: Option<String>,
    /// How much of the gradient lies over the color, 0 to 1. Starts at 0: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub gradient: Option<f32>,
    /// How far down the view the gradient is half white, 0 the top to 1 the bottom. Starts at
    /// 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub center: Option<f32>,
    /// How soft the gradient is, 0 a hard edge to 1 spread over the view's height. Starts at
    /// 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub softness: Option<f32>,
    /// The color around a fixed-size canvas, `"#rrggbb"`. Starts at `"#282a2e"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = color::PATTERN))]
    pub border: Option<String>,
    /// The second border color, blended into the first; the same color, no blend. Starts at
    /// `"#282a2e"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = color::PATTERN))]
    pub border2: Option<String>,
}

impl Command for ViewBackground {
    const NAME: &'static str = "view.background";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = Background;

    fn examples() -> Vec<Example<Self>> {
        let gray = Self {
            color: Some("#3a3a3a".into()),
            ..Self::default()
        };
        let gradient = Self {
            gradient: Some(0.3),
            center: Some(0.5),
            softness: Some(0.5),
            ..Self::default()
        };
        vec![
            Example::new("a flat middle gray", gray),
            Example::new("a soft gradient, lighter above", gradient),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Background, CommandError> {
        let hex = |path: &str, given: Option<&str>, old: [u8; 3]| {
            given.map_or(Ok(old), |c| color::parse(path, c))
        };
        let b = &mut cx.display.background;
        b.color = hex("/color", self.color.as_deref(), b.color)?;
        b.border = hex("/border", self.border.as_deref(), b.border)?;
        b.border2 = hex("/border2", self.border2.as_deref(), b.border2)?;
        b.gradient = self.gradient.unwrap_or(b.gradient);
        b.center = self.center.unwrap_or(b.center);
        b.softness = self.softness.unwrap_or(b.softness);
        Ok(*b)
    }
}
register_command!(ViewBackground, "Set background");
