//! `part.duplicate` and `part.add` (decision 0028, section 3), and the copy a Ctrl drag of the
//! gizmo places (decision 0027, section 14).

use gs_core::transform::Similarity;
use gs_protocol::{CommandError, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, pin_copy_name, pin_name, pin_ref, pinned_name, place_below, report};
use crate::command::PartArg;
use crate::document::{Props, pinned};
use crate::primitive::{PrimitiveCube, PrimitiveSphere};
use crate::transform::{FrameArg, OpArg, TransformApply, placed};
use crate::{Command, Ctx, Example, errors, register_command};

/// Copy a part, directly below it in the list, and make the copy the active part.
///
/// The copy has every detail level, mask, face group, hidden face and other column of the part,
/// its transform, and its eye and Isolate mark, under a new id; it is a Live Boolean `union`
/// that starts no group, whatever the part was. With `transform` the copy is moved, turned or
/// scaled evenly as a whole, in its part transform, as a Ctrl drag of the gizmo places it.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartDuplicate {
    #[doc = crate::command::part_doc!("The part to copy.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Name of the copy. Defaults to the part's name with -2, -3, ... (the first that is free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// Where the copy goes: a move, a turn or an even scale of the whole copy about a frame.
    /// Left out, the copy sits exactly on the part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Placement>,
}

/// A whole-part move, turn or even scale, as `transform.apply` writes it: no weights, so no
/// falloff, mask or mirror plane takes part.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// What it turns and scales about: "gizmo" (the part's gizmo, the default) or an exact
    /// `{origin, axes}`. The engine writes the gizmo's numbers into the call before it runs.
    #[serde(default)]
    pub frame: FrameArg,
    /// `move`, `rotate`, or `scale` with three equal factors above zero.
    pub op: OpArg,
}

impl Placement {
    fn as_transform(&self) -> TransformApply {
        TransformApply {
            frame: self.frame,
            ..TransformApply::new(self.op.clone())
        }
    }

    /// Writes part `part`'s gizmo into the frame when it is "gizmo".
    pub(crate) fn pin_frame(&mut self, cx: &Ctx, part: &str) -> Result<(), CommandError> {
        let mut t = self.as_transform();
        t.pin_frame(cx, part)?;
        self.frame = t.frame;
        Ok(())
    }

    /// The transform a part with transform `from` has once this places it. Refuses an
    /// operation that does not move the part whole, or a result a part cannot keep.
    pub(crate) fn place(&self, from: Similarity) -> Result<Similarity, CommandError> {
        let t = self.as_transform().resolve(|| 0.0)?;
        placed(from, &t).ok_or_else(|| {
            let message = "a copy moves, turns or scales evenly as a whole: move, rotate, or scale \
                           by three equal factors above zero";
            errors::args_invalid("/transform/op", message)
        })
    }
}

impl Command for PartDuplicate {
    const NAME: &'static str = "part.duplicate";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let named = Self {
            part: Some(PartRef("sphere".into())),
            name: Some("copy".into()),
            transform: None,
        };
        let taken = Self {
            part: Some(PartRef("sphere".into())),
            name: Some("cube".into()),
            transform: None,
        };
        let moved = json!({ "transform": { "op": { "move": [3.0, 0.0, 0.0] } } });
        let turned = json!({
            "transform": {
                "frame": { "origin": [2.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] },
                "op": { "rotate": { "axis": "y", "degrees": 90.0 } },
            },
        });
        let stretched = json!({ "transform": { "op": { "scale": [2.0, 1.0, 1.0] } } });
        vec![
            Example::raw("copy the only part", json!({})).after("primitive.sphere", json!({})),
            Example::new("copy the first of two parts, below it, named copy", named)
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({})),
            Example::new("a name another part has is refused", taken)
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
                .fails_with(ErrorCode::Precondition),
            Example::raw(
                "copy the sphere 3 units along X, as a Ctrl drag of the gizmo does",
                moved,
            )
            .after("primitive.sphere", json!({})),
            Example::raw(
                "copy the sphere a quarter turn about a point beside it",
                turned,
            )
            .after("primitive.sphere", json!({})),
            Example::raw("a copy moves whole: a stretch is refused", stretched)
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_copy_name(cx, self.part.as_ref(), &mut self.name)?;
        if let Some(t) = &mut self.transform {
            t.pin_frame(cx, &pinned(self.part.clone())?)?;
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        let name = pinned_name(self.name)?;
        let from = cx.doc.get_part(&part)?.props();
        let placed = (self.transform.as_ref())
            .map(|t| t.place(from.transform))
            .transpose()?;
        cx.doc.copy_part(&part, &name)?;
        let props = cx.doc.get_part(&name)?.props().as_copy();
        let transform = placed.unwrap_or(props.transform);
        cx.doc.set_props(&name, Props { transform, ..props })?;
        place_below(cx, &part, std::slice::from_ref(&name))?;
        report(cx, &name)
    }
}
register_command!(PartDuplicate, "Duplicate part");

/// A shape `part.add` adds.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// `primitive.sphere` with its defaults.
    Sphere,
    /// `primitive.cube` with its defaults.
    Cube,
}

/// Add a sphere or a cube as a new part, at the end of the list or directly below a part.
///
/// The shapes are exactly `primitive.sphere {}` and `primitive.cube {}`; this command adds the
/// choice of place.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartAdd {
    /// What to add.
    pub shape: Shape,
    /// Put the new part directly below this one. Left out: at the end of the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<PartRef>,
    /// Name of the new part. Defaults to the shape's name ("sphere" or "cube"), with -2, -3,
    /// ... if taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
}

impl Command for PartAdd {
    const NAME: &'static str = "part.add";
    const KIND: Kind = Kind::Edit;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("add a cube at the end", json!({ "shape": "cube" }))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "insert a sphere below the first of two parts",
                json!({ "shape": "sphere", "after": "@0" }),
            )
            .after("primitive.sphere", json!({ "name": "head" }))
            .after("primitive.cube", json!({})),
            Example::raw(
                "a part to insert below must exist",
                json!({ "shape": "cube", "after": "body" }),
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        if let Some(after) = &mut self.after {
            pin_ref(cx, after)?;
        }
        let base = match self.shape {
            Shape::Sphere => "sphere",
            Shape::Cube => "cube",
        };
        pin_name(cx, &mut self.name, base)
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let name = pinned_name(self.name)?;
        // The primitive commands with their defaults, so the shapes are theirs exactly.
        let args = json!({ "name": name });
        let decoded = |e: serde_json::Error| crate::errors::internal(&e.to_string());
        let made = match self.shape {
            Shape::Sphere => serde_json::from_value::<PrimitiveSphere>(args)
                .map_err(decoded)?
                .run(cx)?,
            Shape::Cube => serde_json::from_value::<PrimitiveCube>(args)
                .map_err(decoded)?
                .run(cx)?,
        };
        if let Some(PartRef(after)) = self.after {
            place_below(cx, &after, std::slice::from_ref(&made.part))?;
            return report(cx, &made.part);
        }
        Ok(made)
    }
}
register_command!(PartAdd, "Add part");
