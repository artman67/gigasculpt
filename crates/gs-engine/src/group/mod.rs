//! The face group commands (decision 0026): `group.assign`, `group.make` and `group.merge`
//! edit, `group.list` reads. Each edit is one undo step and runs through gs-core's `groups`
//! module, the one home of the id rule, the makers and the colors.

mod assign;
mod list;
mod make;
mod merge;

use gs_core::groups::{MAX_GROUP, NoFreshId, fresh_after};
use gs_protocol::{CommandError, ErrorCode};
use schemars::JsonSchema;
use serde::Serialize;

use crate::document::Part;
use crate::errors::{args_invalid, error};

/// The part's groups after an edit.
#[derive(Serialize, JsonSchema)]
pub struct Grouped {
    /// The part.
    pub part: String,
    /// The first and last id the call gave faces; it gave every id between them too. Left out
    /// when no face changed group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<[u32; 2]>,
    /// How many ids the part's faces hold now, 0 (no group) included when a face holds it.
    pub groups: u64,
    /// The largest id a face holds.
    pub largest: u32,
}

impl Grouped {
    fn of(name: &str, part: &Part, ids: Option<[u32; 2]>) -> Grouped {
        Grouped {
            part: name.to_string(),
            ids,
            groups: part.store().group_counts().len() as u64,
            largest: largest(part),
        }
    }
}

/// The largest id a face of `part` holds: on its active level, or on its top level, which may
/// hold an id no face of a lower level shows (decision 0026, 2).
fn largest(part: &Part) -> u32 {
    let top = part.levels().map_or(0, |l| l.top().largest_group());
    part.store().largest_group().max(top)
}

/// A fresh id for `part`: one above [`largest`], or `E_LIMIT` when no id is left.
fn fresh(part: &Part) -> Result<u32, CommandError> {
    let largest = largest(part);
    fresh_after(largest, 1).ok_or_else(|| no_fresh_id(NoFreshId { largest, wanted: 1 }))
}

/// The refusal when the groups asked for have no fresh ids left (review 0007, m1: the id
/// overflowed and the command panicked).
fn no_fresh_id(e: NoFreshId) -> CommandError {
    CommandError {
        hint: Some("group.merge can move the largest groups to lower ids".into()),
        ..error(ErrorCode::Limit, e.to_string())
    }
}

/// An id a caller gives at `path`, refused above [`MAX_GROUP`].
fn given(path: &str, id: u32) -> Result<u32, CommandError> {
    if id > MAX_GROUP {
        return Err(args_invalid(
            path,
            format!("a group id is at most {MAX_GROUP}"),
        ));
    }
    Ok(id)
}
