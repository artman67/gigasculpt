//! `view.get`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::super::views::ViewInfo;
use super::name;
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, register_command};

/// Read a view: its exact camera, the size it is seen at, whether it is locked, its orbit mode,
/// its pivot rule and the point that gives now, its last edit point, and how far its camera
/// history reaches.
///
/// Views belong to whoever looks: `main` is the window's, and each agent has its own
/// (`agent:<name>`), made from `main` the first time it is used. Pass the reported `camera` to
/// a look's `view` or to `view.set` to get exactly this camera. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewGet {
    /// The view: "main", or "agent:<name>". Left out: yours (main for the human at the window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
}

impl Command for ViewGet {
    const NAME: &'static str = "view.get";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = ViewInfo;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("your own view", json!({})),
            Example::raw("the window's view", json!({ "view": "main" }))
                .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<ViewInfo, CommandError> {
        let view = name(self.view.as_ref())?;
        Ok(cx.views.info(view, cx.doc.document()))
    }
}
register_command!(ViewGet, "Read view");
