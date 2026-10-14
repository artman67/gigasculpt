//! `mesh.flip` (decision 0041, section 9, amended in step DF6): a part turned into its own
//! mirror image in place, through the reflection `part.mirror_copy` uses.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{VertexOrder, pinned};
use crate::part::{mirrored, plane};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, register_command};

/// Turn a part into its own mirror image.
///
/// The part's points are mirrored across the plane square to `axis`, in the part's own space,
/// and every face's corners are reversed so it still faces out. Each point keeps its mask,
/// color and other values, each face its group, each face corner its UV, and the part keeps
/// its transform. Twice gives back every bit about the origin; about the middle a point comes
/// back within one f32 step at the part's far side on that axis. A part with detail levels is
/// refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshFlip {
    #[doc = crate::command::part_doc!("The part to flip.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The axis the mirror plane is square to: `x` flips left and right.
    #[serde(default = "x")]
    pub axis: Axis,
    /// Where the plane crosses the axis, in the part's own space.
    #[serde(default)]
    pub about: About,
}

/// Where the plane of `mesh.flip` crosses its axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum About {
    /// The middle of the part's bounding box, so the part stays where it is.
    #[default]
    Middle,
    /// The part's own origin, where its mirror planes are.
    Origin,
}

/// What was flipped.
#[derive(Serialize, JsonSchema)]
pub struct Flipped {
    /// The part.
    pub part: String,
    /// Where the plane crossed the axis, in the part's own space.
    pub plane: f64,
}

fn x() -> Axis {
    Axis::X
}

impl Command for MeshFlip {
    const NAME: &'static str = "mesh.flip";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[
        {"local": {"part": "/part"}},
        {"volume_non_decreasing": {"tolerance": 0.0}}
    ]"#;
    type Output = Flipped;

    fn examples() -> Vec<Example<Self>> {
        let sphere = json!({ "subdivisions": 4, "center": [2, 0, 0] });
        let in_place = Example::raw("flip a sculpted sphere left to right, in place", json!({}));
        let down = json!({ "axis": "y", "about": "origin" });
        vec![
            (in_place.after("primitive.sphere", sphere))
                .after("mesh.jitter", json!({ "amount": "2%", "seed": 3 })),
            Example::raw("flip a cube above y = 0 to below it", down)
                .after("primitive.cube", json!({ "center": [0, 1.5, 0] })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Flipped, CommandError> {
        let part = pinned(self.part)?;
        cx.doc.single_level(&part, "flip")?;
        // In the part's own space, keeping its transform (decision 0029).
        let mesh = cx.doc.local(&part)?;
        let axis = self.axis as usize;
        let plane = plane(&mesh, axis, self.about == About::Middle);
        let props = cx.doc.get_part(&part)?.props();
        let flipped = mirrored(&mesh, axis, plane);
        cx.doc
            .put_mesh(&part, flipped, VertexOrder::AsGiven, props)?;
        Ok(Flipped { part, plane })
    }
}
register_command!(MeshFlip, "Flip");
