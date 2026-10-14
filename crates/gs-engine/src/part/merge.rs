//! `part.merge_down` and `part.merge_visible` (decision 0028, section 4). The geometry is
//! `gs_geo::parts::merge`.

use gs_core::{AttrDef, AttrValue};
use gs_geo::parts::{MergeError, merge};
use gs_protocol::{Attribute, CommandError, ErrorCode, Kind, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, pin_name, pinned_name, report, shown};
use crate::command::PartArg;
use crate::document::{Props, VertexOrder, pinned};
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Merge a part and the part below it in the list into one part.
///
/// The result takes the upper part's place, name, id and properties; the lower part is
/// removed. Every column carries: a column only one of them had reads its empty value on the
/// other's faces or points (white for a color, 0 for the rest), and face group ids that collide
/// move up. A part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartMergeDown {
    #[doc = crate::command::part_doc!("The upper part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// A merge's result.
#[derive(Serialize, JsonSchema)]
pub struct Merged {
    /// The merged part.
    pub part: NewPart,
    /// The parts merged into it, in list order.
    pub from: Vec<String>,
}

impl Command for PartMergeDown {
    const NAME: &'static str = "part.merge_down";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = Merged;

    fn examples() -> Vec<Example<Self>> {
        let two = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({ "center": [3, 0, 0] }))
        };
        vec![
            two(Example::raw(
                "merge the cube into the sphere",
                json!({ "part": "sphere" }),
            )),
            two(Example::raw(
                "the last part has nothing below it",
                json!({ "part": "cube" }),
            ))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        let part = self.part.as_ref().map_or("", |p| p.0.as_str());
        let Some(lower) = below(cx, part) else {
            return Err(Unavailable::new(format!(
                "part '{part}' is last in the list, so nothing is below it to merge"
            ))
            .fix("move a part below it with part.reorder, or merge the part above it"));
        };
        crate::level::check_one_level(cx, self.part.as_ref(), "a merge", false)?;
        crate::level::check_one_level(cx, Some(&PartRef(lower)), "a merge", false)
    }

    fn run(self, cx: &mut Ctx) -> Result<Merged, CommandError> {
        let part = pinned(self.part)?;
        let lower = below(cx, &part).ok_or_else(|| errors::internal("no part below"))?;
        cx.doc.single_level(&part, "a merge")?;
        cx.doc.single_level(&lower, "a merge")?;
        let (upper_mesh, lower_mesh) = (cx.doc.part(&part)?, cx.doc.part(&lower)?);
        let mesh = merge(&[&upper_mesh, &lower_mesh], empty).map_err(|e| refusal(&e))?;
        // Built from both parts' scene points: the merged part sits at the identity (decision
        // 0029, section 4).
        let props = cx.doc.get_part(&part)?.props().at_identity();
        cx.doc.put_mesh(&part, mesh, VertexOrder::AsGiven, props)?;
        cx.doc.remove_part(&lower);
        Ok(Merged {
            part: report(cx, &part)?,
            from: vec![part, lower],
        })
    }
}
register_command!(PartMergeDown, "Merge down");

/// Merge every part that shows into one new part at the end of the list; they stay as they are.
///
/// Each part gives what it shows (its active detail level). Every column carries: a column only
/// some of them had reads its empty value on the others (white for a color, 0 for the rest),
/// and face group ids that collide move up.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartMergeVisible {
    /// Name of the new part. Defaults to `merged-` and the first part's name (the first that is
    /// free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
}

impl Command for PartMergeVisible {
    const NAME: &'static str = "part.merge_visible";
    const KIND: Kind = Kind::Edit;
    type Output = Merged;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "merge the parts that show, leaving out the hidden cube",
                json!({}),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [3, 0, 0] }))
            .after(
                "primitive.sphere",
                json!({ "center": [0, 3, 0], "name": "head" }),
            )
            .after(
                "part.set_visible",
                json!({ "parts": ["cube"], "visible": false }),
            ),
            Example::raw(
                "nothing shows: the scene is empty",
                json!({ "name": "all" }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let first = shown(cx).into_iter().next().unwrap_or_default();
        pin_name(cx, &mut self.name, &format!("merged-{first}"))
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        nothing_shows(cx)
    }

    fn run(self, cx: &mut Ctx) -> Result<Merged, CommandError> {
        let name = pinned_name(self.name)?;
        let from = shown(cx);
        let meshes = (from.iter().map(|part| cx.doc.part(part))).collect::<Result<Vec<_>, _>>()?;
        let mesh = merge(
            &meshes
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&PlainMesh>>(),
            empty,
        )
        .map_err(|e| refusal(&e))?;
        cx.doc
            .put_mesh(&name, mesh, VertexOrder::AsGiven, Props::default())?;
        Ok(Merged {
            part: report(cx, &name)?,
            from,
        })
    }
}
register_command!(PartMergeVisible, "Merge visible");

/// Refuses a command that reads the parts that show when none does.
pub(crate) fn nothing_shows(cx: &Ctx) -> Result<(), Unavailable> {
    if shown(cx).is_empty() {
        return Err(Unavailable::new("no part shows")
            .fix("show a part with part.set_visible, or end Isolate with part.isolate"));
    }
    Ok(())
}

/// The part directly below part `name` in the list.
fn below(cx: &Ctx, name: &str) -> Option<String> {
    let list = cx.doc.list_names();
    let at = list.iter().position(|n| n == name)?;
    list.into_iter().nth(at + 1)
}

/// What a merged mesh reads, as raw bits, in column `name` where a part lacks it: the empty
/// value of the kind a store gives the column, white for a color (decision 0039, section 2)
/// and 0 for the rest.
pub(crate) fn empty(name: &str, column: &Attribute) -> u32 {
    AttrDef::of_plain(name, column).map_or(0, |def| AttrValue::zero(def.kind).bits())
}

/// The command error for meshes that cannot be merged.
pub(crate) fn refusal(e: &MergeError) -> CommandError {
    errors::unavailable(Unavailable::new(e.to_string()))
}
