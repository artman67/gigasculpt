//! `part.set_paint_visible`: Show Paint, each part's switch for its paint (decision 0039,
//! section 10). Off, the part draws in its material alone; its colors stay as they were, so
//! turning it on again shows exactly what was painted.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::pin_part;
use super::visible::change;
use crate::{Command, Ctx, Example, errors, register_command};

/// Show or hide the paint of a part, or of every part (Show Paint).
///
/// Hidden, the part draws in its material alone, as if never painted. Its colors stay: painting
/// and filling still change them and an export still writes them. A part never painted stores
/// nothing more. One undo step, also for every part at once, and the scene file keeps it.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartSetPaintVisible {
    #[doc = crate::command::part_doc!("The part. Not used with `every`.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Paint shows (true) or is hidden.
    pub visible: bool,
    /// Set it on every part instead, shown or not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub every: bool,
}

/// The parts whose paint switch changed.
#[derive(Serialize, JsonSchema)]
pub struct PaintShown {
    /// Their names, in list order.
    pub changed: Vec<String>,
}

impl Command for PartSetPaintVisible {
    const NAME: &'static str = "part.set_paint_visible";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const CLAIMS: &'static str = r#"["preserves_topology", "positions_only"]"#;
    type Output = PaintShown;

    fn examples() -> Vec<Example<Self>> {
        let two = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
        };
        vec![
            two(Example::raw(
                "hide the active part's paint",
                json!({ "visible": false }),
            )),
            two(Example::raw(
                "hide every part's paint",
                json!({ "visible": false, "every": true }),
            )),
            two(Example::raw(
                "show the sphere's paint again",
                json!({ "part": "sphere", "visible": true }),
            ))
            .after(
                "part.set_paint_visible",
                json!({ "visible": false, "every": true }),
            ),
            two(Example::raw(
                "a part and every part at once",
                json!({ "part": "cube", "visible": false, "every": true }),
            ))
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        if self.every && self.part.is_some() {
            return Err(errors::args_invalid("/part", "every takes no part"));
        }
        if self.every {
            return Ok(());
        }
        pin_part(cx, &mut self.part)
    }

    fn run(self, cx: &mut Ctx) -> Result<PaintShown, CommandError> {
        let part = self.part.map(|PartRef(name)| name);
        let targets: Vec<String> = (cx.doc.list_names().into_iter())
            .filter(|name| self.every || part.as_deref() == Some(name))
            .collect();
        let hidden = !self.visible;
        let changed = change(cx, &targets, |_, props| props.paint_hidden = hidden)?;
        if changed.is_empty() {
            let how = if self.visible { "shows" } else { "is hidden" };
            cx.noop_reason(format!(
                "the paint of every one of those parts already {how}"
            ));
        }
        Ok(PaintShown { changed })
    }
}
register_command!(PartSetPaintVisible, "Show paint");
