//! `visibility.invert` and `visibility.show_all`: one implementation serving the two commands
//! that act on every face of a part.

use std::marker::PhantomData;

use gs_core::VisibilityEdit;
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Visibility;
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// What a `Whole` command does.
pub trait WholeOp: Send + Sized + 'static {
    const NAME: &'static str;
    const DOCS: &'static str;
    const SUMMARY: &'static str;
    const EDIT: VisibilityEdit<'static>;
}

/// `visibility.invert`.
pub struct Invert;
/// `visibility.show_all`.
pub struct ShowAll;

impl WholeOp for Invert {
    const NAME: &'static str = "visibility.invert";
    const DOCS: &'static str = "Swap hidden and visible faces. Inverting twice gives back the \
        same hidden faces. On a part with detail levels it acts on the top level from any level.";
    const SUMMARY: &'static str = "show the hidden faces and hide the others";
    const EDIT: VisibilityEdit<'static> = VisibilityEdit::Invert;
}

impl WholeOp for ShowAll {
    const NAME: &'static str = "visibility.show_all";
    const DOCS: &'static str = "Show every face of the part again, on every detail level.";
    const SUMMARY: &'static str = "show every face";
    const EDIT: VisibilityEdit<'static> = VisibilityEdit::ShowAll;
}

/// Change the visibility of every face of a part.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, bound = "")]
#[schemars(bound = "")]
pub struct Whole<O: WholeOp> {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    #[serde(skip)]
    #[schemars(skip)]
    op: PhantomData<O>,
}

impl<O: WholeOp> Command for Whole<O> {
    const NAME: &'static str = O::NAME;
    const KIND: Kind = Kind::Edit;
    const DOCS: &'static str = O::DOCS;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Visibility;

    fn examples() -> Vec<Example<Self>> {
        let cmd = Self {
            part: None,
            op: PhantomData,
        };
        vec![
            Example::new(O::SUMMARY, cmd)
                .after("primitive.sphere", json!({ "subdivisions": 8 }))
                .after(
                    "brush.stroke",
                    json!({
                        "brush": "Mask Pen",
                        "samples": [{ "position": [0.0, 0.0, 1.0] }],
                        "size": "25%",
                    }),
                )
                .after("visibility.hide", json!({ "select": { "by": "unmasked" } })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Visibility, CommandError> {
        let part = pinned(self.part)?;
        cx.noop_reason("the part's faces already were that way");
        crate::level::whole_part(cx, &part, |s| s.edit_visibility(O::EDIT))?;
        Ok(Visibility::of(&part, cx.doc.get_part(&part)?.store()))
    }
}
register_command!(Whole<Invert>, "Invert visibility");
register_command!(Whole<ShowAll>, "Show all faces");
