//! `group.assign`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Grouped, fresh, given};
use crate::command::PartArg;
use crate::document::pinned;
use crate::view::ViewSpec;
use crate::visibility::{SelectArg, camera_view};
use crate::{Command, Ctx, Example, register_command};

/// A face group id, or a fresh one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum IdArg {
    /// This id, at most 2147483647. 0 takes the faces out of every group.
    Id(u32),
    /// `"new"`: one above the largest id any face holds (`E_LIMIT` once that would pass
    /// 2147483647).
    New(NewId),
}

/// The word `"new"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NewId {
    New,
}

/// Put the faces a selector picks in one face group: a new group (one above the largest id any
/// face holds, hidden faces included) or a given id.
///
/// Exactly the picked faces change. Hidden faces are picked like visible ones, so
/// `{"by": "visible"}` or `{"by": "hidden"}` choose by visibility. On a part with detail levels
/// it acts on the faces of the level you see, and every top face below them gets the id.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupAssign {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Which faces, as `visibility.hide` takes them: by mask, by group, all, visible, hidden,
    /// by index (`{"by": "faces", "indexes": [0, 7]}`) or facing the `camera`.
    pub select: SelectArg,
    /// `"new"` or an id. Left out, `"new"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<IdArg>,
    /// The camera a facing selector looks from, as `view.snapshot` takes it: a standard view
    /// ("front"), a session view ("main", pinned to its exact camera), an exact camera or an
    /// orbit. Only with a facing selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The size of the camera's image, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
}

impl Command for GroupAssign {
    const NAME: &'static str = "group.assign";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = Grouped;

    fn examples() -> Vec<Example<Self>> {
        let mask_pen = json!({
            "brush": "Mask Pen",
            "samples": [{ "position": [0.0, 0.0, 1.0] }],
            "size": "25%",
        });
        let masked = GroupAssign {
            part: None,
            select: SelectArg::Masked {
                threshold: None,
                rule: None,
            },
            id: None,
            camera: None,
            viewport: None,
        };
        vec![
            Example::new("put the masked faces in a new group", masked)
                .after("primitive.sphere", json!({ "subdivisions": 8 }))
                .after("brush.stroke", mask_pen),
            Example::raw(
                "put four faces in group 7",
                json!({ "select": { "by": "faces", "indexes": [0, 1, 2, 3] }, "id": 7 }),
            )
            .after("primitive.cube", json!({})),
            Example::raw(
                "put the faces turned toward the front view in a new group",
                json!({ "select": { "by": "facing", "angle": 60 }, "camera": "front" }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Grouped, CommandError> {
        let part = pinned(self.part)?;
        let view = camera_view(cx, &part, self.camera.as_ref(), self.viewport)?;
        let select = self.select.resolve(cx, &part, view)?;
        let id = match self.id {
            Some(IdArg::Id(id)) => given("/id", id)?,
            None | Some(IdArg::New(_)) => fresh(cx.doc.get_part(&part)?)?,
        };
        cx.noop_reason("every picked face already had that group");
        let mut ids = None;
        cx.doc.edit_store(&part, |s| {
            let set = s.select(&select);
            if set.count() == 0 {
                return Vec::new();
            }
            let nodes = s.assign_group(&set, id);
            ids = (!nodes.is_empty()).then_some([id, id]);
            nodes
        })?;
        Ok(Grouped::of(&part, cx.doc.get_part(&part)?, ids))
    }
}
register_command!(GroupAssign, "Assign face group");
