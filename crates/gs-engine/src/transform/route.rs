//! Where a transform's result is kept (decision 0029, section 6, filling in decision 0027,
//! section 10). A move, a turn or an even scale of the whole part changes only the part's
//! transform, about 100 bytes of undo whatever the part's size; anything else (masked, soft,
//! partly hidden, mirrored, a turn or scale under a parallel plane, inflate, flatten, a stretch,
//! a deformer, a gradient or an axis left out) edits the points, in the part's own space. The
//! call and its log line are the same either way. On a part with detail levels shown below the
//! top, an edit of the points that is a pure map runs on the top ([`on_top`], [`edit_points`]).

use gs_core::transform::{Distance, Op, RIGID, Similarity, Transform};
use gs_core::{Cover, MeshStore, NodeId};
use gs_protocol::CommandError;

use crate::Ctx;
use crate::document::Part;

/// Where a transform goes.
pub(crate) enum Route {
    /// The part's new transform: the whole part moves, turns or scales evenly.
    Placed(Similarity),
    /// An edit of the points: the transform as it acts on the part's own points.
    Points(Transform),
}

/// Where `t`, a transform given in the scene, goes on `part` when the part's transform is
/// `from` (a drag passes the transform it began with). The part's transform takes it when the
/// operation is a move, a turn or a scale by three equal factors above zero, the falloff is
/// rigid, no mirror plane is on, no parallel plane is on but under a move (which every region
/// takes unchanged, decision 0044, section 5), no face is hidden, the mask (when it counts)
/// holds no point, and the result is a transform a part can keep.
pub(crate) fn route(part: &Part, from: Similarity, t: &Transform) -> Route {
    let store = part.store();
    let s = &t.symmetry;
    let whole = t.weights.falloff <= RIGID
        && !s.mirror.contains(&true)
        && (!s.parallel.contains(&true) || matches!(t.op, Op::Move(_)))
        && !store.any_hidden()
        && (!t.weights.mask
            || (0..store.node_count() as NodeId).all(|n| store.node_state(n).mask == Cover::None));
    match placed(from, t).filter(|_| whole) {
        Some(placed) => Route::Placed(placed),
        None => Route::Points(from.part_transform(t)),
    }
}

/// What an edit of the points reads for each point's change, beyond the numbers in its line
/// (decision 0041, amended 2026-10-13 for DF8-notes).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reads {
    /// Its own position, mask and visibility alone: a pure map of space.
    Point,
    /// Its normal (inflate).
    Normal,
    /// The shown level's edges (a falloff measured along the surface).
    Surface,
}

impl Reads {
    /// What `t` reads.
    pub(crate) fn of(t: &Transform) -> Self {
        if matches!(t.op, Op::Inflate(_)) {
            Self::Normal
        } else if t.weights.falloff > RIGID && t.weights.distance == Distance::Surface {
            Self::Surface
        } else {
            Self::Point
        }
    }
}

/// Whether an edit of `part`'s points that `reads` as said runs on the top level rather than on
/// the shown one and the next switch's carry (decision 0041, section 6, amended 2026-10-12 for
/// DF8 and 2026-10-13 for DF8-notes): the part shows a level below its top with no edit waiting
/// to be carried, and the edit is a pure map. Inflate reads normals and the surface falloff
/// floods the shown level's edges, so both keep the carry. This is the one place the rule
/// lives: false here gives back today's carry for every call.
pub(crate) fn on_top(part: &Part, reads: Reads) -> bool {
    reads == Reads::Point
        && (part.levels()).is_some_and(|l| l.active() < l.top_level() && !l.pending())
}

/// Runs `edit`, an edit of part `name`'s points that `reads` as said, where [`on_top`] says: on
/// the top through `level::whole_part`, one undo step that swaps the part (none when `edit`
/// writes nothing), or on the shown level in place.
pub(crate) fn edit_points(
    cx: &mut Ctx,
    name: &str,
    reads: Reads,
    edit: impl FnOnce(&mut MeshStore) -> Vec<NodeId>,
) -> Result<(), CommandError> {
    if on_top(cx.doc.get_part(name)?, reads) {
        crate::level::whole_part(cx, name, edit)
    } else {
        cx.doc.edit_store(name, edit).map(drop)
    }
}

/// The transform a part whose transform is `from` gets when `t` moves it whole: `None` unless
/// `t`'s operation is a move, a turn or a scale by three equal factors above zero, with every
/// axis kept and no gradient, and the result is a transform a part can keep. Weights are not
/// looked at: a copy placed by a Ctrl drag moves whole whatever they say (decision 0027,
/// section 14).
pub(crate) fn placed(from: Similarity, t: &Transform) -> Option<Similarity> {
    (t.op.similarity(t.frame.origin))
        .filter(|_| t.scope.is_plain())
        .map(|op| from.then(&op))
        .filter(Similarity::is_valid)
}
