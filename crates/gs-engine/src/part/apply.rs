//! `part.apply_transform` (decision 0029, section 5): a part's place in the scene written into
//! its points.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, report};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// Write a part's move, turn and scale into its points, every detail level's, and put the part
/// back at its own origin with the scene axes. What the part looks like and where it is stay
/// exactly the same; afterwards its symmetry planes are the scene's planes through the scene
/// origin. One undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartApplyTransform {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for PartApplyTransform {
    const NAME: &'static str = "part.apply_transform";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}}]"#;
    // Changes something only on a part a routed `transform.apply` moved first, which random
    // plans reach too rarely to count on: its claims are checked on plans that start by
    // turning each part whole (unmasked, so the turn goes to the part's transform).
    const SETUP: &'static str = r#"[{"command": "transform.apply", "args": {
        "op": {"rotate": {"axis": [1, 2, 3], "degrees": 35}}, "mask": false
    }}]"#;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let moved = json!({ "op": { "move": [0.5, 0.0, 0.0] } });
        let turned = json!({ "op": { "rotate": { "axis": "z", "degrees": 30 } } });
        vec![
            Example::raw("write a moved sphere's place into its points", json!({}))
                .after("primitive.sphere", json!({}))
                .after("transform.apply", moved),
            Example::raw(
                "write a turned cube's turn into its points",
                json!({ "part": "cube" }),
            )
            .after("primitive.cube", json!({}))
            .after("transform.apply", turned),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        if !cx.doc.bake(&part)? {
            cx.noop_reason(format!(
                "{part} is already at its own origin, unturned and unscaled"
            ));
        }
        report(cx, &part)
    }
}
register_command!(PartApplyTransform, "Apply transform");
