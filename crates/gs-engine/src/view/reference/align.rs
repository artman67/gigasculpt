//! `reference.align`: lining up a reference's front with the sculpt's, once.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::MAIN;
use super::super::frame::scene_box;
use super::super::secondary::Alignment;
use super::super::spec::ViewName;
use super::{ReferenceShown, name, one_open, shown};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Line up a Reference View with the sculpt, once: from now on it shows the side main shows.
///
/// Without `to`, it keeps exactly the picture it shows now and links its rotation, so every
/// later turn of main turns it the same way. So: unlink rotation (`reference.link
/// rotation=false`), turn the reference until it shows the side main shows, then align. Its
/// box is also put on the scene's (centre on centre, same diagonal), so linking zoom and pan
/// afterwards shows both at the same size and place. `to` sets the alignment exactly instead
/// (a turn, scale and offset that place the reference in the sculpt's space), and `reset` puts
/// the reference back as it is. Never changes the reference's meshes. Logged; never undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceAlign {
    /// The reference: "ref:1" ... Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The alignment, exactly. Left out: match what both views show now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Alignment>,
    /// Back to no alignment: the reference as it is in its own space.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
}

impl Command for ReferenceAlign {
    const NAME: &'static str = "reference.align";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Reference;
    type Output = ReferenceShown;

    fn examples() -> Vec<Example<Self>> {
        let open = json!({ "parts": "cube", "link": { "rotation": false } });
        vec![
            Example::raw(
                "the reference turned to show its left side lines up with main's front",
                json!({}),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [3, 0, 0] }))
            .after("reference.open", open)
            .after("view.align", json!({ "view": "ref:1", "to": "left" })),
            Example::raw(
                "a reference a quarter turn about Y and half the size",
                json!({ "to": { "turn": [0, 0.70710677, 0, 0.70710677], "scale": 0.5, "offset": [0, 0, 0] } }),
            )
            .after("primitive.sphere", json!({}))
            .after("reference.open", json!({ "scene": true })),
            Example::raw(
                "a turn that is not a rotation",
                json!({ "to": { "turn": [0, 0, 0, 2], "scale": 1, "offset": [0, 0, 0] } }),
            )
            .after("primitive.sphere", json!({}))
            .after("reference.open", json!({ "scene": true }))
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        one_open(self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ReferenceShown, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        let doc = cx.doc.document();
        let align = match (self.reset, self.to) {
            (true, _) => Alignment::IDENTITY,
            (false, Some(to)) => {
                to.check("/to")?;
                to
            }
            (false, None) => {
                let main = cx.views.camera_of(MAIN, doc)?;
                let seen = cx.views.camera_of(&view, doc)?;
                let theirs = scene_box(cx.views.scene(&view, doc));
                let ours = scene_box(doc);
                Alignment::matching(main.orientation, seen.orientation, ours, theirs)
            }
        };
        cx.views.reset_reference(&view, doc, |s| {
            s.align = align;
            s.link.rotation = true;
        });
        shown(cx, &view)
    }
}
register_command!(ReferenceAlign, "Align reference");
