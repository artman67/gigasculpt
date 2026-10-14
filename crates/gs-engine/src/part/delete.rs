//! `part.delete` (decision 0028, section 3).

use gs_protocol::{CommandError, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::pin_part;
use crate::{Command, Ctx, Example, errors, register_command};

/// Which parts `part.delete` removes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The part.
    #[default]
    Part,
    /// Every part but it.
    Others,
    /// Every part.
    All,
    /// Every part whose eye is off that does not show: Isolate did not choose it and it is not
    /// the active part.
    Hidden,
}

/// Delete a part, every other part, every part, or every hidden part.
///
/// One undo step brings them all back. Ids of deleted parts are never handed out again. When the
/// active part goes, the part after it in the list becomes active, else the one before it.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartDelete {
    /// The part, for scopes `part` and `others`. Defaults to the active part. Not used by the
    /// other scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// What to delete.
    #[serde(default)]
    pub scope: Scope,
}

/// The parts that were deleted.
#[derive(Serialize, JsonSchema)]
pub struct Deleted {
    /// Their names, in list order.
    pub parts: Vec<String>,
}

impl Command for PartDelete {
    const NAME: &'static str = "part.delete";
    const KIND: Kind = Kind::Edit;
    type Output = Deleted;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("delete the only part", json!({})).after("primitive.sphere", json!({})),
            Example::raw(
                "delete every part but the cube",
                json!({ "part": "cube", "scope": "others" }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({})),
            Example::raw("delete every hidden part", json!({ "scope": "hidden" }))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
                .after(
                    "part.set_visible",
                    json!({ "parts": ["sphere"], "visible": false }),
                ),
            Example::raw("delete every part", json!({ "scope": "all" }))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({})),
            Example::raw("a part that does not exist", json!({ "part": "head" }))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::NotFound),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        match self.scope {
            Scope::Part | Scope::Others => pin_part(cx, &mut self.part),
            Scope::All | Scope::Hidden if self.part.is_some() => Err(errors::args_invalid(
                "/part",
                "scopes all and hidden take no part",
            )),
            Scope::All | Scope::Hidden => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<Deleted, CommandError> {
        let part = self.part.map(|PartRef(name)| name);
        let is_part = |name: &str| part.as_deref() == Some(name);
        let doc = cx.doc.document();
        let showing = doc.showing();
        let parts: Vec<String> = (doc.list().into_iter())
            .filter(|(name, p)| match self.scope {
                Scope::Part => is_part(name),
                Scope::Others => !is_part(name),
                Scope::All => true,
                // Its eye is off and it does not show: neither isolated nor active.
                Scope::Hidden => p.props().hidden && !p.shown(showing),
            })
            .map(|(name, _)| name.to_string())
            .collect();
        if parts.is_empty() {
            cx.noop_reason(match self.scope {
                Scope::Hidden => "no part is hidden",
                Scope::All => "the scene has no parts",
                Scope::Part | Scope::Others => "there are no other parts",
            });
        }
        for name in &parts {
            cx.doc.remove_part(name);
        }
        Ok(Deleted { parts })
    }
}
register_command!(PartDelete, "Delete part");
