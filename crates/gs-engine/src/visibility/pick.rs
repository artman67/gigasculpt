//! `visibility.hide`, `visibility.show` and `visibility.only`: one implementation serving the
//! three commands that take a selector.

use std::marker::PhantomData;

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Op, SelectArg, Visibility, apply, camera_view};
use crate::command::PartArg;
use crate::document::pinned;
use crate::view::ViewSpec;
use crate::{Command, Ctx, Example, register_command};

/// What a `Pick` command does with the faces it picks.
pub trait PickOp: Send + Sized + 'static {
    const NAME: &'static str;
    const DOCS: &'static str;
    const OP: Op;
    fn examples() -> Vec<Example<Pick<Self>>>;
}

/// `visibility.hide`.
pub struct Hide;
/// `visibility.show`.
pub struct Show;
/// `visibility.only`.
pub struct Only;

/// A sphere with a mask painted on its front.
fn masked_sphere<O: PickOp>(e: Example<Pick<O>>) -> Example<Pick<O>> {
    let mask_pen = json!({
        "brush": "Mask Pen",
        "samples": [{ "position": [0.0, 0.0, 1.0] }],
        "size": "25%",
    });
    e.after("primitive.sphere", json!({ "subdivisions": 8 }))
        .after("brush.stroke", mask_pen)
}

/// The call with this selector.
fn by<O: PickOp>(select: SelectArg) -> Pick<O> {
    Pick {
        part: None,
        select,
        camera: None,
        viewport: None,
        op: PhantomData,
    }
}

fn masked() -> SelectArg {
    SelectArg::Masked {
        threshold: None,
        rule: None,
    }
}

/// Faces in group `id`, as raw arguments.
fn group(id: u32) -> Value {
    json!({ "select": { "by": "groups", "ids": [id] } })
}

impl PickOp for Hide {
    const NAME: &'static str = "visibility.hide";
    const DOCS: &'static str = "Hide the faces a selector picks: by mask (Hide by Mask hides the \
        unmasked faces, `{\"by\": \"unmasked\"}`) or by face group. Hidden faces are not drawn, \
        not sculpted and not crossed by smoothing; a point is hidden when every face around it \
        is. Faces already hidden stay hidden.";
    const OP: Op = Op::Hide;

    fn examples() -> Vec<Example<Pick<Self>>> {
        let unmasked = SelectArg::Unmasked {
            threshold: None,
            rule: None,
        };
        vec![
            masked_sphere(Example::new("hide every face not masked", by(unmasked))),
            Example::raw("hide every face without a group", group(0))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "hide the faces turned toward the front view",
                json!({ "select": { "by": "facing" }, "camera": "front" }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }
}

impl PickOp for Show {
    const NAME: &'static str = "visibility.show";
    const DOCS: &'static str = "Show again the faces a selector picks: by mask (Show by Mask \
        shows the unmasked faces) or by face group. Other faces keep their visibility.";
    const OP: Op = Op::Show;

    fn examples() -> Vec<Example<Pick<Self>>> {
        let any = SelectArg::Masked {
            threshold: Some(0.25),
            rule: Some(super::RuleArg::Any),
        };
        vec![
            masked_sphere(Example::new(
                "show faces with a corner masked over 0.25",
                by(any),
            ))
            .after("visibility.hide", json!({ "select": { "by": "masked" } })),
            Example::raw("show every face without a group", group(0))
                .after("primitive.sphere", json!({}))
                .after("visibility.hide", group(0)),
        ]
    }
}

impl PickOp for Only {
    const NAME: &'static str = "visibility.only";
    const DOCS: &'static str = "Show the faces a selector picks and hide every other face \
        (isolate a face group or the masked area).";
    const OP: Op = Op::Only;

    fn examples() -> Vec<Example<Pick<Self>>> {
        vec![
            masked_sphere(Example::new("isolate the masked area", by(masked()))),
            Example::raw("show only face group 1, which no face is in yet", group(1))
                .after("primitive.sphere", json!({})),
        ]
    }
}

/// Change the visibility of the faces a selector picks.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, bound = "")]
#[schemars(bound = "")]
pub struct Pick<O: PickOp> {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Which faces: `{"by": "masked"}` or `{"by": "unmasked"}`, with optional `threshold` (0.5)
    /// and `rule` (`mean`, `any` or `all` corners), `{"by": "groups", "ids": [2, 5]}`,
    /// `{"by": "all"}`, `{"by": "visible"}`, `{"by": "hidden"}`,
    /// `{"by": "faces", "indexes": [0, 7]}`, or `{"by": "facing", "angle": 60}` with a `camera`.
    pub select: SelectArg,
    /// The camera a facing selector looks from, as `view.snapshot` takes it: a standard view
    /// ("front"), a session view ("main", pinned to its exact camera), an exact camera or an
    /// orbit. Only with a facing selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The size of the camera's image, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    #[serde(skip)]
    #[schemars(skip)]
    op: PhantomData<O>,
}

impl<O: PickOp> Command for Pick<O> {
    const NAME: &'static str = O::NAME;
    const KIND: Kind = Kind::Edit;
    const DOCS: &'static str = O::DOCS;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = Visibility;

    fn examples() -> Vec<Example<Self>> {
        O::examples()
    }

    fn run(self, cx: &mut Ctx) -> Result<Visibility, CommandError> {
        let part = pinned(self.part)?;
        let view = camera_view(cx, &part, self.camera.as_ref(), self.viewport)?;
        let select = self.select.resolve(cx, &part, view)?;
        cx.noop_reason("every face the selector picked already had that visibility");
        let store = cx.doc.edit_store(&part, |s| apply(s, O::OP, &select))?;
        Ok(Visibility::of(&part, &store))
    }
}
register_command!(Pick<Hide>, "Hide faces");
register_command!(Pick<Show>, "Show faces");
register_command!(Pick<Only>, "Show only faces");
