//! `visibility.grow` and `visibility.shrink`: what is visible grown or shrunk by rings of faces,
//! or grown to whole pieces or face groups (decision 0026, 6).

use gs_core::VisibilityEdit;
use gs_core::select::{FaceSet, Selector};
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Visibility;
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, errors, register_command};

/// What `visibility.grow` grows to instead of rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GrowTo {
    /// Every face of each connected piece that shows a face.
    Shells,
    /// Every face of each face group a visible face holds (faces in no group count as one
    /// group).
    Groups,
}

/// Show the hidden faces around what is visible: a ring is every face sharing a point with a
/// visible face. With `to` it shows every face of each connected piece (`"shells"`) or each
/// face group (`"groups"`) that shows a face instead.
///
/// Rings are of the detail level you see; growing to pieces or groups works on the top level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VisibilityGrow {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// How many rings, 1 to 100 (left out, 1). Not with `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub rings: Option<u32>,
    /// Grow to whole pieces or groups instead of rings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<GrowTo>,
}

/// Hide the visible faces beside hidden ones: a ring is every visible face sharing a point with
/// a hidden face. A visible face at an open border of the mesh stays.
///
/// Rings are of the detail level you see.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VisibilityShrink {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// How many rings, 1 to 100 (left out, 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub rings: Option<u32>,
}

/// `rings`, checked: 1 to 100, left out 1.
fn rings(rings: Option<u32>) -> Result<u32, CommandError> {
    let n = rings.unwrap_or(1);
    if (1..=100).contains(&n) {
        Ok(n)
    } else {
        Err(errors::args_invalid(
            "/rings",
            "rings must be from 1 to 100",
        ))
    }
}

/// `set` stepped `n` times, stopping early once a step changes nothing.
fn step(mut set: FaceSet, n: u32, ring: impl Fn(&FaceSet) -> FaceSet) -> FaceSet {
    for _ in 0..n {
        let next = ring(&set);
        if next == set {
            break;
        }
        set = next;
    }
    set
}

/// A sphere with a masked patch and the rest hidden, as the examples start.
fn patch_shown() -> [(&'static str, Value); 3] {
    [
        ("primitive.sphere", json!({ "subdivisions": 8 })),
        (
            "brush.stroke",
            json!({
                "brush": "Mask Pen",
                "samples": [{ "position": [0.0, 0.0, 1.0] }],
                "size": "25%",
            }),
        ),
        ("visibility.hide", json!({ "select": { "by": "unmasked" } })),
    ]
}

impl Command for VisibilityGrow {
    const NAME: &'static str = "visibility.grow";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Visibility;

    fn examples() -> Vec<Example<Self>> {
        let grow = |rings, to| Self {
            part: None,
            rings,
            to,
        };
        let after = |e: Example<Self>| {
            (patch_shown().into_iter()).fold(e, |e, (cmd, args)| e.after(cmd, args))
        };
        vec![
            after(Example::new(
                "show two rings around the visible patch",
                grow(Some(2), None),
            )),
            after(Example::new(
                "show every piece that shows a face",
                grow(None, Some(GrowTo::Shells)),
            )),
            Example::new(
                "show the whole group of every visible face",
                grow(None, Some(GrowTo::Groups)),
            )
            .after("primitive.cube", json!({}))
            .after("group.make", json!({ "maker": { "by": "visible" } }))
            .after(
                "visibility.hide",
                json!({ "select": { "by": "faces", "indexes": [0, 1] } }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Visibility, CommandError> {
        let part = pinned(self.part)?;
        cx.noop_reason("no hidden face was beside or with a visible one");
        let Some(to) = self.to else {
            let n = rings(self.rings)?;
            let store = cx.doc.edit_store(&part, |s| {
                let set = step(s.select(&Selector::Visible), n, |f| s.grow_faces(f));
                s.edit_visibility(VisibilityEdit::Show(&set))
            })?;
            return Ok(Visibility::of(&part, &store));
        };
        if self.rings.is_some() {
            let message = "give rings or to, not both";
            return Err(errors::args_invalid("/rings", message));
        }
        crate::level::whole_part(cx, &part, |s| {
            let visible = s.select(&Selector::Visible);
            let set = match to {
                GrowTo::Shells => s.shells_of(&visible),
                GrowTo::Groups => s.groups_of(&visible),
            };
            s.edit_visibility(VisibilityEdit::Show(&set))
        })?;
        Ok(Visibility::of(&part, cx.doc.get_part(&part)?.store()))
    }
}
register_command!(VisibilityGrow, "Grow visibility");

impl Command for VisibilityShrink {
    const NAME: &'static str = "visibility.shrink";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Visibility;

    fn examples() -> Vec<Example<Self>> {
        let shrink = Self {
            part: None,
            rings: Some(1),
        };
        let e = Example::new("hide the visible patch's outer ring", shrink);
        vec![(patch_shown().into_iter()).fold(e, |e, (cmd, args)| e.after(cmd, args))]
    }

    fn run(self, cx: &mut Ctx) -> Result<Visibility, CommandError> {
        let part = pinned(self.part)?;
        let n = rings(self.rings)?;
        cx.noop_reason("no visible face was beside a hidden one");
        let store = cx.doc.edit_store(&part, |s| {
            let set = step(s.select(&Selector::Visible), n, |f| s.shrink_faces(f));
            s.edit_visibility(VisibilityEdit::Only(&set))
        })?;
        Ok(Visibility::of(&part, &store))
    }
}
register_command!(VisibilityShrink, "Shrink visibility");
