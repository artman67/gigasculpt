//! The gizmo's engine half (decision 0027): each part's pose (`gizmo.place`, `gizmo.reset`),
//! its handles as the window will draw them (`gizmo.describe`), drags (`gizmo.drag`), which
//! keep one undo step open across calls and commit as one `transform.apply` line, the open
//! deformer (`gizmo.deformer`, decision 0041), kept open across handle drags until Accept, and
//! the locked Mesh to Axis and Reset Orientation (`gizmo.mesh_to_axis`,
//! `gizmo.reset_orientation`, decision 0029), which move the part with its gizmo.
//!
//! The pose is session state, kept here by the part's stable id: not in undo, not in the scene
//! file, and put back with the views when a call fails. A pose that follows its mask goes to
//! the centre of the unmasked points after each change (`follow`). gs-core's `gizmo` holds the
//! pose's rules and the handle function the window draws with; the transforms themselves are
//! `transform`'s.

mod axis;
mod deformer;
mod describe;
mod drag;
mod follow;
mod live;
mod place;
mod preview;

use std::collections::BTreeMap;

use gs_core::gizmo::{Centre, Pose};
use gs_protocol::{CommandError, Vec3};
use schemars::JsonSchema;
use serde::Serialize;

use crate::Ctx;
use crate::document::Part;
pub(crate) use deformer::Deformer;
pub use deformer::OpenDeformer;
pub(crate) use live::{Drag, Event};
pub use preview::DeformPreview;

/// The session's gizmos: a pose for every part that was placed, the drag in progress, the open
/// deformer, and the drag and deformer events the current call queued.
#[derive(Clone, Default)]
pub(crate) struct Gizmos {
    /// Placed poses, by the part's stable id.
    poses: BTreeMap<u64, Pose>,
    /// Where the unmasked centre was when each pose that follows its mask was last set or
    /// followed it, by the part's stable id: the pose moves again only when the centre moves
    /// from here.
    centres: BTreeMap<u64, gs_core::Vec3>,
    /// The document revision the poses last followed their masks at.
    followed: u64,
    /// What `gizmo.drag` and `gizmo.deformer` queued in this call; the engine applies it after
    /// the call.
    pub queued: Vec<Event>,
    /// The drag in progress: its part's undo step is open.
    pub drag: Option<Drag>,
    /// An edit or a history call cut the held drag: its later updates change nothing until the
    /// window sends its end or cancel.
    pub cut: bool,
    /// The open deformer: its part's undo step is open while it has moved points.
    pub deformer: Option<Deformer>,
}

/// The poses and the centres they follow, as [`Gizmos::saved`] keeps them for a call.
pub(crate) struct Saved {
    poses: BTreeMap<u64, Pose>,
    centres: BTreeMap<u64, gs_core::Vec3>,
}

impl Gizmos {
    /// The poses alone, for a job's copy of the session: a job reads them (`frame: "gizmo"` in
    /// a batch) and never drags.
    pub fn poses_only(&self) -> Self {
        Self {
            poses: self.poses.clone(),
            ..Self::default()
        }
    }

    /// True while a drag is in progress or a deformer is open: the document holds a change not
    /// yet kept.
    pub fn unsettled(&self) -> bool {
        self.drag.is_some() || self.deformer.is_some()
    }

    /// The poses, to put back with [`Gizmos::put_back`]: they are part of a call's
    /// transaction, as the views are, so a call that fails or a dry run leaves them as they
    /// were.
    pub fn saved(&self) -> Saved {
        Saved {
            poses: self.poses.clone(),
            centres: self.centres.clone(),
        }
    }

    /// Puts back the poses [`Gizmos::saved`] took and drops the drag events the call queued.
    pub fn put_back(&mut self, saved: Saved) {
        (self.poses, self.centres) = (saved.poses, saved.centres);
        self.queued.clear();
    }
}

/// Part `part`'s pose, and whether it was placed. A part never placed has the default pose:
/// at the centre of the bounding box of its visible points in the scene (the scene origin when
/// none is visible), with the scene axes, locked and not sticky.
pub(crate) fn pose_of(cx: &Ctx, part: &str) -> Result<(Pose, bool), CommandError> {
    let found = cx.doc.get_part(part)?;
    if let Some(pose) = cx.gizmos.poses.get(&stable(found)) {
        return Ok((*pose, true));
    }
    let origin = found.store().centre(Centre::Visible, &found.transform());
    let pose = Pose {
        origin: origin.unwrap_or(gs_core::Vec3::ZERO),
        ..Pose::default()
    };
    Ok((pose, false))
}

/// Sets part `part`'s pose, and drops the poses of parts that no longer exist. A pose that
/// follows its mask notes where the unmasked centre is now, so it holds where it was set until
/// that centre moves.
pub(crate) fn set_pose(cx: &mut Ctx, part: &str, pose: Pose) -> Result<(), CommandError> {
    let found = cx.doc.get_part(part)?;
    let id = stable(found);
    let centre = pose.follow.then(|| follow::unmasked(found)).flatten();
    let live: Vec<u64> = (cx.doc.document().parts())
        .map(|(_, p)| stable(p))
        .collect();
    let gizmos = &mut *cx.gizmos;
    gizmos.poses.retain(|id, _| live.contains(id));
    gizmos.centres.retain(|id, _| live.contains(id));
    gizmos.poses.insert(id, pose);
    match centre {
        Some(c) => gizmos.centres.insert(id, c),
        None => gizmos.centres.remove(&id),
    };
    Ok(())
}

/// The id a pose is kept by: the part's stable id (decision 0029, section 10), which survives
/// every swap of the part, as its upload identity does not.
fn stable(part: &Part) -> u64 {
    part.props().id.unwrap_or_default()
}

/// A pose, as commands report it.
#[derive(Serialize, JsonSchema)]
pub struct PoseInfo {
    /// The gizmo's centre, in scene units.
    pub origin: Vec3,
    /// Its axes as a unit quaternion `[x, y, z, w]`; its rotation's columns are the gizmo's X,
    /// Y and Z.
    pub axes: [f32; 4],
    /// Drags transform the part (true) or only move the gizmo.
    pub locked: bool,
    /// The gizmo stays put after a transform (true) or follows what moved.
    pub sticky: bool,
    /// The gizmo goes to the centre of the unmasked visible points whenever it moves (true).
    pub follow: bool,
}

impl From<Pose> for PoseInfo {
    fn from(p: Pose) -> Self {
        Self {
            origin: Vec3(p.origin.to_array()),
            axes: p.axes.to_array(),
            locked: p.locked,
            sticky: p.sticky,
            follow: p.follow,
        }
    }
}
