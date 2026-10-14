//! `view.look`: how any view is drawn, its MatCap and whether its edges show (decision 0025,
//! section 4).

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::reference::{SAME_AS_MAIN, checked_look};
use super::super::secondary::RefLook;
use super::super::spec::ViewName;
use super::name;
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, register_command};

/// Choose how a view is drawn: its MatCap and whether its edges show. One command for every
/// view: the window's Look options, its Wire switch and Shift+F, and a Reference View's look
/// menu and Wire all run it. A Reference View has a MatCap of its own ("main": whatever the
/// main view draws). `main` and an agent's view draw the session's look, so naming a MatCap
/// for them chooses it for every view and snapshot that draws the session's, as
/// `material.select` does; each view has its own edges. Leave out what stays. Logged; never
/// undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewLook {
    /// The view: "main", "ref:1" ... or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The MatCap, by name as `material.list` gives it, or "main" for whatever the main view
    /// draws. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub matcap: Option<String>,
    /// Draw its edges over the surface. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wireframe: Option<bool>,
    /// Switch the edges over, after the values given.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_wireframe: bool,
}

/// A view's look after `view.look`.
#[derive(Serialize, JsonSchema)]
pub struct LookShown {
    /// The view.
    pub view: String,
    /// The MatCap it draws, by name: a Reference View's own, else the session's active
    /// material. Always given: the engine keeps the palette, so a session that cannot render
    /// has one too.
    pub matcap: Option<String>,
    /// Its edges are drawn over the surface.
    pub wireframe: bool,
}

impl Command for ViewLook {
    const NAME: &'static str = "view.look";
    const KIND: Kind = Kind::View;
    /// Choosing the session's look bakes it for the window, as `material.select` does.
    const COST: Cost = Cost::Short;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = LookShown;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("show the edges of your view", json!({ "wireframe": true })),
            Example::raw(
                "switch the window's edges",
                json!({ "view": "main", "flip_wireframe": true }),
            ),
            Example::raw(
                "red wax with its edges for the reference, the main view unchanged",
                json!({ "view": "ref:1", "matcap": "red-wax", "wireframe": true }),
            )
            .after("primitive.sphere", json!({}))
            .after("reference.open", json!({ "scene": true })),
            Example::raw(
                "the reference back to the main view's look",
                json!({ "view": "ref:1", "matcap": SAME_AS_MAIN }),
            )
            .after("primitive.sphere", json!({}))
            .after("reference.open", json!({ "scene": true })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<LookShown, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        let own = cx.views.secondary(&view).map(|s| s.look.clone());
        let mut wireframe = self.wireframe.unwrap_or(cx.views.wireframe(&view));
        if self.flip_wireframe {
            wireframe = !wireframe;
        }
        let kept = own.as_ref().and_then(|now| now.matcap.clone());
        let wanted = RefLook {
            matcap: self.matcap.or(kept),
            wireframe,
        };
        let look = checked_look(cx, wanted)?;
        if own.is_some() {
            let doc = cx.doc.document();
            cx.views.reset_reference(&view, doc, |s| s.look = look);
        } else {
            if let Some(chosen) = look.matcap.as_deref() {
                select(cx, chosen)?;
            }
            cx.views.settle(&view, |v| v.wireframe = wireframe);
        }
        Ok(shown(cx, view))
    }
}
register_command!(ViewLook, "Set look");

/// Makes `look` the session's active material: what `main`, every agent's view and every
/// snapshot draw, renderer or not (decision 0043).
fn select(cx: &mut Ctx, look: &str) -> Result<(), CommandError> {
    crate::material::select(cx, look).map(drop)
}

/// View `view`'s look now.
fn shown(cx: &Ctx, view: String) -> LookShown {
    let own = cx
        .views
        .secondary(&view)
        .and_then(|s| s.look.matcap.clone());
    let session = || Some(cx.palette.now.active().name.clone());
    LookShown {
        matcap: own.or_else(session),
        wireframe: cx.views.wireframe(&view),
        view,
    }
}
