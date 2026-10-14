//! `mesh.project_detail`: Project Detail (catalog `topology-projection.project-detail`), moving
//! a part's points onto another part's surface and carrying columns across (decision 0037,
//! section 4). The search, the moves and the carry are gs-core's (`gs_core::projection`).

use gs_core::projection::{Landing, Projected, Projection, Side, Surface, carry, project};
use gs_core::{AttrDef, Levels, MeshStore, NodeId, Settings};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Length, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{Part, pinned};
use crate::errors::{self, error};
use crate::level::{check_thawed, is_uv_corner, refused, uv_corners};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// How far a point may move when no distance is given, in percent of the source's diagonal.
const DISTANCE: f32 = 2.0;

/// The most divisions `divide` makes. It makes the count closest to the source's faces and
/// stops looking past twice them, so this only bounds a source far denser than its target.
const MOST: usize = 8;

/// Copy the sculpted surface of one part onto another of different topology (Project Detail).
///
/// Each point of the target looks along its normal both ways and moves to the nearer hit on the
/// source within `distance`; where neither direction hits, to the closest point within it;
/// with nothing in reach it stays. A point moves `1 - mask` of the way, so fully masked points
/// stay, and so do points whose every face is hidden; hidden faces of the source are never hit.
/// The target keeps its faces and all of its own columns (UVs, groups, hidden faces). `carry`
/// copies source columns across by their kind: a point column blended from the corners of the
/// source triangle hit, a face column from the source face under each face's centre. UVs never
/// cross: they belong to the source's face corners.
///
/// The source gives its top level, with edits pending on a lower level carried up; a frozen
/// source gives its base, as it shows. A target with levels moves its top level, which becomes
/// active. With `divide`, a target of one level is divided smooth toward the source's face
/// count (at most 8 times) and projected after each division, so it gets levels. The parts are
/// matched where they stand in the scene, whatever their transforms. One undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshProjectDetail {
    #[doc = crate::command::part_doc!("The part that takes the detail (the target).")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The part the detail comes from.
    pub source: PartRef,
    /// How far a point may move: scene units, or a percentage such as "2%" of the source's
    /// bounding-box diagonal. More than 0. Defaults to 2%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<Length>,
    /// Which side of the target the source may be found on.
    #[serde(default)]
    pub side: Toward,
    /// Divide the target smooth toward the source's face count, projecting after each
    /// division. Only on a target with one level.
    #[serde(default)]
    pub divide: bool,
    /// 0 to 100: soften the moves.
    #[serde(default)]
    #[schemars(range(min = 0, max = 100))]
    pub blur: f32,
    /// Source columns to copy onto the target, by name: for example "mask", "color" or
    /// "group". A name the target lacks is created.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carry: Vec<String>,
}

/// Which side of the target the source may be found on.
#[derive(Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Toward {
    /// Either side.
    #[default]
    Both,
    /// Outside the target: along its normals.
    Outside,
    /// Inside the target: against its normals.
    Inside,
}

impl Toward {
    /// The projection's side, in the order the variants are declared.
    fn side(self) -> Side {
        [Side::Both, Side::Outside, Side::Inside][self as usize]
    }
}

/// What Project Detail did.
#[derive(Serialize, JsonSchema)]
pub struct DetailProjected {
    /// The target part.
    pub part: String,
    /// The source part.
    pub source: String,
    /// Points that moved (in the last projection, with `divide`).
    pub moved: u64,
    /// Points that found nothing within the distance and stayed.
    pub out_of_reach: u64,
    /// Levels `divide` added.
    pub levels_added: u32,
    /// The largest move, in scene units.
    pub largest_move: f32,
}

impl Command for MeshProjectDetail {
    const NAME: &'static str = "mesh.project_detail";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    // Writes only the target; the source is read.
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = DetailProjected;

    fn examples() -> Vec<Example<Self>> {
        examples()
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check_thawed(cx, self.part.as_ref(), "project detail")
    }

    fn run(self, cx: &mut Ctx) -> Result<DetailProjected, CommandError> {
        let name = pinned(self.part.clone())?;
        let source = self.source_name(cx, &name)?;
        let reach = self.reach(cx, &source)?;
        if !(0.0..=100.0).contains(&self.blur) {
            return Err(errors::args_invalid("/blur", "blur is 0 to 100"));
        }
        let src = top_of(cx.doc.get_part(&source)?)?;
        let surface = Surface::of(&src, true);
        if surface.is_empty() {
            return Err(error(
                ErrorCode::Precondition,
                format!("part '{source}' has no visible faces to project onto"),
            ));
        }
        let target = cx.doc.get_part(&name)?;
        let defs = self.carried(&src, target.store())?;
        if self.divide {
            divisible(&name, target)?;
        }
        let t = target.transform();
        let how = Projection {
            onto: &surface,
            reach,
            side: self.side.side(),
            to: t.then(&cx.doc.transform(&source)?.inverse()),
            guarded: true,
            blur: self.blur,
            keep: !defs.is_empty(),
        };
        let stack = target.levels().is_some();
        cx.progress(None, format!("projecting {source}'s detail onto {name}"));
        let (done, added) = match (self.divide, stack) {
            (true, _) => divided(cx, &name, &how, (&src, &defs))?,
            (false, true) => (on_top(cx, &name, &how, (&src, &defs))?, 0),
            (false, false) => (in_place(cx, &name, &how, (&src, &defs))?, 0),
        };
        Ok(DetailProjected {
            part: name,
            source,
            moved: done.moved as u64,
            out_of_reach: done.out_of_reach as u64,
            levels_added: added,
            largest_move: t.length(done.largest),
        })
    }
}
register_command!(MeshProjectDetail, "Project detail");

impl MeshProjectDetail {
    /// The source's name, refused when it does not exist or is the target.
    fn source_name(&self, cx: &Ctx, target: &str) -> Result<String, CommandError> {
        let Some(source) = cx.doc.resolve(&self.source.0) else {
            return Err(CommandError {
                path: Some("/source".into()),
                ..errors::not_found("part", &self.source.0, cx.doc.part_names())
            });
        };
        if cx.doc.resolve(target).as_deref() == Some(source.as_str()) {
            return Err(errors::args_invalid(
                "/source",
                format!("'{source}' is the target; the detail must come from another part"),
            ));
        }
        Ok(source)
    }

    /// How far a point may move, in the source's own units (the search's).
    fn reach(&self, cx: &Ctx, source: &str) -> Result<f32, CommandError> {
        let distance = self.distance.unwrap_or(Length::Percent(DISTANCE));
        let (Length::Units(number) | Length::Percent(number)) = distance;
        let reach = cx.length(source, distance)?.part;
        if !(number > 0.0 && number.is_finite() && reach > 0.0 && reach.is_finite()) {
            return Err(errors::args_invalid(
                "/distance",
                "the distance must be more than 0: at 0 no point could move",
            ));
        }
        Ok(reach)
    }

    /// The definitions of the columns to carry, as the source holds them; refuses a UV corner,
    /// a name the source lacks, and one `target` holds under another domain or kind.
    fn carried(&self, src: &MeshStore, target: &MeshStore) -> Result<Vec<AttrDef>, CommandError> {
        let refuse = |i: usize, why: String| errors::args_invalid(&format!("/carry/{i}"), why);
        (self.carry.iter().enumerate())
            .map(|(i, name)| {
                if is_uv_corner(name) {
                    return Err(refuse(
                        i,
                        format!(
                            "'{name}' is a UV column: UVs belong to the source's face corners \
                             and never cross to another surface; the target keeps its own"
                        ),
                    ));
                }
                let Some(def) = src.attrs().iter().find(|d| d.name == *name) else {
                    return Err(refuse(i, format!("the source has no column '{name}'")));
                };
                target
                    .attr_fits(def)
                    .map_err(|e| refuse(i, e.to_string()))?;
                Ok(def.clone())
            })
            .collect()
    }
}

/// What a part shows to project onto: its top level with pending edits carried up, or its one
/// store (a frozen part's base).
fn top_of(part: &Part) -> Result<MeshStore, CommandError> {
    let Some(levels) = part.levels() else {
        return Ok(part.store().share());
    };
    let mut levels = levels.share();
    levels.set(levels.top_level()).map_err(|e| refused(&e))?;
    Ok(levels.top().share())
}

/// Refuses `divide` on a target with levels, and on one with hidden faces, which a divide would
/// change as Subdivide would (decision 0037, section 5).
fn divisible(name: &str, target: &Part) -> Result<(), CommandError> {
    let why = if let Some(levels) = target.levels() {
        format!("part '{name}' has {} detail levels already", levels.count())
    } else if target.store().any_hidden() {
        format!("part '{name}' has hidden faces; dividing only the visible region is not built yet")
    } else {
        return Ok(());
    };
    Err(CommandError {
        hint: Some("project without divide, or show all and delete the higher levels".into()),
        path: Some("/divide".into()),
        ..error(ErrorCode::Precondition, why)
    })
}

/// The source and the columns to carry from it.
type Detail<'a> = (&'a MeshStore, &'a [AttrDef]);

/// Carries every column of `defs` from `src` where the points of `store` landed.
fn carry_all(
    store: &mut MeshStore,
    how: &Projection,
    (src, defs): Detail,
    landed: &[Vec<Option<Landing>>],
) -> Result<Vec<NodeId>, CommandError> {
    let mut nodes = Vec::new();
    for def in defs {
        let written = carry(store, how, src, def, landed);
        nodes.extend(written.map_err(|e| errors::args_invalid("/carry", e.to_string()))?);
    }
    Ok(nodes)
}

/// Projects and carries on a target of one level, in place.
fn in_place(
    cx: &mut Ctx,
    name: &str,
    how: &Projection,
    from: Detail,
) -> Result<Projected, CommandError> {
    let mut part = cx.doc.write_part(name)?;
    part.sculpt(|store| {
        let done = project(store, how);
        let mut nodes = done.nodes.clone();
        let carried = carry_all(store, how, from, &done.landed);
        nodes.extend(carried.as_deref().unwrap_or_default());
        nodes.sort_unstable();
        nodes.dedup();
        (carried.map(|_| done), nodes)
    })
}

/// Projects and carries on the top level of a target with levels, which becomes active.
fn on_top(
    cx: &mut Ctx,
    name: &str,
    how: &Projection,
    from: Detail,
) -> Result<Projected, CommandError> {
    let part = cx.doc.get_part(name)?;
    let props = part.props();
    let mut levels = (part.levels().map(Levels::share))
        .ok_or_else(|| errors::internal("the target lost its levels"))?;
    levels.set(levels.top_level()).map_err(|e| refused(&e))?;
    let done = project(levels.store_mut(), how);
    carry_all(levels.store_mut(), how, from, &done.landed)?;
    cx.doc.replace_part(name, Part::leveled(levels, props));
    Ok(done)
}

/// Divides a target of one level onto the source and carries at the final top. Returns what the
/// last projection did and the levels added.
fn divided(
    cx: &mut Ctx,
    name: &str,
    how: &Projection,
    from: Detail,
) -> Result<(Projected, u32), CommandError> {
    let part = cx.doc.get_part(name)?;
    let settings = Settings {
        creases: Vec::new(),
        pin_corners: false,
        corners: uv_corners(),
    };
    let (base, props) = (part.to_levels(&settings), part.props());
    let base = base.map_err(|e| refused(&e))?;
    let target = from.0.face_count();
    let (mut levels, done) =
        Levels::divided_onto(base, how, target, MOST).map_err(|e| refused(&e))?;
    carry_all(levels.store_mut(), how, from, &done.landed)?;
    let added = levels.count() as u32 - 1;
    cx.doc.replace_part(name, Part::leveled(levels, props));
    Ok((done, added))
}

fn examples() -> Vec<Example<MeshProjectDetail>> {
    let call = |part: &str, source: &str, distance: f32| MeshProjectDetail {
        part: Some(PartRef(part.into())),
        source: PartRef(source.into()),
        distance: Some(Length::Percent(distance)),
        side: Toward::Both,
        divide: false,
        blur: 0.0,
        carry: Vec::new(),
    };
    let tetrahedron = json!({
        "positions": [[-1, -1, -1], [1, -1, -1], [-1, 1, -1], [-1, -1, 1]],
        "faces": [
            [0, 2, 1, 4_294_967_295_u32],
            [0, 1, 3, 4_294_967_295_u32],
            [0, 3, 2, 4_294_967_295_u32],
            [1, 2, 3, 4_294_967_295_u32]
        ],
        "attributes": {"mask": {"domain": "vertex", "values": {"f32": [0, 1, 0, 0.5]}}}
    });
    let divide = MeshProjectDetail {
        divide: true,
        ..call("sphere", "cube", 30.0)
    };
    let masked = MeshProjectDetail {
        carry: vec!["mask".into()],
        ..call("sphere", "masked", 60.0)
    };
    let uv = MeshProjectDetail {
        carry: vec!["uv0.u".into()],
        ..call("sphere", "cube", 30.0)
    };
    let sphere = || json!({ "subdivisions": 4 });
    vec![
        Example::new("press a sphere onto a cube", call("sphere", "cube", 30.0))
            .after("primitive.cube", json!({ "subdivisions": 8 }))
            .after("primitive.sphere", sphere()),
        Example::new("divide a coarse sphere onto a cube, gaining levels", divide)
            .after("primitive.cube", json!({ "subdivisions": 16 }))
            .after("primitive.sphere", json!({ "subdivisions": 2 })),
        Example::new("project onto a masked part, carrying its mask", masked)
            .after(
                "part.create",
                json!({ "name": "masked", "mesh": tetrahedron }),
            )
            .after("primitive.sphere", sphere()),
        Example::new("UVs are never carried across", uv)
            .after("primitive.cube", json!({ "subdivisions": 8 }))
            .after("primitive.sphere", sphere())
            .fails_with(ErrorCode::ArgsInvalid),
    ]
}
