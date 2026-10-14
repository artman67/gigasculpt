//! `mesh.voxel_remesh`: Voxel Remesh (catalog `geometry.voxel-remesh`), rebuilding a part as
//! an evenly spaced surface of quads. The work is `gs_geo::voxel::remesh_holes`, a pure
//! function.

use gs_core::math::{Vec3, closest_on_triangle};
use gs_core::{AttrDef, AttrValue};
use gs_geo::voxel::{
    CellSize, CloseHoles, RemeshError, Settings, SignMethod, Sources, remesh_holes,
};
use gs_io::mesh::UV_COLUMNS;
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Cost, ErrorCode, Kind, Length, PartRef,
    PlainMesh, arity, face_triangles,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::pinned;
use crate::errors::{self, error};
use crate::level::is_uv_corner;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Rebuild a part as a new, evenly spaced mesh of quads, closing holes and merging
/// overlapping pieces into one shell.
///
/// The surface is turned into a signed distance field on a grid and meshed again (Surface
/// Nets), so the result is closed and manifold with cells of one size everywhere. A cavity whose
/// faces point inward stays hollow; a hole gets a patch across its rim, or with `close_holes` 0
/// stays open about where its rim was. Detail smaller than a cell is lost unless `project` is
/// on. Mask, color and other columns are carried across by
/// their kind, so the mask stays where it was and strokes still leave the masked area alone;
/// surface made to close a hole is unmasked. Face groups come across only with `keep_groups`.
/// UVs do not survive. A part with detail levels is refused; freeze them (`level.freeze`) to
/// remesh the base and keep them for `level.unfreeze`.
///
/// The part keeps its cell size: a remesh with neither `resolution` nor `voxel_size` uses the
/// last one's, so a part sculpted bigger gets more faces, not bigger ones. The grid lies in the
/// part's own space, so it follows the part when it is moved or turned; baking the part's place
/// first (`part.apply_transform`) lays it on the scene axes instead, which can change the result.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshVoxelRemesh {
    #[doc = crate::command::part_doc!("The part to remesh.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Cells along the longest side of the part's bounding box, 16 to 2048. Higher keeps more
    /// detail and makes more faces. Sets the cell size the part keeps. Left out (with no
    /// `voxel_size`): the kept cell size, while the longest side fits in 1024 cells (or the
    /// resolution it was set at, if higher), else that many cells; 128 on a part never remeshed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 16, max = 2048))]
    pub resolution: Option<u32>,
    /// Cell size instead of a resolution (and used if both are given): scene units, or a
    /// percentage such as "1%" of the part's bounding-box diagonal. The part keeps it like a
    /// resolution's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voxel_size: Option<Length>,
    /// Keep face groups (each new face takes the group of the old face nearest it). Off: the
    /// result has no groups.
    #[serde(default)]
    pub keep_groups: bool,
    /// Move the new surface onto the old one, keeping detail finer than a cell.
    #[serde(default)]
    pub project: bool,
    /// 0 to 100: soften the projected detail. Only used with `project`.
    #[serde(default)]
    #[schemars(range(min = 0, max = 100))]
    pub blur: f32,
    /// Hole closing: 0, none (the inside is still read with each hole capped, then the surface
    /// across it is removed, so the part stays open about its rim), or 1, large triangles (each
    /// hole closed by a patch across its rim). Left out: the session's hole closing
    /// (`mesh.voxel_settings`, 1 at first), written into the logged call. A closed part
    /// remeshes the same either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: Option<u8>,
}

/// The resolution of a part's first remesh, when none is given.
pub(crate) const FIRST_RESOLUTION: u32 = 128;

/// Where the inside of the old surface was read from.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Inside {
    /// The part was closed: inside was read exactly.
    Winding,
    /// The part had holes: each was capped across its rim, then inside was read exactly.
    Capped,
    /// Some edge was shared by three or more faces, or by two wound the same way: holes were
    /// closed over where the surface folds round them.
    GeneralizedWinding,
}

/// The remeshed part.
#[derive(Serialize, JsonSchema)]
pub struct VoxelRemeshed {
    /// The part.
    pub part: String,
    /// Cell size used, in scene units.
    pub voxel_size: f32,
    /// Vertices of the new surface.
    pub vertices: u64,
    /// Faces of the new surface (all quads).
    pub faces: u64,
    /// How inside and outside were told apart.
    pub inside: Inside,
}

impl Command for MeshVoxelRemesh {
    const NAME: &'static str = "mesh.voxel_remesh";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    // Replaces only its own part. No support claim: the whole part is new.
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = VoxelRemeshed;

    fn examples() -> Vec<Example<Self>> {
        let plain = |part: Option<&str>, resolution| Self {
            part: part.map(|p| PartRef(p.into())),
            resolution,
            voxel_size: None,
            keep_groups: false,
            project: false,
            blur: 0.0,
            close_holes: None,
        };
        let projected = Self {
            part: Some(PartRef("cube".into())),
            resolution: None,
            voxel_size: Some(Length::Percent(5.0)),
            keep_groups: false,
            project: true,
            blur: 20.0,
            close_holes: None,
        };
        let tetrahedron = json!({
            "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
            "faces": [
                [0, 2, 1, 4_294_967_295_u32],
                [0, 1, 3, 4_294_967_295_u32],
                [0, 3, 2, 4_294_967_295_u32],
                [1, 2, 3, 4_294_967_295_u32]
            ],
            "attributes": {"mask": {"domain": "vertex", "values": {"f32": [0, 0.5, 0, 0]}}}
        });
        let open = json!({
            "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
            "faces": [
                [0, 2, 1, 4_294_967_295_u32],
                [0, 1, 3, 4_294_967_295_u32],
                [0, 3, 2, 4_294_967_295_u32]
            ]
        });
        vec![
            Example::new("remesh a sphere at resolution 32", plain(None, Some(32)))
                .after("primitive.sphere", json!({ "subdivisions": 6 })),
            Example::raw("remesh again at the cell size the part kept", json!({}))
                .after("primitive.sphere", json!({ "subdivisions": 4 }))
                .after("mesh.voxel_remesh", json!({ "resolution": 24 })),
            Example::new(
                "remesh a cube with cells 5% of its size, projected and softened",
                projected,
            )
            .after("primitive.cube", json!({})),
            Example::new(
                "remesh a masked part, keeping its mask",
                plain(Some("masked"), Some(16)),
            )
            .after(
                "part.create",
                json!({ "name": "masked", "mesh": tetrahedron }),
            ),
            Example::raw(
                "remesh a tetrahedron missing a face, leaving its hole open",
                json!({ "part": "open", "resolution": 16, "close_holes": 0 }),
            )
            .after("part.create", json!({ "name": "open", "mesh": open })),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        self.close_holes
            .get_or_insert(super::voxel_mode::session_closing(cx));
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<VoxelRemeshed, CommandError> {
        let part = pinned(self.part)?;
        cx.doc.single_level_or_frozen(&part, "voxel remesh")?;
        // Remeshed in the part's own space; the new surface keeps its transform (decision
        // 0029, section 4), and so does the cell size it keeps.
        let (mesh, t) = (cx.doc.local(&part)?, cx.doc.transform(&part)?);
        let cell = match (self.voxel_size, self.resolution) {
            (Some(size), _) => CellSize::Size(cx.length(&part, size)?.part),
            (None, Some(n)) => CellSize::Resolution(n),
            (None, None) => (cx.doc.voxel_cell(&part)?)
                .map_or(CellSize::Resolution(FIRST_RESOLUTION), CellSize::Kept),
        };
        let settings = Settings {
            cell,
            project: self.project,
            blur: self.blur,
        };
        let close = match self.close_holes {
            Some(0) => CloseHoles::Off,
            _ => CloseHoles::Large,
        };
        cx.progress(None, format!("voxel remeshing {part}"));
        let mut out = remesh_holes(&mesh, &settings, close).map_err(|e| refusal(&e))?;
        let defs = cx.doc.get_part(&part)?.store().attrs();
        let keep = |name: &str| self.keep_groups || name != AttrDef::face_group().name;
        carry(&mesh, (&mut out.mesh, &out.sources), defs, keep);
        let reply = VoxelRemeshed {
            part: part.clone(),
            voxel_size: t.length(out.grid.voxel as f32),
            vertices: out.mesh.positions.len() as u64,
            faces: out.mesh.faces.len() as u64,
            inside: match out.sign {
                SignMethod::Winding => Inside::Winding,
                SignMethod::CappedWinding => Inside::Capped,
                SignMethod::GeneralizedWinding => Inside::GeneralizedWinding,
            },
        };
        // A new surface: the part is replaced whole, numbered in the store's own order, keeping
        // any frozen levels.
        cx.doc.remesh_in_place(&part, out.mesh, (out.kept, t))?;
        Ok(reply)
    }
}
register_command!(MeshVoxelRemesh, "Voxel remesh");

/// Carries `old`'s columns onto its remesh `new` by each column's kind in `defs` (decision 0037,
/// section 3): a vertex column blends the corners of the old triangle under each new vertex by
/// their weights, a face column copies the old face nearest each new face, and surface made to
/// close a hole gets the column's empty value (white for a color, decision 0039). The UV
/// corners never cross, since a new face has no old corners to
/// take one from (a boolean cut takes them with [`carry_uv_corners`]); nor does a name `keep`
/// refuses, or one with no definition.
pub(crate) fn carry(
    old: &PlainMesh,
    (new, sources): (&mut PlainMesh, &Sources),
    defs: &[AttrDef],
    keep: impl Fn(&str) -> bool,
) {
    for (name, column) in &old.attributes {
        let Some(def) = defs.iter().find(|d| d.name == *name) else {
            continue;
        };
        if is_uv_corner(name) || !keep(name) {
            continue;
        }
        let read = |i: u32| match &column.values {
            AttrValues::F32(v) => AttrValue::F32(v[i as usize]),
            AttrValues::U32(v) => AttrValue::U32(v[i as usize]),
        };
        let zero = AttrValue::zero(def.kind);
        let values = match column.domain {
            AttrDomain::Vertex => gs_par::map(sources.points.len(), |v| {
                sources.points[v].map_or(zero, |(corners, weights)| {
                    AttrValue::blend_weighted(def, &corners.map(read), &weights)
                })
            }),
            AttrDomain::Face => {
                gs_par::map(sources.faces.len(), |f| sources.faces[f].map_or(zero, read))
            }
        };
        let values = match column.values {
            AttrValues::F32(_) => {
                AttrValues::F32(values.iter().map(|v| f32::from_bits(bits(*v))).collect())
            }
            AttrValues::U32(_) => AttrValues::U32(values.iter().map(|v| bits(*v)).collect()),
        };
        let domain = column.domain;
        new.attributes
            .insert(name.clone(), Attribute { domain, values });
    }
}

/// Carries `old`'s UV corners onto `new`, a boolean cut of it whose face `f` came from `old`'s
/// face `faces[f]` (decision 0057, section 6), the step `carry` leaves out: each corner of a new
/// face takes the UV of its source face's corner at the same point, bit for bit, or at a point
/// the cut made, the source face's UVs blended by the weights of its nearest point in that face.
/// So a UV is only ever read in its own face, never across faces or parts. A face with no
/// source gets zeros. Runs only when `old` has every UV corner column.
pub(crate) fn carry_uv_corners(old: &PlainMesh, new: &mut PlainMesh, faces: &[Option<u32>]) {
    let column = |name: &str| match old.attributes.get(name).map(|c| (c.domain, &c.values)) {
        Some((AttrDomain::Face, AttrValues::F32(v))) => Some(v.as_slice()),
        _ => None,
    };
    let Some(uv) = UV_COLUMNS
        .iter()
        .map(|name| column(name))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let corners = |f: usize| -> [f32; 8] {
        let mut out = [0.0; 8];
        let (face, Some(s)) = (new.faces[f], faces[f]) else {
            return out;
        };
        let s = s as usize;
        for k in 0..arity(face) {
            let w = in_face(old, old.faces[s], new.positions[face[k] as usize]);
            for (c, value) in out[2 * k..2 * k + 2].iter_mut().enumerate() {
                *value = (0..4).map(|j| w[j] * uv[2 * j + c][s]).sum();
            }
        }
        out
    };
    let values = gs_par::map(new.faces.len(), corners);
    for (i, name) in UV_COLUMNS.iter().enumerate() {
        let values = AttrValues::F32(values.iter().map(|v| v[i]).collect());
        let domain = AttrDomain::Face;
        new.attributes
            .insert((*name).to_string(), Attribute { domain, values });
    }
}

/// The weights of `p` in `face` of `mesh`, by corner: all on a corner at `p` bit for bit, else
/// those of the nearest point of the face's nearer triangle (as it is drawn).
fn in_face(mesh: &PlainMesh, face: [u32; 4], p: [f32; 3]) -> [f32; 4] {
    let mut w = [0.0; 4];
    let at = |k: usize| mesh.positions[face[k] as usize];
    let bits = |q: [f32; 3]| q.map(f32::to_bits);
    if let Some(j) = (0..arity(face)).find(|&j| bits(at(j)) == bits(p)) {
        w[j] = 1.0;
        return w;
    }
    let p = Vec3::from_array(p);
    let mut best = (f32::INFINITY, [0; 3], [0.0; 3]);
    for t in face_triangles(face, at) {
        let (q, weights) = closest_on_triangle(p, &t.map(|k| Vec3::from_array(at(k))));
        let d = (q - p).len2();
        if d < best.0 {
            best = (d, *t, weights);
        }
    }
    for (k, j) in best.1.into_iter().enumerate() {
        w[j] += best.2[k];
    }
    w
}

fn bits(v: AttrValue) -> u32 {
    match v {
        AttrValue::F32(x) => x.to_bits(),
        AttrValue::U32(x) => x,
    }
}

/// The command error for a remesh that could not run.
pub(crate) fn refusal(e: &RemeshError) -> CommandError {
    match e {
        RemeshError::TooManyCells => CommandError {
            hint: Some("use a lower resolution or a larger voxel_size".into()),
            ..error(ErrorCode::Limit, e.to_string())
        },
        RemeshError::BadCellSize => errors::args_invalid("/voxel_size", e.to_string()),
        _ => errors::unavailable(
            Unavailable::new(e.to_string()).fix("check the part with mesh.validate"),
        ),
    }
}
