//! `mesh.quad_remesh` (decision 0038, catalog `geometry.quad-remesh`): a part rebuilt as quads
//! whose edges follow its form. The quads come from quadwild-bimdf, a separate program run by
//! `gs_geo::quad`; around it this checks the part, cuts and mirrors a symmetric one, holds the
//! quads to the old surface and carries columns across (decision 0037's search), and with
//! `detail` divides levels onto the old surface.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gs_core::projection::{Landing, Projection, Side as Toward, Surface, carry, landings};
use gs_core::{AttrDef, Levels, MeshStore, NodeId, Settings, Vec3};
use gs_geo::decimate::{Repairs, mirror, repair, unpaired};
use gs_geo::quad::{self, Job, Program, QuadError, Quads, bwrap_in, places};
use gs_geo::voxel::Kept;
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Cost, ErrorCode, Kind, Length, PartRef,
    PlainMesh, System,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::symmetrize::{Mirrored, Side, symmetrize};
use crate::command::PartArg;
use crate::document::{Order, Part, pinned};
use crate::errors::{self, error};
use crate::level::{is_uv_corner, refused, uv_corners};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Rebuild a part as quads whose edges follow its form (Quad Remesh): a clean base to sculpt
/// and divide.
///
/// The quads come from quadwild-bimdf, a separate program, run inside a sandbox when
/// bubblewrap is installed. Their count lands near `faces`; the part keeps its holes and
/// handles, and every point lies within one and a half quad edges of the old surface. Mask,
/// colors and other point and face values are carried from the old surface; face groups only
/// with `keep_groups`, whose borders the edges then follow; UVs never, since a new layout has
/// none. Without `detail` the part becomes the quad base and its sculpted detail is gone; with
/// it, the quads become level 0 and levels are divided onto the old surface, so the detail
/// stays. A part with detail levels is refused: freeze them (`level.freeze`), remesh, and
/// `level.unfreeze` projects them onto the quads. A part with hidden faces is refused too. One
/// undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshQuadRemesh {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The number of quads wanted, 50 to 200,000. The result lands near it.
    #[serde(default = "faces")]
    #[schemars(range(min = 50, max = 200_000))]
    pub faces: u64,
    /// Group borders become borders the edges follow, and the groups come across. Off: the
    /// result has no groups.
    #[serde(default)]
    pub keep_groups: bool,
    /// A frozen part's creases become borders the edges follow too.
    #[serde(default = "crate::command::yes")]
    pub keep_creases: bool,
    /// Remesh the positive half across this axis's plane and mirror it, so the result is
    /// exactly symmetric and holds its mirror map. The part must be symmetric already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<Axis>,
    /// Keep the sculpted detail as levels above the quads, divided toward the old face count
    /// and projected onto the old surface.
    #[serde(default)]
    pub detail: bool,
    /// How far the projection and the carried columns look: scene units, or a percentage such
    /// as "2%" (the default) of the part's bounding-box diagonal. More than 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<Length>,
}

fn faces() -> u64 {
    5000
}

/// The remeshed part.
#[derive(Serialize, JsonSchema)]
pub struct QuadRemeshed {
    /// The part.
    pub part: String,
    /// Quads made (at level 0 with `detail`).
    pub quads: u64,
    /// Points made (at level 0 with `detail`).
    pub points: u64,
    /// The quads asked for.
    pub target: u64,
    /// How many times the program's second step ran: 1, or 2 when the first count missed the
    /// target by more than 5%.
    pub passes: u32,
    /// Triangles given to the program (for a symmetric part, its half).
    pub triangles: u64,
    /// Triangles before they were decimated to 200,000, if there were more.
    pub decimated_from: Option<u64>,
    /// Points merged with a neighbour too close to keep apart.
    pub merged: u64,
    /// Triangles dropped with shells too small to remesh.
    pub shells_dropped: u64,
    /// Points removed from open rims, so the quads along them are not thin.
    pub rim_points_removed: u64,
    /// Edges the quads were told to follow: group borders and creases.
    pub sharp_edges: u64,
    /// Whether the program ran inside bubblewrap.
    pub sandboxed: bool,
    /// Levels `detail` added.
    pub levels_added: u32,
    /// Points that found nothing within the distance: with `detail`, left where the division
    /// put them; without, given no carried values.
    pub out_of_reach: u64,
    /// The farthest a quad point lies from the old surface, in scene units.
    pub largest_distance: f32,
}

/// The fewest and most quads asked for.
const FACES: std::ops::RangeInclusive<u64> = 50..=200_000;
/// How far the projection looks when no distance is given, in percent of the diagonal.
const DISTANCE: f32 = 2.0;
/// The most divisions `detail` makes. It stops at the count closest to the old one.
const DIVISIONS: usize = 8;
/// How far a quad point may lie from the old surface, in mean quad edges.
const GATE: f32 = 1.5;
/// Points this close to a symmetry plane, as a fraction of the diagonal, count as on it
/// (`mesh.symmetrize`'s snap).
const SNAP: f32 = 1e-6;
/// How far a point may be from its partner's mirror image for the part to count as symmetric,
/// as a fraction of its diagonal: `symmetry.map`'s default 0.01%.
const SYMMETRIC: f32 = 1e-4;
/// Runs so far in this process, naming each job folder.
static RUNS: AtomicU64 = AtomicU64::new(0);

impl Command for MeshQuadRemesh {
    const NAME: &'static str = "mesh.quad_remesh";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    // Replaces only its own part. No support claim: the whole part is new.
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = QuadRemeshed;

    fn examples() -> Vec<Example<Self>> {
        examples()
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        program().map(drop)
    }

    fn run(self, cx: &mut Ctx) -> Result<QuadRemeshed, CommandError> {
        let name = pinned(self.part.clone())?;
        cx.doc.single_level_or_frozen(&name, "quad remesh")?;
        let target = self.target()?;
        let reach = self.reach(cx, &name)?;
        let part = cx.doc.get_part(&name)?;
        self.usable(&name, part)?;
        let creases = if self.keep_creases {
            creases(part)
        } else {
            Vec::new()
        };
        let source = part.store().share();
        let program = program().map_err(errors::unavailable)?;
        // In the part's own space, where its symmetry planes are (decision 0029, section 7).
        let (mesh, t) = (cx.doc.local(&name)?, cx.doc.transform(&name)?);
        let settings = quad::Settings {
            faces: target,
            keep_groups: self.keep_groups,
            creases,
            plane: None,
        };
        cx.progress(None, format!("quad remeshing {name}"));
        let made = match self.symmetry {
            None => Made {
                quads: remeshed(cx, &mesh, &settings, &program)?,
                map: None,
            },
            Some(axis) => symmetric(cx, &name, &mesh, axis as usize, &settings, &program)?,
        };
        let surface = Surface::of(&source, false);
        let largest = farthest(&surface, &made.quads)?;
        let mut reply = reply(&name, &made.quads, &program, self.faces);
        reply.largest_distance = t.length(largest);
        let mut mesh = made.quads.mesh;
        // Detail levels hold no mirror map (decision 0021, section 10).
        if let (Some(axis), Some(partner), false) = (self.symmetry, made.map, self.detail) {
            let map = Attribute {
                domain: AttrDomain::Vertex,
                values: AttrValues::U32(partner),
            };
            (mesh.attributes).insert(gs_core::mirror::def(axis as usize).name, map);
        }
        // A new surface in the part's place, keeping its cell size and any frozen levels.
        cx.doc.remesh_in_place(&name, mesh, (None::<Kept>, t))?;
        let how = Projection {
            keep: true,
            ..Projection::plain(&surface, reach)
        };
        let defs = self.carried(&source);
        let from = (&source, defs.as_slice());
        if self.detail {
            cx.progress(None, format!("dividing {name} onto its old surface"));
            let faces = source.face_count();
            (reply.out_of_reach, reply.levels_added) = divided(cx, &name, &how, from, faces)?;
        } else {
            reply.out_of_reach = carried_onto(cx, &name, &how, from)?;
        }
        Ok(reply)
    }
}
register_command!(MeshQuadRemesh, "Quad remesh");

impl MeshQuadRemesh {
    /// The quads asked for, refused outside [`FACES`].
    fn target(&self) -> Result<usize, CommandError> {
        if FACES.contains(&self.faces) {
            return Ok(usize::try_from(self.faces).unwrap_or(usize::MAX));
        }
        let (fewest, most) = (FACES.start(), FACES.end());
        Err(errors::args_invalid(
            "/faces",
            format!("faces is {fewest} to {most}, not {}", self.faces),
        ))
    }

    /// How far the projection and the carry look, in the part's own units.
    fn reach(&self, cx: &Ctx, name: &str) -> Result<f32, CommandError> {
        let reach = (cx.length(name, self.distance.unwrap_or(Length::Percent(DISTANCE)))?).part;
        if reach > 0.0 && reach.is_finite() {
            Ok(reach)
        } else {
            Err(errors::args_invalid(
                "/distance",
                "the distance must be more than 0 and finite",
            ))
        }
    }

    /// Refuses a part with hidden faces, `detail` on a frozen part, and a part of fewer than 4
    /// faces (decision 0038, section 6).
    fn usable(&self, name: &str, part: &Part) -> Result<(), CommandError> {
        let store = part.store();
        let (why, fix) = if store.any_hidden() {
            (
                format!("part '{name}' has hidden faces, and quad remesh works on a whole part"),
                "show them with visibility.show_all".to_string(),
            )
        } else if self.detail && part.frozen().is_some() {
            (
                format!("part '{name}' is frozen, and its detail is in the frozen levels"),
                "quad remesh without detail, then level.unfreeze projects the frozen levels \
                 onto the quads"
                    .to_string(),
            )
        } else if store.face_count() < 4 {
            (
                format!(
                    "part '{name}' has {} faces, and quad remesh needs at least 4",
                    store.face_count()
                ),
                "remesh a part with more faces".to_string(),
            )
        } else {
            return Ok(());
        };
        Err(errors::unavailable(Unavailable::new(why).fix(fix)))
    }

    /// The columns to carry from the old surface: all but UVs, mirror maps, hidden faces and,
    /// without `keep_groups`, face groups.
    fn carried(&self, source: &MeshStore) -> Vec<AttrDef> {
        let skip = |name: &str| {
            is_uv_corner(name)
                || gs_core::mirror::is_map(name)
                || name == AttrDef::hide().name
                || (!self.keep_groups && name == AttrDef::face_group().name)
        };
        (source.attrs().iter())
            .filter(|d| !skip(&d.name))
            .cloned()
            .collect()
    }
}

/// The Quad Remesh program, looked for as decision 0038 section 1 says, or why it is missing
/// and how to get it on this system.
fn program() -> Result<Program, Unavailable> {
    let env = std::env::var_os;
    let exe = std::env::current_exe().ok();
    let beside = exe.as_deref().and_then(Path::parent);
    let cache = gs_protocol::cache_dir().ok();
    let at = places(env("GS_QUADWILD"), beside, cache.as_deref());
    Program::find(&at, bwrap_in(env("PATH"))).map_err(|why| {
        Unavailable::new(format!("the Quad Remesh program is missing ({why})"))
            .fix(quad::fix(System::HERE, cache.as_deref()))
    })
}

/// A fresh job folder: `SESSION.scratch/quad-PID-N` in the session folder, or in the system's
/// temp folder when there is no session folder.
fn job_dir(session: &str) -> PathBuf {
    let leaf = format!(
        "quad-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    );
    match gs_protocol::session_dir() {
        Ok(dir) => dir.join(format!("{session}.scratch")).join(leaf),
        Err(_) => std::env::temp_dir().join("gigasculpt").join(leaf),
    }
}

/// `mesh` remeshed by the program in a job folder of its own, removed after.
fn remeshed(
    cx: &Ctx,
    mesh: &PlainMesh,
    settings: &quad::Settings,
    program: &Program,
) -> Result<Quads, CommandError> {
    let job = Job::new(job_dir(&cx.config.session)).map_err(|e| failed(&e))?;
    quad::remesh(mesh, settings, program, &job, &|| cx.cancelled()).map_err(|e| failed(&e))
}

/// The command error for a run that made no quads.
fn failed(e: &QuadError) -> CommandError {
    match e {
        QuadError::Cancelled => error(ErrorCode::Cancelled, "quad remesh was cancelled"),
        QuadError::TimedOut => CommandError {
            hint: Some("ask for fewer faces, or remesh a lighter part".into()),
            ..error(ErrorCode::Timeout, e.to_string())
        },
        _ => errors::unavailable(
            Unavailable::new(format!("quad remesh made no usable quads: {e}")).fix(
                if matches!(e, QuadError::Killed { .. }) {
                    "run it again; if it is killed again, free memory or remesh a lighter part"
                } else {
                    "check the part with mesh.validate, or ask for another face count"
                },
            ),
        ),
    }
}

/// The quads, and for a symmetric part the mirror partner of each point when the halves
/// joined with nothing to repair.
struct Made {
    quads: Quads,
    map: Option<Vec<u32>>,
}

/// A symmetric part remeshed (decision 0038, section 5): cut at the plane, the positive half
/// remeshed to half the target with its rim snapped onto the plane, mirrored and joined at the
/// plane points, and repaired.
fn symmetric(
    cx: &Ctx,
    name: &str,
    mesh: &PlainMesh,
    axis: usize,
    settings: &quad::Settings,
    program: &Program,
) -> Result<Made, CommandError> {
    let diagonal = cx.doc.part_diagonal(name)?;
    let plane = ["x", "y", "z"][axis];
    let refuse = |why: String| {
        errors::unavailable(Unavailable::new(why).fix("quad remesh without symmetry"))
    };
    if let Some(i) = unpaired(&mesh.positions, axis, SYMMETRIC * diagonal) {
        return Err(refuse(format!(
            "part '{name}' is not symmetric across {plane}: point {i} has no mirror partner \
             within 0.01% of its size"
        )));
    }
    let cut = symmetrize(mesh, axis, Side::Positive, SNAP * diagonal).ok_or_else(|| {
        refuse(format!(
            "no face of part '{name}' lies on the positive side"
        ))
    })?;
    let half = kept_half(cut);
    let half_settings = quad::Settings {
        faces: settings.faces.div_ceil(2),
        creases: across(&settings.creases, mesh, &half, axis, SNAP * diagonal),
        plane: Some(axis),
        keep_groups: settings.keep_groups,
    };
    let quads = remeshed(cx, &half, &half_settings, program)?;
    let (whole, partner) = mirror(&quads.mesh, axis, &[]);
    let (joined, fixed, points) = repair(&whole);
    if let (Ok(before), Ok(after)) = (quad::shape(mesh), quad::shape(&joined))
        && after.loops > before.loops
    {
        return Err(failed(&QuadError::Check(format!(
            "the two halves did not join along the {plane} plane: the result has {after}, the \
             part {before}"
        ))));
    }
    let map = (fixed == Repairs::default() && points.len() == partner.len()).then_some(partner);
    Ok(Made {
        quads: Quads {
            mesh: joined,
            ..quads
        },
        map,
    })
}

/// The kept half of a symmetrized mesh: its points come first and its faces first, the mirrored
/// copies after; the map goes.
fn kept_half(cut: Mirrored) -> PlainMesh {
    let Mirrored { mut mesh, on_plane } = cut;
    let faces = mesh.faces.len() / 2;
    let points = usize::midpoint(mesh.positions.len(), on_plane);
    mesh.faces.truncate(faces);
    mesh.positions.truncate(points);
    (mesh.attributes).retain(|name, _| !gs_core::mirror::is_map(name));
    for column in mesh.attributes.values_mut() {
        let n = match column.domain {
            AttrDomain::Face => faces,
            AttrDomain::Vertex => points,
        };
        match &mut column.values {
            AttrValues::F32(v) => v.truncate(n),
            AttrValues::U32(v) => v.truncate(n),
        }
    }
    mesh
}

/// `creases`, point pairs of `mesh`, as point pairs of `half`, its kept half. An old point keeps
/// its position in the half (onto the plane when within `snap` of it, as the cut snaps it), so
/// each end is found by its position; a crease with an end not in the half goes.
fn across(
    creases: &[[u32; 2]],
    mesh: &PlainMesh,
    half: &PlainMesh,
    axis: usize,
    snap: f32,
) -> Vec<[u32; 2]> {
    if creases.is_empty() {
        return Vec::new();
    }
    let mut at: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    for (i, p) in half.positions.iter().enumerate() {
        at.entry(p.map(f32::to_bits)).or_insert(i as u32);
    }
    let find = |v: u32| {
        let mut p = *mesh.positions.get(v as usize)?;
        if p[axis].abs() <= snap {
            p[axis] = 0.0;
        }
        at.get(&p.map(f32::to_bits)).copied()
    };
    (creases.iter())
        .filter_map(|&[a, b]| Some([find(a)?, find(b)?]))
        .collect()
}

/// A frozen part's base creases as point pairs of its mesh, when its base is still the
/// record's (the same numbering, so not remeshed since it was frozen); none otherwise. A part
/// of one level has none: creases live on a level stack (decision 0014).
fn creases(part: &Part) -> Vec<[u32; 2]> {
    let (Some(record), Order::Given(own)) = (part.frozen(), part.order()) else {
        return Vec::new();
    };
    if !Arc::ptr_eq(&own, record.level_numbering(0)) {
        return Vec::new();
    }
    (record.base_creases().iter())
        .map(|&(a, b, _)| [a, b])
        .collect()
}

/// The farthest any quad point lies from `surface`, refusing quads with a point farther than
/// [`GATE`] mean quad edges (decision 0038, section 3, step 10).
fn farthest(surface: &Surface, quads: &Quads) -> Result<f32, CommandError> {
    let reach = GATE * quads.edge;
    let points = &quads.mesh.positions;
    let found: Vec<Option<f32>> = gs_par::map(points.len(), |i| {
        let p = Vec3::from_array(points[i]);
        let hit = surface.find(p, Vec3::ZERO, reach, Toward::Both, 0.0);
        hit.map(|h| (h.point - p).len2().sqrt())
    });
    if let Some(i) = found.iter().position(Option::is_none) {
        return Err(failed(&QuadError::Check(format!(
            "quad point {i} lies more than {GATE} quad edges from the old surface"
        ))));
    }
    Ok(found.into_iter().flatten().fold(0.0, f32::max))
}

/// The reply for `quads`, before the projection and carry fill in theirs.
fn reply(name: &str, quads: &Quads, program: &Program, target: u64) -> QuadRemeshed {
    let prep = quads.preparation;
    QuadRemeshed {
        part: name.into(),
        quads: quads.mesh.faces.len() as u64,
        points: quads.mesh.positions.len() as u64,
        target,
        passes: quads.passes,
        triangles: prep.triangles,
        decimated_from: prep.decimated_from,
        merged: prep.merged,
        shells_dropped: prep.shells_dropped,
        rim_points_removed: prep.rim_points_removed,
        sharp_edges: prep.sharp_edges,
        sandboxed: program.sandboxed(),
        levels_added: 0,
        out_of_reach: 0,
        largest_distance: 0.0,
    }
}

/// The old surface and the columns to carry from it.
type Carried<'a> = (&'a MeshStore, &'a [AttrDef]);

/// Writes every column of `defs` from `source` into `store` where its points `landed`. The
/// store is new and holds none of them, so a refusal is the engine's fault.
fn put_columns(
    store: &mut MeshStore,
    how: &Projection,
    (source, defs): Carried,
    landed: &[Vec<Option<Landing>>],
) -> Result<Vec<NodeId>, CommandError> {
    let mut nodes = Vec::new();
    for def in defs {
        let written = carry(store, how, source, def, landed);
        nodes
            .extend(written.map_err(|e| errors::internal(&format!("carrying {}: {e}", def.name)))?);
    }
    nodes.sort_unstable();
    nodes.dedup();
    Ok(nodes)
}

/// Without `detail`: the columns carried onto the quads, each point looking along its normal
/// and then for the closest point, as Project Detail looks, without moving. Returns the points
/// that found nothing.
fn carried_onto(
    cx: &mut Ctx,
    name: &str,
    how: &Projection,
    from: Carried,
) -> Result<u64, CommandError> {
    if from.1.is_empty() {
        return Ok(0);
    }
    let mut part = cx.doc.write_part(name)?;
    part.sculpt(|store| {
        let landed = landings(store, how);
        let live: usize = (0..store.node_count() as NodeId)
            .map(|n| store.node(n).pos.iter().filter(|p| !p.is_dead()).count())
            .sum();
        let found = landed.iter().flatten().filter(|l| l.is_some()).count();
        let written = put_columns(store, how, from, &landed);
        let nodes = written.as_ref().cloned().unwrap_or_default();
        (written.map(|_| (live - found) as u64), nodes)
    })
}

/// With `detail` (decision 0038, section 4): the quads, now the part, become level 0 of a stack
/// divided toward `faces`, the old face count, every point projected onto the old surface
/// after each division, and the columns carried at the top. Returns the points the last
/// projection left where they were, and the levels added.
fn divided(
    cx: &mut Ctx,
    name: &str,
    how: &Projection,
    from: Carried,
    faces: usize,
) -> Result<(u64, u32), CommandError> {
    let part = cx.doc.get_part(name)?;
    let props = part.props();
    let plain = Settings {
        creases: Vec::new(),
        pin_corners: false,
        corners: uv_corners(),
    };
    let base = part.to_levels(&plain).map_err(|e| refused(&e))?;
    let (mut levels, last) =
        Levels::divided_onto(base, how, faces, DIVISIONS).map_err(|e| refused(&e))?;
    put_columns(levels.store_mut(), how, from, &last.landed)?;
    let added = u32::try_from(levels.count() - 1).unwrap_or(u32::MAX);
    cx.doc.replace_part(name, Part::leveled(levels, props));
    Ok((last.out_of_reach as u64, added))
}

fn examples() -> Vec<Example<MeshQuadRemesh>> {
    let to = |faces| MeshQuadRemesh {
        part: None,
        faces,
        keep_groups: false,
        keep_creases: true,
        symmetry: None,
        detail: false,
        distance: None,
    };
    let sphere = |n: u32| json!({ "subdivisions": n });
    vec![
        Example::new("remesh a sphere into about 200 quads", to(200))
            .after("primitive.sphere", sphere(12)),
        Example::new(
            "remesh a sphere into about 200 quads, mirrored across x",
            MeshQuadRemesh {
                symmetry: Some(Axis::X),
                ..to(200)
            },
        )
        .after("primitive.sphere", sphere(12)),
        Example::new(
            "remesh a sphere into about 100 quads, keeping its surface as levels above them",
            MeshQuadRemesh {
                detail: true,
                distance: Some(Length::Percent(5.0)),
                ..to(100)
            },
        )
        .after("primitive.sphere", sphere(16)),
        Example::new("a part with detail levels is refused", to(200))
            .after("primitive.sphere", sphere(4))
            .after("level.subdivide", json!({}))
            .fails_with(ErrorCode::Precondition),
    ]
}
