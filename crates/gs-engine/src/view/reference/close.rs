//! `reference.close` and `reference.list`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::{ReferenceShown, name, one_open, shown};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Close a Reference View: its view, its camera history and its scene go. The sculpt does not
/// change. Logged; never undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceClose {
    /// The reference: "ref:1" ... Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
}

/// What `reference.close` closed.
#[derive(Serialize, JsonSchema)]
pub struct Closed {
    /// Its name.
    pub view: String,
    /// The references still open, by number.
    pub open: Vec<String>,
}

impl Command for ReferenceClose {
    const NAME: &'static str = "reference.close";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Reference;
    type Output = Closed;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("close the active reference", json!({}))
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true })),
            Example::raw("close one of two by name", json!({ "view": "ref:2" }))
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true }))
                .after("reference.open", json!({ "parts": "sphere" })),
            Example::raw("a reference that is not open", json!({ "view": "ref:4" }))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        one_open(self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Closed, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        cx.views.close_reference(&view);
        Ok(Closed {
            view,
            open: cx.views.references(),
        })
    }
}
register_command!(ReferenceClose, "Close reference");

/// List every open Reference View, by number: its camera, where its scene came from, the parts
/// it shows, what follows main, its alignment and its look. The window shows them all;
/// `session.state`'s `reference` names the active one. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceList {}

impl Command for ReferenceList {
    const NAME: &'static str = "reference.list";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Vec<ReferenceShown>;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("every open reference", Self {})
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Vec<ReferenceShown>, CommandError> {
        (cx.views.references().iter())
            .map(|view| shown(cx, view))
            .collect()
    }
}
register_command!(ReferenceList, "List references");
