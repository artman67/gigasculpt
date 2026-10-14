//! `mask.set`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{MaskReport, ScopeArg};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, errors, register_command};

/// Set the mask of every visible point to one value: 1 masks the whole part (Mask All), 0
/// clears the mask (Clear Mask).
///
/// Hidden points keep their mask unless `scope` is `all`. Clearing releases the mask's memory
/// where nothing is left masked. On a part with detail levels it acts on the top level from
/// any level, so it means the same at every level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskSet {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The mask value, 0 (free) to 1 (fully masked).
    #[schemars(range(min = 0.0, max = 1.0))]
    pub value: f32,
    /// Which points: `visible` (the default) or `all`.
    #[serde(default, skip_serializing_if = "ScopeArg::is_visible")]
    pub scope: ScopeArg,
}

impl Command for MaskSet {
    const NAME: &'static str = "mask.set";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = MaskReport;

    fn examples() -> Vec<Example<Self>> {
        let all = Self {
            part: None,
            value: 1.0,
            scope: ScopeArg::Visible,
        };
        let clear = Self {
            part: None,
            value: 0.0,
            scope: ScopeArg::All,
        };
        vec![
            Example::new("mask every visible point", all).after("primitive.sphere", json!({})),
            Example::new("clear the mask everywhere, hidden points too", clear)
                .after("primitive.sphere", json!({}))
                .after("mask.set", json!({ "value": 1 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaskReport, CommandError> {
        let part = pinned(self.part)?;
        if !(0.0..=1.0).contains(&self.value) {
            return Err(errors::args_invalid("/value", "value must be from 0 to 1"));
        }
        cx.noop_reason(format!(
            "every point in scope already had mask {}",
            self.value
        ));
        let (value, scope) = (self.value, self.scope.core());
        crate::level::whole_part(cx, &part, |s| s.mask_set(value, scope))?;
        Ok(MaskReport::of(&part, cx.doc.get_part(&part)?.store()))
    }
}
register_command!(MaskSet, "Set mask");
