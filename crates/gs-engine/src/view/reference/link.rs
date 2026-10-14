//! `reference.link`: what of a Reference View follows main.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::{ReferenceShown, name, one_open, shown};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// One part of a Reference View's link to main.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Linked {
    Rotation,
    Zoom,
    Pan,
}

/// Choose what of a Reference View follows main: `rotation` (it turns with main), `zoom` (its
/// distance, lens and projection follow main's) and `pan` (the point it looks at follows
/// main's), each through its alignment; or `flip` one of them.
///
/// Unlinking keeps the picture: the reference stays where it is and moves on its own from
/// there (`view.orbit view=ref:1` and the rest). Linking again takes main's at once. Logged;
/// never undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceLink {
    /// The reference: "ref:1" ... Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// It turns with main. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<bool>,
    /// Its distance, lens and projection follow main's. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<bool>,
    /// The point it looks at follows main's. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<bool>,
    /// Switch this one over, after the values given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<Linked>,
}

impl Command for ReferenceLink {
    const NAME: &'static str = "reference.link";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Reference;
    type Output = ReferenceShown;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "stop turning with main, to turn the reference on its own",
                json!({ "rotation": false }),
            )
            .after("primitive.sphere", json!({}))
            .after("reference.open", json!({ "scene": true })),
            Example::raw("switch zoom linking over", json!({ "flip": "zoom" }))
                .after("primitive.sphere", json!({}))
                .after("reference.open", json!({ "scene": true })),
            Example::raw("no reference is open", json!({ "pan": true }))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        one_open(self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ReferenceShown, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        let doc = cx.doc.document();
        cx.views.reset_reference(&view, doc, |s| {
            let link = &mut s.link;
            link.rotation = self.rotation.unwrap_or(link.rotation);
            link.zoom = self.zoom.unwrap_or(link.zoom);
            link.pan = self.pan.unwrap_or(link.pan);
            match self.flip {
                Some(Linked::Rotation) => link.rotation = !link.rotation,
                Some(Linked::Zoom) => link.zoom = !link.zoom,
                Some(Linked::Pan) => link.pan = !link.pan,
                None => {}
            }
        });
        shown(cx, &view)
    }
}
register_command!(ReferenceLink, "Link reference");
