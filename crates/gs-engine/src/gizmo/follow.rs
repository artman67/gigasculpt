//! A gizmo that follows its mask (`gizmo.unmasked-center`'s follow mask centre, proposed and
//! off by default; `gizmo.place` `follow`). After a call that changed the scene, with no
//! stroke or drag open, every pose that follows goes to the centre of its part's unmasked
//! visible points when that centre is not where it was when the pose was last set or moved.
//! So the gizmo moves by itself as the mask is painted, grown or undone, and a placement by
//! hand holds until the centre moves. It runs after every call, a replayed one too, so the
//! log moves it the same way.

use gs_core::Vec3;
use gs_core::gizmo::Centre;

use super::stable;
use crate::document::Part;
use crate::engine::Engine;

/// The centre of the bounding box of `part`'s unmasked visible points in the scene, or `None`
/// when every visible point is masked.
pub(super) fn unmasked(part: &Part) -> Option<Vec3> {
    part.store().centre(Centre::Unmasked, &part.transform())
}

impl Engine {
    /// Moves every gizmo that follows its mask to its part's new unmasked centre, once per
    /// change of the document and only when no stroke or drag is open.
    pub(crate) fn follow_masks(&mut self) {
        let state = &mut self.state;
        let gizmos = &mut state.gizmos;
        if gizmos.followed == self.rev || crate::ctx::unfinished(&state.sculpt, gizmos) {
            return;
        }
        gizmos.followed = self.rev;
        for (id, pose) in gizmos.poses.iter_mut().filter(|(_, p)| p.follow) {
            let found = state.doc.parts().find(|(_, p)| stable(p) == *id);
            let Some(centre) = found.and_then(|(_, p)| unmasked(p)) else {
                continue;
            };
            if gizmos.centres.get(id) != Some(&centre) {
                pose.origin = centre;
                gizmos.centres.insert(*id, centre);
            }
        }
    }
}
