//! `mask.invert`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{MaskReport, ScopeArg};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// Invert the mask: every visible point's mask `m` becomes `1 - m`, so what was protected is
/// free and the other way round. Inverting twice gives back the same mask exactly.
///
/// Hidden points keep their mask unless `scope` is `all`. On a part with detail levels it acts
/// on the top level from any level, so it means the same at every level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskInvert {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Which points: `visible` (the default) or `all`.
    #[serde(default, skip_serializing_if = "ScopeArg::is_visible")]
    pub scope: ScopeArg,
}

impl Command for MaskInvert {
    const NAME: &'static str = "mask.invert";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = MaskReport;

    fn examples() -> Vec<Example<Self>> {
        let visible = Self {
            part: None,
            scope: ScopeArg::Visible,
        };
        vec![
            Example::new("invert the mask of a sphere", visible)
                .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaskReport, CommandError> {
        let part = pinned(self.part)?;
        cx.noop_reason("the part has no point in scope");
        let scope = self.scope.core();
        crate::level::whole_part(cx, &part, |s| s.mask_invert(scope))?;
        Ok(MaskReport::of(&part, cx.doc.get_part(&part)?.store()))
    }
}
register_command!(MaskInvert, "Invert mask");
