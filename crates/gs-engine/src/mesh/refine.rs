//! `mesh.refine` (decision 0021, L4; catalog `geometry.triangle-remesh`): Live Detail's pass
//! over a whole part or a selected region, again until it changes nothing, as one undo step.
//! Blender's Detail Flood Fill does the same. The work is gs-core's
//! `live::refine_all`, the one home of the algorithm.

use gs_core::live::{self, LiveReport, Target};
use gs_core::select::Selector;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::pinned;
use crate::errors::{self, error};
use crate::sculpt::{Axis, torn_uvs};
use crate::visibility::SelectArg;
use crate::{Command, Ctx, Example, register_command};

/// Passes a call runs at most. A sphere of 2,000 triangles refined to a tenth of its edges
/// comes to rest in about ten.
pub const MOST_PASSES: u32 = 64;

/// Re-lay a part's triangles at one edge length: edges longer than `size` split, edges shorter
/// than `merge` times it merge, and edges turn and points slide to even the mesh out, pass
/// after pass until nothing changes. Live Detail's work, over the whole part or the faces
/// `select` picks rather than under a brush.
///
/// Mask, face groups and paint carry over as in Live Detail: fully masked points, hidden faces
/// and group borders stay where they are. The faces it reaches come out triangles: a quad no
/// split needs is split along its shorter diagonal. A second run with the same settings changes
/// nothing. A part with detail levels or UVs is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshRefine {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The longest edge to keep, in scene units: `live_detail.pick` measures one.
    pub size: f32,
    /// Edges shorter than this times `size` merge, 0 to 1; at most 0.45 is used. Left out,
    /// 0.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<f32>,
    /// Merge short edges; off only adds triangles. Left out, on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined: Option<bool>,
    /// Only the faces this picks, by mask, face group or visibility, found again before each
    /// pass so the faces a pass makes count by their own mask and group. Left out, every
    /// visible face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<SelectArg>,
    /// Edit in mirrored pairs across this axis's plane, so a symmetric part keeps its mirror
    /// map and stays exactly symmetric. The part needs a map there (`symmetry.map` or
    /// `mesh.symmetrize`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<Axis>,
}

/// What the passes did.
#[derive(Serialize, JsonSchema)]
pub struct Refined {
    /// The part.
    pub part: String,
    /// Passes run.
    pub passes: u32,
    /// The last pass changed nothing, so running it again changes nothing. False when the cap
    /// of 64 passes came first: run it again to go on.
    pub converged: bool,
    /// Edges split in two.
    pub splits: u32,
    /// Short edges merged.
    pub collapses: u32,
    /// Edges turned to even out the mesh.
    pub flips: u32,
    /// Quads split into two triangles.
    pub quad_splits: u32,
    pub points_added: u32,
    pub points_removed: u32,
    /// Points after.
    pub points: u64,
    /// Faces after.
    pub faces: u64,
    /// The passes released the part's mirror map: the topology is no longer symmetric
    /// (`mesh.symmetrize` restores it, `symmetry` keeps it).
    pub mirror_cleared: bool,
}

impl Command for MeshRefine {
    const NAME: &'static str = "mesh.refine";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Refined;

    fn examples() -> Vec<Example<Self>> {
        let to = |size, select, symmetry| Self {
            part: None,
            size,
            merge: None,
            combined: None,
            select,
            symmetry,
        };
        let side = SelectArg::Groups { ids: vec![1] };
        vec![
            Example::new("edges 0.1 long over a whole sphere", to(0.1, None, None))
                .after("primitive.sphere", json!({ "subdivisions": 8 })),
            Example::new("one side of a cube", to(0.3, Some(side), None))
                .after("primitive.cube", json!({}))
                .after("group.make", json!({ "maker": { "by": "normals" } })),
            Example::new(
                "a symmetric sphere, kept symmetric across x",
                to(0.1, None, Some(Axis::X)),
            )
            .after("primitive.sphere", json!({ "subdivisions": 8 }))
            .after("symmetry.map", json!({ "axis": "x" })),
            Example::new("a part with detail levels is refused", to(0.1, None, None))
                .after("primitive.sphere", json!({ "subdivisions": 4 }))
                .after("level.subdivide", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Refined, CommandError> {
        let part = pinned(self.part.clone())?;
        if !(self.size > 0.0 && self.size.is_finite()) {
            let message = format!("the size must be a positive number, not {}", self.size);
            return Err(errors::args_invalid("/size", message));
        }
        cx.doc.single_level(&part, "refining")?;
        let found = cx.doc.get_part(&part)?;
        if let Some(why) = torn_uvs(found.store()) {
            return Err(error(
                ErrorCode::Precondition,
                format!("part '{part}' {why}"),
            ));
        }
        if let Some(axis) = self.symmetry
            && !found.store().has_mirror(axis as usize)
        {
            let name = ["x", "y", "z"][axis as usize];
            return Err(CommandError {
                hint: Some(
                    "run symmetry.map to find the map of a symmetric part, or mesh.symmetrize \
                     to make it symmetric with one"
                        .into(),
                ),
                ..error(
                    ErrorCode::Precondition,
                    format!("part '{part}' has no mirror map across {name}"),
                )
            });
        }
        let select = self.selector(cx, &part)?;
        let ld = gs_core::LiveDetail {
            target: Target::Constant(cx.doc.transform(&part)?.part_length(self.size)),
            merge: self.merge.unwrap_or(gs_core::LiveDetail::default().merge),
            combined: self.combined.unwrap_or(true),
            budget: u32::MAX,
            ..gs_core::LiveDetail::default()
        };
        ld.check().map_err(|m| errors::args_invalid("/merge", m))?;
        let mirrored = std::array::from_fn(|a| self.symmetry.is_some_and(|s| s as usize == a));
        let mut report = LiveReport::default();
        let mut rest = false;
        cx.progress(None, format!("refining {part}'s triangles"));
        let store = cx.doc.edit_store(&part, |s| {
            let (written, r) =
                live::refine_all(s, select.as_ref(), &ld, mirrored, MOST_PASSES, &mut report);
            rest = r;
            written
        })?;
        let (points, faces) = (store.vertex_count() as u64, store.face_count() as u64);
        cx.noop_reason(
            "no edge was longer than the size or shorter than its merge length, but for \
             masked and hidden ones: a smaller size adds triangles",
        );
        Ok(Refined {
            part,
            passes: report.passes,
            converged: rest,
            splits: report.splits,
            collapses: report.collapses,
            flips: report.flips,
            quad_splits: report.quad_splits,
            points_added: report.added,
            points_removed: report.removed,
            points,
            faces,
            mirror_cleared: report.mirror_cleared,
        })
    }
}
register_command!(MeshRefine, "Refine triangles");

impl MeshRefine {
    /// The region, if `select` names one: by mask, face group or visibility, which the faces a
    /// pass makes carry.
    fn selector(&self, cx: &Ctx, part: &str) -> Result<Option<Selector>, CommandError> {
        let Some(select) = &self.select else {
            return Ok(None);
        };
        if matches!(select, SelectArg::Faces { .. } | SelectArg::Facing { .. }) {
            let message = "pick the region by mask, face group or visibility, which the faces \
                           a pass makes carry; faces by index or by facing do not";
            return Err(errors::args_invalid("/select/by", message));
        }
        select.resolve(cx, part, None).map(Some)
    }
}
