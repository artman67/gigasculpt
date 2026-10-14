//! `paint.fill`: Fill Color (decision 0039, section 11), the paint stroke's mix and rounding
//! over every point of a part at once (gs-core `MeshStore::fill_color`).

use gs_protocol::{CommandError, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::part::{pin_part, shown};
use crate::sculpt::word;
use crate::{Command, CommandRng, Ctx, Example, errors, register_command};

/// Fill a part, or every part that shows, with one color (Fill Color).
///
/// Each point goes from its color toward `color` by `strength` times how free its mask leaves
/// it, mixed in linear light as a paint stroke mixes, and is rounded once with a dither from
/// `seed`; at full strength a free point takes the color exactly. Masked points and points
/// whose faces are all hidden keep their color. On a part with detail levels it fills the top
/// level from any level. One undo step whatever the number of parts.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaintFill {
    #[doc = crate::command::part_doc!("The part. Not used with `every`.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Fill every part that shows instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub every: bool,
    /// The color, `"#rrggbb"` in sRGB. Defaults to the session's Main (`color.set`), written
    /// into the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^#[0-9a-fA-F]{6}$"))]
    pub color: Option<String>,
    /// Color Strength, 0 to 1: how far toward `color` a free point goes.
    #[serde(default = "full")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub strength: f32,
    /// The seed of the rounding's dither. Without one, one is derived from the session seed
    /// and the command's sequence number, and written into the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

fn full() -> f32 {
    1.0
}

/// What a fill did.
#[derive(Serialize, JsonSchema)]
pub struct Filled {
    /// The parts whose colors changed, in list order.
    pub parts: Vec<String>,
    /// The color, `"#rrggbb"`.
    pub color: String,
    /// The seed used.
    pub seed: u64,
}

impl Command for PaintFill {
    const NAME: &'static str = "paint.fill";
    const KIND: Kind = Kind::Edit;
    const RANDOM: bool = true;
    const CLAIMS: &'static str = r#"[{"mask_holds": {}}, "hidden_holds"]"#;
    type Output = Filled;

    fn examples() -> Vec<Example<Self>> {
        let sphere = |e: Example<Self>| e.after("primitive.sphere", json!({}));
        vec![
            sphere(Example::raw(
                "fill the active part with red",
                json!({ "color": "#ff0000", "seed": 1 }),
            )),
            sphere(Example::raw(
                "fill with the session's Main",
                json!({ "seed": 1 }),
            ))
            .after("color.set", json!({ "main": "#2050c0" })),
            sphere(Example::raw(
                "tint every part that shows a quarter of the way to brown",
                json!({ "every": true, "color": "#604020", "strength": 0.25, "seed": 1 }),
            ))
            .after("primitive.cube", json!({})),
            sphere(Example::raw(
                "a color by name",
                json!({ "color": "red", "seed": 1 }),
            ))
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        if self.color.is_none() {
            self.color = Some(cx.sculpt.settings.colors.main.clone());
        }
        if self.every && self.part.is_some() {
            return Err(errors::args_invalid("/part", "every takes no part"));
        }
        if self.every {
            return Ok(());
        }
        pin_part(cx, &mut self.part)
    }

    fn run(self, cx: &mut Ctx) -> Result<Filled, CommandError> {
        let color =
            (self.color).ok_or_else(|| errors::internal("the engine did not pin the color"))?;
        let word = word(&color, "/color")?;
        if !(0.0..=1.0).contains(&self.strength) {
            return Err(errors::args_invalid(
                "/strength",
                "strength must be from 0 to 1",
            ));
        }
        let seed =
            (self.seed).ok_or_else(|| errors::internal("the engine did not pin the seed"))?;
        let targets = match self.part {
            Some(PartRef(name)) => vec![name],
            None => shown(cx),
        };
        cx.noop_reason("every point it reaches already has that color, or is masked or hidden");
        let (strength, rng) = (self.strength, CommandRng::from_seed(seed));
        let mut parts = Vec::new();
        for name in targets {
            let mut wrote = false;
            crate::level::whole_part(cx, &name, |s| {
                let nodes = s.fill_color(word, strength, rng);
                wrote = !nodes.is_empty();
                nodes
            })?;
            if wrote {
                parts.push(name);
            }
        }
        let color = color.to_ascii_lowercase();
        Ok(Filled { parts, color, seed })
    }
}
register_command!(PaintFill, "Fill color");
