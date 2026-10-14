//! `color.set`: the session's Main and Secondary colors and its Gradient and Alternate switches,
//! which the window's strokes paint with and `paint.fill` fills with (decision 0039, sections 7
//! and 11), and the swap of the two colors. Like the brush settings they are not saved with the
//! scene and not undone, and `brush.stroke` never reads them: a stroke with a brush that paints
//! carries its own `colors`, so the log replays without them.

use gs_core::PaintColors;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Command, Ctx, Example, errors, register_command};

/// The colors a brush that paints uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    /// What the brush paints, `"#rrggbb"` in sRGB.
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub main: String,
    /// What it paints with Alt held, `"#rrggbb"` in sRGB.
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub secondary: String,
    /// Gradient: paint Main at the brush's centre and Secondary at its rim, mixed by the
    /// brush's falloff in linear light (Alt swaps the two).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gradient: bool,
    /// Alternate: Alt swaps Main and Secondary. Off, Alt does nothing for color: the brush
    /// paints Main. Left out: on.
    #[serde(default = "crate::command::yes", skip_serializing_if = "Clone::clone")]
    pub alternate: bool,
}

impl Default for Colors {
    /// White and black, without Gradient, with Alternate.
    fn default() -> Self {
        Self {
            main: "#ffffff".into(),
            secondary: "#000000".into(),
            gradient: false,
            alternate: true,
        }
    }
}

impl Colors {
    /// The colors as gs-core takes them; `E_ARGS_INVALID` under `path` for a color that is
    /// not `#rrggbb`.
    pub(crate) fn core(&self, path: &str) -> Result<PaintColors, CommandError> {
        Ok(PaintColors {
            main: word(&self.main, &format!("{path}/main"))?,
            secondary: word(&self.secondary, &format!("{path}/secondary"))?,
            gradient: self.gradient,
            alternate: self.alternate,
        })
    }
}

/// `"#rrggbb"` as an opaque color word, red in its lowest byte as the color column holds it;
/// `E_ARGS_INVALID` under `path` for anything else.
pub(crate) fn word(hex: &str, path: &str) -> Result<u32, CommandError> {
    let digits = (hex.strip_prefix('#'))
        .filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_hexdigit()));
    let Some(rgb) = digits.and_then(|d| u32::from_str_radix(d, 16).ok()) else {
        let message = format!("{hex:?} is not a color written #rrggbb");
        return Err(errors::args_invalid(path, message));
    };
    Ok(0xff00_0000 | (rgb >> 16) | (rgb & 0xff00) | ((rgb & 0xff) << 16))
}

/// Change the colors the window's brushes paint with: Main, Secondary (what Alt paints),
/// Gradient and Alternate, or swap Main and Secondary. Leave one out to keep it. A stroke with
/// a brush that paints carries the colors it used, so the log replays without these. Answers
/// with them all.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ColorSet {
    /// Swap Main and Secondary (Swap Colors) before setting what this call gives.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub swap: bool,
    /// What brushes paint, `"#rrggbb"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub main: Option<String>,
    /// What they paint with Alt held, `"#rrggbb"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub secondary: Option<String>,
    /// Turn Gradient on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<bool>,
    /// Turn Alternate on or off: off, Alt does nothing for color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate: Option<bool>,
}

impl Command for ColorSet {
    const NAME: &'static str = "color.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = Colors;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("paint red", json!({ "main": "#ff0000" })),
            Example::raw("Alt paints dark blue", json!({ "secondary": "#1a2b80" })),
            Example::raw(
                "Main at the brush's centre, Secondary at its rim",
                json!({ "gradient": true }),
            ),
            Example::raw("swap Main and Secondary", json!({ "swap": true })),
            Example::raw("Alt paints Main too", json!({ "alternate": false })),
            Example::raw("a color by name", json!({ "main": "red" }))
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Colors, CommandError> {
        let mut next = cx.sculpt.settings.colors.clone();
        if self.swap {
            std::mem::swap(&mut next.main, &mut next.secondary);
        }
        if let Some(main) = self.main {
            next.main = main.to_ascii_lowercase();
        }
        if let Some(secondary) = self.secondary {
            next.secondary = secondary.to_ascii_lowercase();
        }
        next.gradient = self.gradient.unwrap_or(next.gradient);
        next.alternate = self.alternate.unwrap_or(next.alternate);
        next.core("")?;
        cx.sculpt.settings.colors = next.clone();
        Ok(next)
    }
}
register_command!(ColorSet, "Set paint colors");
