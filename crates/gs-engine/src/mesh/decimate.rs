//! `mesh.decimate` (decision 0034, catalog `plugins.decimate`): fewer, larger triangles made of
//! the part's own points. The work is `gs_geo::decimate`, a pure function around meshoptimizer.

use std::sync::Arc;

use gs_geo::decimate::{self as geo, Repairs, Settings, mirror, repair, unpaired};
use gs_geo::voxel::Kept;
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Cost, ErrorCode, HIDE, Kind, PartRef,
    PlainMesh,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::drop_uvs;
use super::symmetrize::{Side, symmetrize};
use crate::command::PartArg;
use crate::document::pinned;
use crate::errors;
use crate::level::uv_corners;
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Make a part lighter: fewer, larger triangles where the surface is flat, and the faces kept
/// where there is detail. No point moves: the result is made of the part's own points.
///
/// The result is all triangles. Face groups and UVs come through exactly, seams and borders
/// included; point values (mask, color) stay with their points, and faces stay where the paint
/// changes, as they stay where the shape does. Masked points (0.5 or more) and the points of
/// hidden faces are kept, so a masked area keeps its density while the rest gives up more;
/// hidden faces come back as they were. Faces folded onto each other are removed and
/// edges on more than two faces split. A part with detail levels is refused; freeze them
/// (`level.freeze`) to decimate the base and keep them for `level.unfreeze`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per thing to keep"
)]
pub struct MeshDecimate {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Faces to keep, 0.01 to 100, as a percentage of the part's shown faces (a quad counts
    /// as one). Left out with `faces` left out too: 50, half of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 100), extend("default" = 50))]
    pub percent: Option<f32>,
    /// Faces to keep, as a count. Wins when `percent` is given too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub faces: Option<u64>,
    /// Keep face groups, their borders fixed in place. Off: the result has no groups.
    #[serde(default = "crate::command::yes")]
    pub keep_groups: bool,
    /// Keep UVs exactly, seams and all. Off: the result has no UVs.
    #[serde(default = "crate::command::yes")]
    pub keep_uvs: bool,
    /// Keep painted detail: weigh the color in the error, so faces stay where the paint
    /// changes, at a little more distance from the old surface there. Off: the paint stays
    /// with its points but does not hold any face.
    #[serde(default = "crate::command::yes")]
    pub keep_paint: bool,
    /// Keep every point on an open edge, so holes and cut edges keep their outline.
    #[serde(default)]
    pub keep_borders: bool,
    /// Make the triangles more even, at a little more distance from the old surface.
    #[serde(default = "crate::command::yes")]
    pub uniform: bool,
    /// Decimate the positive half across this axis's plane and mirror it, so a symmetric part
    /// stays exactly symmetric and holds its mirror map. The part must be symmetric already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<Axis>,
}

/// The decimated part.
#[derive(Serialize, JsonSchema)]
pub struct Decimated {
    /// The part.
    pub part: String,
    /// Faces before, hidden ones included.
    pub faces_before: u64,
    /// Faces after: triangles, and any hidden faces as they were.
    pub faces: u64,
    /// Points before.
    pub points_before: u64,
    /// Points after.
    pub points: u64,
    /// The shown faces asked for.
    pub target: u64,
    /// Whether the shown faces came down to the target: locked points can stop them short.
    pub reached: bool,
    /// meshoptimizer's estimate of how far the new surface lies from the old, in scene units.
    pub error: f32,
    /// Points kept in place: masked ones, those of hidden faces and group borders (with
    /// `symmetry`, in the decimated half).
    pub locked: u64,
    /// Faces folded onto each other or repeated, removed.
    pub duplicate_faces: u64,
    /// Edges left on more than two faces, split.
    pub crowded_edges: u64,
    /// Points the splits added.
    pub points_added: u64,
}

/// Points this close to the plane, as a fraction of the part's diagonal, count as on it
/// (`mesh.symmetrize`'s snap).
const SNAP: f32 = 1e-6;
/// How far a point may be from its partner's mirror image for the part to count as symmetric,
/// as a fraction of its diagonal: `symmetry.map`'s default 0.01%.
const SYMMETRIC: f32 = 1e-4;

impl Command for MeshDecimate {
    const NAME: &'static str = "mesh.decimate";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    // Replaces only its own part. No support claim: the whole part is new.
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Decimated;

    fn examples() -> Vec<Example<Self>> {
        let to = |percent, faces, symmetry| Self {
            part: None,
            percent,
            faces,
            keep_groups: true,
            keep_uvs: true,
            keep_paint: true,
            keep_borders: false,
            uniform: true,
            symmetry,
        };
        vec![
            Example::new(
                "decimate a sphere to 10% of its faces",
                to(Some(10.0), None, None),
            )
            .after("primitive.sphere", json!({ "subdivisions": 16 })),
            Example::new(
                "decimate a sphere to 500 faces, mirrored across x",
                to(None, Some(500), Some(Axis::X)),
            )
            .after("primitive.sphere", json!({ "subdivisions": 16 })),
            Example::new(
                "a part with detail levels is refused",
                to(Some(50.0), None, None),
            )
            .after("primitive.sphere", json!({ "subdivisions": 4 }))
            .after("level.subdivide", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        crate::level::check_one_level(cx, self.part.as_ref(), "decimate", true)
    }

    fn run(self, cx: &mut Ctx) -> Result<Decimated, CommandError> {
        let part = pinned(self.part.clone())?;
        cx.doc.single_level_or_frozen(&part, "decimate")?;
        // In the part's own space, where its symmetry planes are (decision 0029, section 7).
        let (mut mesh, t) = (
            Arc::unwrap_or_clone(cx.doc.local(&part)?),
            cx.doc.transform(&part)?,
        );
        let before = (mesh.faces.len() as u64, mesh.positions.len() as u64);
        let target = self.target(&part, shown(&mesh))?;
        // A map is never half true (decision 0021, section 10): a symmetric run writes a new one.
        mesh.attributes
            .retain(|name, _| !gs_core::mirror::is_map(name));
        if !self.keep_uvs {
            drop_uvs(&mut mesh);
        }
        let settings = Settings {
            faces: target,
            keep_groups: self.keep_groups,
            keep_borders: self.keep_borders,
            uniform: self.uniform,
            corners: if self.keep_uvs {
                uv_corners()
            } else {
                Vec::new()
            },
            paint: if self.keep_paint {
                paint(&mesh)
            } else {
                Vec::new()
            },
            paint_weight: geo::PAINT_WEIGHT,
        };
        cx.progress(None, format!("decimating {part}"));
        let out = match self.symmetry {
            None => geo::decimate(&mesh, &settings),
            Some(axis) => symmetric(cx, &part, &mesh, axis as usize, settings)?,
        };
        let reply = Decimated {
            part: part.clone(),
            faces_before: before.0,
            faces: out.mesh.faces.len() as u64,
            points_before: before.1,
            points: out.mesh.positions.len() as u64,
            target: target as u64,
            reached: shown(&out.mesh) <= target,
            error: t.length(out.error),
            locked: out.locked as u64,
            duplicate_faces: out.repairs.duplicate_faces,
            crowded_edges: out.repairs.crowded_edges,
            points_added: out.repairs.points_added,
        };
        // A new surface in the part's place, keeping its cell size and any frozen levels.
        cx.doc.remesh_in_place(&part, out.mesh, (None::<Kept>, t))?;
        Ok(reply)
    }
}
register_command!(MeshDecimate, "Decimate");

impl MeshDecimate {
    /// The shown faces to keep, refusing a target the part cannot go down to. With neither
    /// `faces` nor `percent`, half of them: the window's Decimate runs as it opens (review 0009,
    /// R7), its slider showing the schema's default, 50.
    fn target(&self, part: &str, shown: usize) -> Result<usize, CommandError> {
        let percent = self.percent.unwrap_or(50.0);
        let wanted = match self.faces {
            Some(n) => usize::try_from(n).unwrap_or(usize::MAX),
            None => ((shown as f64 * f64::from(percent) / 100.0).round() as usize).max(1),
        };
        let refuse = |message: String, fix| errors::unavailable(Unavailable::new(message).fix(fix));
        if shown <= 3 {
            return Err(refuse(
                format!("part '{part}' has {shown} shown faces, and decimate needs more than 3"),
                "decimate a part with more faces, or show its hidden faces",
            ));
        }
        if wanted >= shown {
            return Err(refuse(
                format!("{wanted} faces is not fewer than part '{part}''s {shown} shown faces"),
                "ask for fewer faces",
            ));
        }
        Ok(wanted)
    }
}

/// Each point's color in linear light, for meshoptimizer to weigh; empty with no color column.
fn paint(mesh: &PlainMesh) -> Vec<[f32; 3]> {
    match mesh.attributes.get(&gs_core::AttrDef::color().name) {
        Some(Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(colors),
        }) => colors.iter().map(|&c| gs_core::linear_rgb(c)).collect(),
        _ => Vec::new(),
    }
}

/// The faces not hidden.
fn shown(mesh: &PlainMesh) -> usize {
    match mesh.attributes.get(HIDE).map(|a| &a.values) {
        Some(AttrValues::U32(h)) => h.iter().filter(|&&x| x == 0).count(),
        _ => mesh.faces.len(),
    }
}

/// A symmetric part decimated (decision 0034, section 5): cut at the plane, the positive half
/// decimated to half the target, mirrored and joined at the plane points, and repaired. The
/// result holds the mirror map unless repair had to change the joined mesh.
fn symmetric(
    cx: &Ctx,
    part: &str,
    mesh: &PlainMesh,
    axis: usize,
    settings: Settings,
) -> Result<geo::Decimated, CommandError> {
    let diagonal = cx.doc.part_diagonal(part)?;
    let axis_name = ["x", "y", "z"][axis];
    if let Some(i) = unpaired(&mesh.positions, axis, SYMMETRIC * diagonal) {
        return Err(errors::unavailable(
            Unavailable::new(format!(
                "part '{part}' is not symmetric across {axis_name}: point {i} has no mirror \
                 partner within 0.01% of its size"
            ))
            .fix("decimate without symmetry"),
        ));
    }
    let cut = symmetrize(mesh, axis, Side::Positive, SNAP * diagonal).ok_or_else(|| {
        errors::unavailable(
            Unavailable::new(format!(
                "no face of part '{part}' lies on the positive side"
            ))
            .fix("decimate without symmetry"),
        )
    })?;
    let kept = first_half(cut.mesh);
    let half = Settings {
        faces: settings.faces.div_ceil(2),
        // The cut made points of its own: the paint is read again from the half kept.
        paint: if settings.paint.is_empty() {
            Vec::new()
        } else {
            paint(&kept)
        },
        ..settings
    };
    let mut out = geo::decimate(&kept, &half);
    let (whole, partner) = mirror(&out.mesh, axis, &half.corners);
    let (joined, fixed, points) = repair(&whole);
    let same = fixed == Repairs::default() && points.len() == partner.len();
    out.mesh = joined;
    if same {
        out.mesh.attributes.insert(
            gs_core::mirror::def(axis).name,
            Attribute {
                domain: AttrDomain::Vertex,
                values: AttrValues::U32(partner),
            },
        );
    }
    out.repairs = Repairs {
        duplicate_faces: out.repairs.duplicate_faces + fixed.duplicate_faces,
        crowded_edges: out.repairs.crowded_edges + fixed.crowded_edges,
        points_added: out.repairs.points_added + fixed.points_added,
    };
    Ok(out)
}

/// A symmetrized mesh's kept half: its first half of faces (the mirrored ones follow), without
/// the map. The mirrored points stay, unused, until decimate's repair drops them.
fn first_half(mut mesh: PlainMesh) -> PlainMesh {
    let n = mesh.faces.len() / 2;
    mesh.faces.truncate(n);
    mesh.attributes
        .retain(|name, _| !gs_core::mirror::is_map(name));
    for a in mesh.attributes.values_mut() {
        if a.domain == AttrDomain::Face {
            match &mut a.values {
                AttrValues::F32(v) => v.truncate(n),
                AttrValues::U32(v) => v.truncate(n),
            }
        }
    }
    mesh
}
