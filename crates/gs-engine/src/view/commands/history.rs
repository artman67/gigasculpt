//! `view.undo` and `view.redo`: a view's own camera history, apart from document undo.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::{Moved, may_change, name};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Put back a view's camera from before its last change (up to 50 back). Every camera change
/// and every gesture is one step; document undo never touches it and it never touches the
/// document.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewUndo {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
}

/// Take back the last `view.undo` of a view. Any other camera change clears what can be redone.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewRedo {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
}

/// One step through view `view`'s camera history.
fn travel(cx: &mut Ctx, view: Option<&ViewName>, forward: bool) -> Result<Moved, CommandError> {
    let view = name(view)?;
    cx.views.travel(view, forward)?;
    let camera = cx.views.camera_of(view, cx.doc.document())?;
    Ok(Moved {
        view: view.to_string(),
        camera: camera.into(),
    })
}

impl Command for ViewUndo {
    const NAME: &'static str = "view.undo";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("back to the camera before the turn", json!({}))
                .after("primitive.sphere", json!({}))
                .after("view.orbit", json!({ "right_deg": 30 })),
            Example::raw("nothing to go back to", json!({})).fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        travel(cx, self.view.as_ref(), false)
    }
}
register_command!(ViewUndo, "Undo view");

impl Command for ViewRedo {
    const NAME: &'static str = "view.redo";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("forward again after an undo", json!({}))
                .after("primitive.sphere", json!({}))
                .after("view.orbit", json!({ "right_deg": 30 }))
                .after("view.undo", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        travel(cx, self.view.as_ref(), true)
    }
}
register_command!(ViewRedo, "Redo view");
