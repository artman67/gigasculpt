//! Live Boolean's two commands (decision 0057, section 7, as amended for step LB5).
//!
//! - **`boolean.apply {groups?, solver?, resolution?, name?}`**: Edit, Long, one undo step.
//!   Each group, a list of `[part, operation]` pairs whose first is the base (its operation is
//!   not used, and a warning says so when it is not `union`), becomes one new part at the end
//!   of the list, with `start` on and `union`; every operand is hidden in the same step, and the
//!   active part stays unless it was an operand, when the first result becomes active.
//!   - `groups` left out is every group of the list with a part that shows, each its parts that
//!     show with their marks (section 5), written into the logged call. Given, each group is cut
//!     as given, shown or not; an empty group is `E_ARGS_INVALID` at `/groups/i`.
//!   - `solver` is `auto` (the default, the ladder below), `manifold` or `voxel`, always logged.
//!   - `resolution` (16 or more; no upper bound in the schema, so a call past the voxel cell
//!     limit meets `E_LIMIT`) is written into the logged call as 128 unless `solver` is
//!     `manifold`, since the arguments are pinned before it is known whether a voxel step runs.
//!   - `name` (`boolean` by default) is a base: each result takes the first free name of it,
//!     `boolean`, `boolean-2`, ...; a name that reads as a reference is refused.
//!   - The result, `{groups: [{part, operands, solver, repairs, dropped, live}]}`: the new part
//!     as `part.create` reports it, its operands' names, the path that cut it (`manifold` or
//!     `voxel`), each repair as `{part, step, why}` (steps 3 to 5), the operands' columns the
//!     result does not have (`hide` whenever an operand has it; the UVs on the voxel fold), and
//!     whether it was the live mode's settled result, taken uncut (`auto` at 128 only).
//!   - Refused, the scene left as it was: no group with two parts (`E_PRECONDITION`); an operand
//!     with no faces, or a cut that leaves no faces (`E_PRECONDITION`, naming the part); with
//!     `manifold`, an operand the ladder would repair, or a group with no clean exact result
//!     (`E_PRECONDITION`, naming the part); a voxel step over its cell limit (`E_LIMIT`).
//!   - Warnings (`Success.warnings`, a job's answer too): a base whose operation is not
//!     `union`, each repair, an operand remeshed by step 5 whose UVs were dropped, and a group
//!     sent down the voxel fold, with why.
//! - **`boolean.check {parts?, coplanar_threshold?}`**: Query, Long. `parts` left out is every
//!   part that shows. Per part, in the scene: `closed`, and `open_edges`, `non_manifold_edges`,
//!   `inside_out_shells` and `self_crossings`, each `{count, faces}` (the faces' indices, at most
//!   1000). Between every two checked parts of one group, `coplanar`: `{parts, count, faces}`,
//!   the pairs of faces, one of each, within `coplanar_threshold` (0 to 0.01, by default
//!   0.00001) times the two parts' bounding box diagonal of each other's plane and overlapping
//!   there.
//!
//! The ladder (`ladder.rs`): steps 1 to 4 are gs-geo's `boolean::prepare`; step 5 remeshes an
//! operand alone (`voxel::repair_operand`); the cut is `boolean::fold_prepared`, and a group the
//! exact path cannot cut (a failed repair, a failed cut, a result that crosses itself) goes down
//! the voxel fold (`voxel::remesh_fold`). The carry (section 6) is `mesh`'s `carry_columns` and
//! `carry_uv_corners` on the operands laid end to end in the scene.
//!
//! The live mode (`live/`, section 8 as amended for step LB6): with `view.display`'s
//! `live_boolean` on, each group is cut again off the engine thread after every change to it,
//! and `session.state`'s `boolean.live` reports each group's result.

mod apply;
mod check;
mod grid;
mod groups;
mod ladder;
mod live;

pub(crate) use live::{Board, Live, LiveState};
pub use live::{LiveDraw, LiveMesh, LivePeel, PeelOperand, Stand, Tint};

use crate::document::Operation;

/// A part's operation, as gs-geo's.
pub(crate) fn geo(operation: Operation) -> gs_geo::operation::Operation {
    match operation {
        Operation::Union => gs_geo::operation::Operation::Union,
        Operation::Difference => gs_geo::operation::Operation::Difference,
        Operation::Intersect => gs_geo::operation::Operation::Intersect,
    }
}

/// The operation's word, as `part.set_operation` takes it.
fn word(operation: Operation) -> &'static str {
    match operation {
        Operation::Union => "union",
        Operation::Difference => "difference",
        Operation::Intersect => "intersect",
    }
}
