//! `view.set`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::{ViewName, ViewSpec};
use super::{Change, Moved, Touches, change, may_change, name, size};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Put a camera on a view: a standard view framing every part ("front"), another view's camera
/// ("main", to look from exactly where the window looks), an exact camera as `view.get`
/// reports it, or an orbit.
///
/// One step of the view's camera history (`view.undo` goes back); never part of document undo.
/// Gestures in the window are logged as this command with the camera they ended on.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewSet {
    /// The view to change: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels; framing a standard view
    /// or an orbit uses its shape. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The camera. A session view's name is replaced by its exact camera before it runs.
    pub camera: ViewSpec,
    /// False: not a step of the camera history (the engine logs the framing a view keeps
    /// this way). Left out: true.
    #[serde(default = "crate::command::yes", skip_serializing_if = "Clone::clone")]
    pub remember: bool,
}

impl Command for ViewSet {
    const NAME: &'static str = "view.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::View;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        let exact = json!({ "camera": {
            "orientation": [0, 0, 0, 1], "target": [0, 0, 0], "distance": 3
        } });
        vec![
            Example::raw("the front view of every part", json!({ "camera": "front" }))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "look from where the window looks",
                json!({ "camera": "main" }),
            )
            .after("primitive.sphere", json!({})),
            Example::raw("an exact camera, 3 units in front of the origin", exact),
            Example::raw(
                "a view that is not there",
                json!({ "view": "agent", "camera": "top" }),
            )
            .fails_with(gs_protocol::ErrorCode::NotFound),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let [w, h] = size(self.viewport)?;
        let aspect = w as f32 / h as f32;
        let doc = cx.doc.document();
        // A standard view or an orbit frames the scene this view shows (a Reference View's
        // own); another view's camera is that view's, wherever it looks.
        let frames = cx.views.scene(name(self.view.as_ref())?, doc);
        let camera = match &self.camera {
            ViewSpec::View(_) => self.camera.resolve(doc, cx.views, aspect)?,
            framing => framing.resolve(frames, cx.views, aspect)?,
        };
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: None,
            gesture: None,
            end: false,
            remember: self.remember,
            moves: Touches::NONE,
        };
        change(cx, &c, |now, _, _, _| *now = camera)
    }
}
register_command!(ViewSet, "Set view");
