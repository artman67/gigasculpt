//! Level 0's UVs written into a part with levels (decision 0061 as amended 2026-10-13, step
//! UV5): the one way UVs enter a level stack below the top.

use gs_protocol::{CommandError, ErrorCode};

use super::{change, thawed};
use crate::Ctx;
use crate::errors::{self, error};

/// Writes `corners` as part `name`'s level-0 UVs: four `[u, v]` per base face, in the order the
/// part lists its faces at level 1 (a triangle's fourth corner unused). Every level above gets
/// what dividing would have given it: the UVs smoothed over each step whose level below keeps
/// its own (`level.describe`'s `own_uvs`) and linear over the others. The active level shows
/// them at once and edits pending on it stay pending. One undo step that swaps the part.
///
/// Refuses a frozen part (run `level.unfreeze` first), a part with one level (its UV columns
/// take the corners in place) and a count other than the base's faces.
pub fn write_level_uvs(
    cx: &mut Ctx,
    name: &str,
    corners: Vec<[[f32; 2]; 4]>,
) -> Result<(), CommandError> {
    let part = cx.doc.get_part(name)?;
    thawed(name, part, "writing its UVs").map_err(errors::unavailable)?;
    if part.levels().is_none() {
        let message = format!("part '{name}' has one level: write its UV columns in place");
        return Err(error(ErrorCode::Precondition, message));
    }
    change(cx, name, "", |l| l.write_uvs(corners).map(|()| Some(0))).map(drop)
}
