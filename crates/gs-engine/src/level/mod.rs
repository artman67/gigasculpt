//! Detail levels (decision 0014): `level.subdivide`, `level.set`, `level.step`,
//! `level.delete_lower`, `level.delete_higher`, `level.reconstruct`, `level.describe`, and
//! `level.freeze` and `level.unfreeze` (decision 0030).
//!
//! Level numbers count from 1, the base, as the catalog's switch spec and the level slider
//! do. Every edit here builds the changed part from a share of the old one (decision 0010) and
//! puts it in place: one undo step that swaps the whole part, so undo across level changes is
//! the pointer swap every replaced part already has. A frozen part refuses every level command
//! but `level.describe` and `level.unfreeze`.

mod delete;
mod describe;
mod freeze;
mod reconstruct;
mod subdivide;
mod switch;
mod uvs;

pub(crate) use subdivide::{Crease, LevelSubdivide};
pub use uvs::write_level_uvs;

use gs_core::{Levels, MeshError, MeshStore, NodeId};
use gs_protocol::{CommandError, ErrorCode, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::Serialize;

use crate::document::Part;
use crate::errors::{self, error};
use crate::{Ctx, Example, Unavailable};

/// The UV columns as corner groups for a level stack (gs-core `Settings::corners`): each
/// channel's four corners, so a lower level takes each corner's UV from its top descendant.
pub(crate) fn uv_corners() -> Vec<[String; 4]> {
    (0..2)
        .map(|channel| {
            std::array::from_fn(|corner| gs_io::mesh::UV_COLUMNS[corner * 2 + channel].to_string())
        })
        .collect()
}

/// Whether `name` is one of the UV corner columns. A carry across surfaces never brings them:
/// they belong to the old faces' corners (decision 0037, section 3).
pub(crate) fn is_uv_corner(name: &str) -> bool {
    gs_io::mesh::UV_COLUMNS.contains(&name)
}

/// Takes the UV corner columns off a mesh whose faces are about to change, for a command that
/// cannot keep them (decimate without `keep_uvs`).
pub(crate) fn drop_uvs(mesh: &mut PlainMesh) {
    mesh.attributes.retain(|name, _| !is_uv_corner(name));
}

/// A part's levels after a level command.
#[derive(Serialize, JsonSchema)]
pub struct LevelState {
    /// The part.
    pub part: String,
    /// How many levels it has, the base included.
    pub count: u32,
    /// The active level, from 1 (the base) to `count` (the top).
    pub active: u32,
    /// Points of the active level.
    pub vertices: u64,
    /// Faces of the active level.
    pub faces: u64,
    /// Points of the top level that moved when edits made on a lower level were carried up.
    pub carried: u64,
}

impl LevelState {
    fn of(part: &str, p: &Part, carried: usize) -> Self {
        let (count, active) = p.levels().map_or((1, 0), |l| (l.count(), l.active()));
        Self {
            part: part.to_string(),
            count: count as u32,
            active: active as u32 + 1,
            vertices: p.store().vertex_count() as u64,
            faces: p.store().face_count() as u64,
            carried: carried as u64,
        }
    }
}

/// An example on a cube with one level added, the starting scene of most level examples.
fn subdivided_cube<T: Serialize>(summary: &'static str, args: T) -> Example<T> {
    Example::new(summary, args)
        .after("primitive.cube", serde_json::json!({}))
        .after("level.subdivide", serde_json::json!({}))
}

/// A level stack's refusal as a command error.
pub(crate) fn refused(e: &MeshError) -> CommandError {
    match e {
        MeshError::TooLarge(_) => CommandError {
            hint: Some("subdivide a smaller part, or delete higher levels first".into()),
            ..error(ErrorCode::Limit, e.to_string())
        },
        MeshError::Input(message) => error(ErrorCode::Precondition, message.clone()),
        MeshError::Config(_) => errors::internal(&e.to_string()),
    }
}

/// Refuses command `what` on a frozen part (decision 0030, section 2): its levels are set
/// aside, and only `level.unfreeze` brings them back.
fn thawed(name: &str, part: &Part, what: &str) -> Result<(), Unavailable> {
    let Some(record) = part.frozen() else {
        return Ok(());
    };
    let count = record.count();
    Err(Unavailable::new(format!(
        "part '{name}' has {count} frozen detail levels; {what} waits until they come back"
    ))
    .fix("run level.unfreeze first"))
}

/// A level command's `check`: `refusal` of the part it names, whatever it finds when it does not
/// refuse left aside. The engine pins the part before the check, so a part not found is left
/// for `run` to report.
fn check<'a, T>(
    cx: &'a Ctx,
    part: Option<&PartRef>,
    refusal: impl FnOnce(&str, &'a Part) -> Result<T, Unavailable>,
) -> Result<(), Unavailable> {
    let name = part.map_or("", |p| p.0.as_str());
    let Ok(found) = cx.doc.get_part(name) else {
        return Ok(());
    };
    refusal(name, found).map(drop)
}

/// The `check` of a level command that waits while the part is frozen.
pub(crate) fn check_thawed(
    cx: &Ctx,
    part: Option<&PartRef>,
    what: &str,
) -> Result<(), Unavailable> {
    check(cx, part, |name, found| thawed(name, found, what))
}

/// Runs `change` on a share of part `name`'s level stack (`None` when it has one level) and,
/// when it returns `Some`, puts the changed part in place as one undo step and reports it.
/// `None` leaves the part as it was, with `why` as the no-op reason. A frozen part was refused
/// by the command's `check` (`check_thawed`).
fn change(
    cx: &mut Ctx,
    name: &str,
    why: &str,
    change: impl FnOnce(&mut Levels) -> Result<Option<usize>, MeshError>,
) -> Result<LevelState, CommandError> {
    let part = cx.doc.get_part(name)?;
    let unchanged = LevelState::of(name, part, 0);
    let Some(levels) = part.levels() else {
        cx.noop_reason(format!("part '{name}' has one level: {why}"));
        return Ok(unchanged);
    };
    let (mut levels, props) = (levels.share(), part.props());
    let Some(carried) = change(&mut levels).map_err(|e| refused(&e))? else {
        cx.noop_reason(format!("part '{name}': {why}"));
        return Ok(unchanged);
    };
    let next = Part::leveled(levels, props);
    let state = LevelState::of(name, &next, carried);
    cx.doc.replace_part(name, next);
    Ok(state)
}

/// Runs `edit` on part `name`'s top, the path for an edit of the whole part that means the same
/// at every level: the mask and visibility commands (review 0004, M2), face group making and
/// merging, a paint fill, Blender's in-place write, and since DF8 the point edits that map every
/// point (a pure-map `transform.apply`, `mesh.translate`; decision 0041, section 6). A lower
/// level shows the top's protection thinner (a face is hidden only when every top descendant
/// is), and its difference carried up would miss what it does not show. So with a lower level
/// active the edit runs on the top, after pending edits carry up, and the active level is
/// gathered again: one undo step that swaps the part, like the other level commands. Otherwise
/// it runs on the active store in place. Read the result from the part afterwards.
pub(crate) fn whole_part(
    cx: &mut Ctx,
    name: &str,
    edit: impl FnOnce(&mut MeshStore) -> Vec<NodeId>,
) -> Result<(), CommandError> {
    let part = cx.doc.get_part(name)?;
    let Some(levels) = part.levels().filter(|l| l.active() < l.top_level()) else {
        cx.doc.edit_store(name, edit)?;
        return Ok(());
    };
    let (mut levels, props) = (levels.share(), part.props());
    let active = levels.active();
    levels.set(levels.top_level()).map_err(|e| refused(&e))?;
    if edit(levels.store_mut()).is_empty() {
        // Nothing to change: the part stays as it was, pending edits too.
        return Ok(());
    }
    levels.set(active).map_err(|e| refused(&e))?;
    cx.doc.replace_part(name, Part::leveled(levels, props));
    Ok(())
}
