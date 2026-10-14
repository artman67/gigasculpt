//! `level.freeze` and `level.unfreeze` (decision 0030).

use gs_core::{Domain, Levels, MeshStore, NewValues, NodeId, Settings};
use gs_protocol::{AttrDomain, CommandError, Cost, Kind, Length, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{LevelState, refused, uv_corners};
use crate::command::PartArg;
use crate::ctx::PartLength;
use crate::document::{Part, pinned, scene_diagonal};
use crate::errors;
use crate::mesh::digest;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// The projection distance when none is given, in percent of the old top's diagonal.
const DISTANCE: f32 = 2.0;

/// Set a part's detail levels aside and leave its base level, so topology tools such as
/// `mesh.voxel_remesh` can run on it (Freeze Subdivision Levels). Pending edits on a lower
/// level are carried up first. The levels are kept as they were, sharing their memory with the
/// part, until `level.unfreeze` divides the base back up and projects them onto it. One undo
/// step.
///
/// Refused for a part with one level or one frozen already. While frozen, `voxel remesh` keeps
/// the levels; split, merge, mirror copy, symmetrize, reorder, the other level commands and
/// Live Detail refuse the part until it is unfrozen.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelFreeze {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for LevelFreeze {
    const NAME: &'static str = "level.freeze";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        vec![super::subdivided_cube(
            "freeze a subdivided cube's levels",
            Self { part: None },
        )]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::check(cx, self.part.as_ref(), to_freeze)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let levels = to_freeze(&name, part).map_err(errors::unavailable)?;
        let (record, mut base) = (levels.share(), levels.share());
        let carried = base.set(0).map_err(|e| refused(&e))?;
        base.delete_higher();
        let next = Part::leveled(base, part.props()).with_frozen(Some(record));
        let state = LevelState::of(&name, &next, carried.points);
        cx.doc.replace_part(&name, next);
        Ok(state)
    }
}
register_command!(LevelFreeze, "Freeze levels");

/// The levels `level.freeze` sets aside from part `name`; refused for a part frozen already or
/// with one level.
fn to_freeze<'a>(name: &str, part: &'a Part) -> Result<&'a Levels, Unavailable> {
    if part.frozen().is_some() {
        let why = format!("part '{name}' is frozen already");
        return Err(Unavailable::new(why).fix("run level.unfreeze first"));
    }
    part.levels().ok_or_else(|| {
        let why = format!("part '{name}' has one level: there are no levels to freeze");
        Unavailable::new(why).fix("level.subdivide adds levels")
    })
}

/// The levels `level.freeze` set aside from part `name`; refused for a part not frozen.
fn frozen<'a>(name: &str, part: &'a Part) -> Result<&'a Levels, Unavailable> {
    part.frozen().ok_or_else(|| {
        Unavailable::new(format!("part '{name}' is not frozen"))
            .fix("level.freeze sets a part's levels aside")
    })
}

/// Bring back the detail levels `level.freeze` set aside (Freeze Subdivision Levels, the second
/// click), one of three ways:
///
/// - **restored**: the base is as freeze left it, so the levels come back exactly.
/// - **carried**: the base kept its faces and face columns, and only its points or point
///   columns changed: a stroke, or a mask set, painted or cleared. The levels come back at
///   level 1 with those changes as its edits, carried up at the next level change; a point
///   column cleared everywhere (the mask) is cleared on every level.
/// - **projected**: anything else (a voxel remesh, say, and also groups or hidden faces changed
///   on the base; change those after unfreezing to keep the levels exact). The base becomes
///   level 1 and is divided, smooth, until its face count is closest to the old top's, not at
///   all when it is closest already (a remesh as dense as the old top: the part then has one
///   level); after each division, or once with none, every point looks along its normal both
///   ways and moves to the nearer hit on the old top within `distance`, or where neither
///   direction hits, to the closest point within it; points with nothing in reach keep their
///   place. Every point column (the color, the mask, the UV hints) then comes from the old top
///   where each point landed, so paint finer than the base comes back too; points with nothing
///   in reach, and a column the old top does not hold, keep the base's. Creases do not carry
///   over to a new base. The base's UVs are divided as the frozen levels divided theirs:
///   smoothed over the steps that smoothed them (`level.describe`'s `own_uvs`), linear over
///   the others.
///
/// One undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelUnfreeze {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// How far a point may move onto the old surface, when projecting: scene units, or a
    /// percentage such as "2%" of the old top's bounding-box diagonal. More than 0: at 0 no
    /// point could move, and the frozen detail would be lost. Default 2%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<Length>,
}

/// How `level.unfreeze` brought the levels back.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Back {
    /// The base was as freeze left it: the levels came back exactly.
    Restored,
    /// Only the base's points or point columns changed: they are level 1's edits, carried up
    /// at the next level change.
    Carried,
    /// The base was divided and the old top projected onto it.
    Projected,
}

/// A part after `level.unfreeze`.
#[derive(Serialize, JsonSchema)]
pub struct Unfrozen {
    /// The part's levels now.
    pub levels: LevelState,
    /// How they came back.
    pub how: Back,
    /// Points of the new top that the last projection moved onto the old surface (0 unless
    /// projected).
    pub projected: u64,
}

impl Command for LevelUnfreeze {
    const NAME: &'static str = "level.unfreeze";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    type Output = Unfrozen;

    fn examples() -> Vec<Example<Self>> {
        let back = |summary| {
            super::subdivided_cube(
                summary,
                Self {
                    part: None,
                    distance: None,
                },
            )
            .after("level.freeze", json!({}))
        };
        vec![
            back("bring frozen levels back unchanged"),
            back("project frozen levels onto a remeshed base")
                .after("mesh.voxel_remesh", json!({ "resolution": 16 })),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::check(cx, self.part.as_ref(), frozen)
    }

    fn run(self, cx: &mut Ctx) -> Result<Unfrozen, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let record = frozen(&name, part).map_err(errors::unavailable)?;
        let distance = self.distance.unwrap_or(Length::Percent(DISTANCE));
        let (Length::Units(number) | Length::Percent(number)) = distance;
        // Review 0007, S1: at 0 the levels came back as the divided base, the detail gone.
        if !(number > 0.0 && number.is_finite()) {
            return Err(too_short());
        }
        let props = part.props();
        let (next, how, projected) = if let Some((levels, how)) = unchanged_faces(part, record)? {
            (Part::leveled(levels, props), how, 0)
        } else {
            // In the part's own units, a percentage of the old top's diagonal in the scene
            // (review 0008, m1: scene units were used as part units).
            let t = part.transform();
            let reach = PartLength::new(distance, t, || scene_diagonal(record.top(), t)).part;
            if !reach.is_finite() {
                return Err(too_short());
            }
            let settings = Settings {
                creases: Vec::new(),
                pin_corners: record.pin_corners(),
                corners: uv_corners(),
            };
            let base = part.to_levels(&settings).map_err(|e| refused(&e))?;
            cx.progress(
                None,
                format!("dividing {name}'s base and projecting its levels"),
            );
            // The base's UVs, smoothed again over the steps the frozen levels smoothed them.
            let uvs = base.active_uvs();
            let own: Vec<bool> = record.info().iter().map(|i| i.own_uvs).collect();
            let (mut levels, moved) =
                Levels::unfrozen(base, record, reach).map_err(|e| refused(&e))?;
            if let Some(uvs) = uvs {
                levels
                    .write_uvs_smoothing(uvs, &own)
                    .map_err(|e| refused(&e))?;
            }
            (Part::leveled(levels, props), Back::Projected, moved)
        };
        let levels = LevelState::of(&name, &next, 0);
        cx.doc.replace_part(&name, next);
        Ok(Unfrozen {
            levels,
            how,
            projected: projected as u64,
        })
    }
}
register_command!(LevelUnfreeze, "Unfreeze levels");

/// The refusal of a distance at which no point could move onto the old top. No command throws
/// a frozen record away on purpose, so there is none to name.
fn too_short() -> CommandError {
    crate::errors::args_invalid(
        "/distance",
        "the distance must be more than 0: at 0 no point could move onto the frozen levels, \
         and their detail would be lost",
    )
}

/// The record back as it was (restored) or with the base's point changes as level 1's edits
/// (carried), when `part`'s base kept the record base's faces, face columns and their names;
/// `None` when it did not. A point column the base gained (a first mask) is registered on
/// level 1 and loaded with the rest; one it lost, which no point of the base holds any more (a
/// mask cleared everywhere), is released on every level first, as clearing it on level 1 does
/// (review 0007, M2).
fn unchanged_faces(part: &Part, record: &Levels) -> Result<Option<(Levels, Back)>, CommandError> {
    let mut held = record.share();
    held.set(0).map_err(|e| refused(&e))?;
    let was = Part::leveled(held.share(), part.props()).view();
    let now = part.view();
    if digest(&was) == digest(&now) {
        return Ok(Some((record.share(), Back::Restored)));
    }
    if was.positions.len() != now.positions.len() || shape(&was) != shape(&now) {
        return Ok(None);
    }
    let lost: Vec<&String> = (was.attributes.keys())
        .filter(|name| !now.attributes.contains_key(*name))
        .collect();
    if !lost.is_empty() {
        held.set(held.top_level()).map_err(|e| refused(&e))?;
        for name in lost {
            release(held.store_mut(), name);
        }
        held.set(0).map_err(|e| refused(&e))?;
    }
    let level = held.store_mut();
    for def in part.store().attrs() {
        let gained = def.domain == Domain::Vertex && now.attributes.contains_key(&def.name);
        if gained && level.add_attr(def.clone()).is_err() {
            return Ok(None);
        }
    }
    Ok(held.load_active(&now).ok().map(|()| (held, Back::Carried)))
}

/// Releases point column `name` on every node of `store` that holds it, so it reads as its empty
/// value (zero, white for a color)
/// everywhere, as `mask.set` with 0 releases the mask.
fn release(store: &mut MeshStore, name: &str) {
    let Some(id) = store.attr_id(name) else {
        return;
    };
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId)
        .filter(|&n| store.node(n).attr(id).is_some())
        .collect();
    store.put_attr(id, &nodes, vec![NewValues::Zeros; nodes.len()]);
}

/// The hash of what a point edit never changes: the faces and the face columns with their
/// names. Point columns are left out, names included: one gained or lost is a point change.
fn shape(m: &PlainMesh) -> blake3::Hash {
    let attributes = (m.attributes.iter())
        .filter(|(_, a)| a.domain == AttrDomain::Face)
        .map(|(name, a)| (name.clone(), a.clone()))
        .collect();
    digest(&PlainMesh {
        positions: Vec::new(),
        faces: m.faces.clone(),
        attributes,
    })
}
