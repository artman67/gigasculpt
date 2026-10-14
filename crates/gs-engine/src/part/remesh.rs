//! `part.remesh` (decision 0028, section 5): parts merged with `gs_geo::parts::merge`, then
//! voxel remeshed with `gs_geo::voxel::remesh_fold` into one new part, each part read as a
//! solid of its own (decision 0002, amendment F5) and folded in by its operation (decision
//! 0057, section 3), its holes closed as `close_holes` says (decision 0052 as amended by step
//! Prefs-c).

use gs_core::AttrDef;
use gs_core::transform::Similarity;
use gs_geo::parts::merge;
use gs_geo::voxel::{CellSize, CloseHoles, Settings, remesh_fold};
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Cost, ErrorCode, Kind, Length, PartRef,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::merge::{empty, nothing_shows, refusal};
use super::{NewPart, names, pin_name, pin_refs, pinned_name, report, shown};
use crate::ctx::PartLength;
use crate::mesh::{Bounds, FIRST_RESOLUTION, carry_columns, remesh_refusal, session_closing};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Voxel remesh several parts together into one new part at the end of the list.
///
/// The parts become one surface of even quads, closed unless `close_holes` leaves a part's
/// holes open: each part, whichever way its faces point, joins, cuts or intersects what is
/// above it by its operation (`part.set_operation`), the first part the base. Each part gives
/// what it shows (its active detail level) and stays as it is. With
/// `groups` each region of the result keeps a face group saying which part it came from (1 for
/// the first, 2 for the second, ...). Masks, hidden faces and UVs do not carry; colors and
/// other columns do, as for `mesh.voxel_remesh`. The new part keeps its cell size for later remeshes.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartRemesh {
    /// The parts. Left out or empty: every part that shows, in list order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartRef>,
    /// Cells along the longest side of the parts' bounding box together, 16 to 2048. Left out
    /// (with no `voxel_size`): 128.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 16, max = 2048))]
    pub resolution: Option<u32>,
    /// Cell size instead of a resolution (and used if both are given): scene units, or a
    /// percentage such as "1%" of the diagonal of the parts' bounding box together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voxel_size: Option<Length>,
    /// Give each part's region of the result its own face group.
    #[serde(default = "crate::command::yes")]
    pub groups: bool,
    /// Move the new surface onto the old ones, keeping detail finer than a cell.
    #[serde(default)]
    pub project: bool,
    /// 0 to 100: soften the projected detail. Only used with `project`.
    #[serde(default)]
    #[schemars(range(min = 0, max = 100))]
    pub blur: f32,
    /// Hole closing, for every part: 0, none (each part's inside is still read with its holes
    /// capped, then the surface across them is removed, so an open part stays open about its
    /// rim), or 1, large triangles (each hole closed by a patch across its rim). Left out: the
    /// session's hole closing (`mesh.voxel_settings`, 1 at first), written into the logged call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: Option<u8>,
    /// Name of the new part. Defaults to `remeshed` (or the first free name after it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
}

/// The remeshed part.
#[derive(Serialize, JsonSchema)]
pub struct Remeshed {
    /// The new part.
    pub part: NewPart,
    /// The parts remeshed into it, in the order given.
    pub from: Vec<String>,
    /// Cell size used, in scene units.
    pub voxel_size: f32,
}

impl Command for PartRemesh {
    const NAME: &'static str = "part.remesh";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    type Output = Remeshed;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "join a sphere and a cube that overlap into one part",
                json!({ "resolution": 32 }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [1, 0, 0] })),
            Example::raw(
                "remesh only the parts that show, with cells 5% of their size and no groups",
                json!({ "voxel_size": "5%", "groups": false, "name": "blob" }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [3, 0, 0] }))
            .after(
                "part.set_visible",
                json!({ "parts": ["cube"], "visible": false }),
            ),
            Example::raw(
                "nothing shows: the only part was deleted",
                json!({ "resolution": 32 }),
            )
            .after("primitive.sphere", json!({}))
            .after("part.delete", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_refs(cx, &mut self.parts)?;
        if self.parts.is_empty() {
            self.parts = shown(cx).into_iter().map(PartRef).collect();
        }
        if self.resolution.is_none() && self.voxel_size.is_none() {
            self.resolution = Some(FIRST_RESOLUTION);
        }
        self.close_holes.get_or_insert(session_closing(cx));
        pin_name(cx, &mut self.name, "remeshed")
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        if self.parts.is_empty() {
            return nothing_shows(cx);
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<Remeshed, CommandError> {
        let name = pinned_name(self.name)?;
        let from = names(&self.parts);
        let mut meshes = Vec::with_capacity(from.len());
        let mut operations = Vec::with_capacity(from.len());
        // Each column's kind, from the first part that holds it.
        let mut defs = vec![AttrDef::face_group()];
        for (k, part) in from.iter().enumerate() {
            let found = cx.doc.get_part(part)?;
            operations.push(crate::boolean::geo(found.props().operation));
            defs.extend_from_slice(found.store().attrs());
            let mut mesh = (*found.world()).clone();
            // Every face of each part is surface, and a mask would not mean anything here.
            mesh.attributes.remove("mask");
            mesh.attributes.remove("hide");
            mesh.attributes.remove("group");
            if self.groups {
                let group = vec![k as u32 + 1; mesh.faces.len()];
                let column = Attribute {
                    domain: AttrDomain::Face,
                    values: AttrValues::U32(group),
                };
                mesh.attributes.insert("group".to_string(), column);
            }
            meshes.push(mesh);
        }
        // A part without a column reads its empty value there (white for a color).
        let merged = merge(&meshes.iter().collect::<Vec<_>>(), empty).map_err(|e| refusal(&e))?;
        let cell = match self.voxel_size {
            // The new part sits at the identity, so its own units are the scene's.
            Some(size) => CellSize::Size(
                PartLength::new(size, Similarity::IDENTITY, || {
                    Bounds::of(&merged.positions).map_or(0.0, |b| b.diagonal)
                })
                .part,
            ),
            None => CellSize::Resolution(self.resolution.unwrap_or(FIRST_RESOLUTION)),
        };
        let settings = Settings {
            cell,
            project: self.project,
            blur: self.blur,
        };
        // Each part's inside is its own, whichever way it faces, folded in by its operation.
        let parts: Vec<_> = (meshes.iter().zip(operations))
            .map(|(m, operation)| (m.faces.len(), operation))
            .collect();
        let close = if self.close_holes == Some(0) {
            CloseHoles::Off
        } else {
            CloseHoles::Large
        };
        cx.progress(
            None,
            format!("remeshing {} parts into {name}", meshes.len()),
        );
        let mut out =
            remesh_fold(&merged, &parts, &settings, close).map_err(|e| remesh_refusal(&e))?;
        carry_columns(&merged, (&mut out.mesh, &out.sources), &defs, |_| true);
        let voxel_size = out.grid.voxel as f32;
        // Built from scene points: the new part sits at the identity (decision 0029).
        let placed = (out.kept, Similarity::IDENTITY);
        cx.doc.insert_remeshed(&name, out.mesh, placed)?;
        Ok(Remeshed {
            part: report(cx, &name)?,
            from,
            voxel_size,
        })
    }
}
register_command!(PartRemesh, "Remesh parts");
