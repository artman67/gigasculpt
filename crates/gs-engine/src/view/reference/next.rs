//! `reference.next`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::{ReferenceShown, name, one_open, shown};
use crate::command::ViewArg;
use crate::errors;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Make the next open Reference View active, by number, wrapping around after the last. The
/// active reference is the one the `reference.*` commands act on when `view` is left out, and
/// the one whose controls the window's reference header shows; the window shows every open
/// reference. Logged; never undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceNext {
    /// The reference to move on from: "ref:1" ... Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
}

impl Command for ReferenceNext {
    const NAME: &'static str = "reference.next";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Reference;
    type Output = ReferenceShown;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("from the reference opened last to the first", json!({}))
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true }))
                .after("reference.open", json!({ "parts": "sphere" })),
            Example::raw("with one open, it stays active", json!({}))
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true })),
            Example::raw("none open", json!({})).fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        one_open(self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ReferenceShown, CommandError> {
        let from = name(self.view.as_ref())?;
        let next = (cx.views.next_reference(from))
            .ok_or_else(|| errors::internal("the reference to move on from is not open"))?;
        shown(cx, &next)
    }
}
register_command!(ReferenceNext, "Next reference");
