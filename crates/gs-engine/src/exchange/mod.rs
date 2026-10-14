//! The Blender exchange (decision 0032): `exchange.write` puts parts into a folder as raw column
//! files plus a JSON manifest, in Blender's axes and under Blender's names, and `exchange.read`
//! brings a folder the add-on wrote back into the scene as one undo step. The add-on in
//! `addons/blender/gigasculpt_bridge` is the other half; the socket carries the calls, the folder
//! carries the data.
//!
//! Where things live: `names` holds the one name table and the axis and crease conversions,
//! `manifest` the folder's description and its files, `write` and `read` the two commands,
//! `incoming` the decoding of what Blender sent (n-gons split as `file.import` splits them),
//! `color` the colors Blender sends made into the store's words, `place` the in-place write
//! that keeps a part's levels and every column Blender never saw, and `renumber` the match of a
//! Levels form top in Blender's numbering to the part's own.

mod color;
mod incoming;
mod manifest;
mod names;
mod place;
mod read;
mod renumber;
mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gs_core::Levels;
use gs_protocol::{CommandError, ErrorCode, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::document::Part;
use crate::errors::{self, error};

/// How a part with detail levels travels (decision 0032, section 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Form {
    /// The top level as a plain mesh with every column the exchange carries.
    #[default]
    Mesh,
    /// The top level's points and faces, with the base creases, for Blender to rebuild as a
    /// Multires modifier. A part with one level always goes in the Mesh form.
    Levels,
}

/// The `folder` argument as a path: absolute, and an existing folder.
fn folder(path: &str) -> Result<PathBuf, CommandError> {
    let dir = Path::new(path);
    if !dir.is_absolute() {
        return Err(CommandError {
            hint: Some("the add-on names the session's exchange folder in full".into()),
            ..errors::args_invalid("/folder", "give an absolute path")
        });
    }
    if !dir.exists() {
        return Err(error(
            ErrorCode::NotFound,
            format!("there is no folder at {path}"),
        ));
    }
    if !dir.is_dir() {
        return Err(error(
            ErrorCode::Precondition,
            format!("{path} is not a folder"),
        ));
    }
    Ok(dir.to_path_buf())
}

/// A part's top level as plain mesh data in its numbering, with edits pending on a lower level
/// carried up on a copy (the scene is untouched), and that copy of its levels if it has more
/// than one. A frozen part shows its base.
fn top(part: &Part) -> Result<(Arc<PlainMesh>, Option<Levels>), CommandError> {
    let Some(levels) = part.levels() else {
        return Ok((part.view(), None));
    };
    let (mesh, levels) = level_top(levels)?;
    Ok((Arc::new(mesh), Some(levels)))
}

/// A level stack's top as plain mesh data in its numbering, from a copy of the stack with the
/// top active (pending edits carried up), and that copy.
fn level_top(levels: &Levels) -> Result<(PlainMesh, Levels), CommandError> {
    let mut levels = levels.share();
    let t = levels.top_level();
    levels
        .set(t)
        .map_err(|e| error(ErrorCode::Precondition, e.to_string()))?;
    let mesh = levels.level_numbering(t).apply(levels.top().to_plain());
    Ok((mesh, levels))
}

/// Whether `a` and `b` have the same points and faces: as many points, and every face with the
/// same corners in the same order.
fn same_topology(a: &PlainMesh, b: &PlainMesh) -> bool {
    fn shape(m: &PlainMesh) -> (usize, &[[u32; 4]]) {
        (m.positions.len(), &m.faces)
    }
    shape(a) == shape(b)
}
