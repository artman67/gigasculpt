//! The gizmo (decision 0027): a part's pose, how a move and a turn snap, how big it is drawn,
//! and where its origin is centred. Its handles on screen are `handles`, an open deformer's are
//! `deformer`, and how a held handle meets the pen is `aim`.

mod aim;
mod deformer;
mod handles;

pub use aim::{along_ray, on_plane, sweep, toward_viewer};
pub use deformer::{
    Deformer, DeformerHandle, DeformerHandles, Drive, Kind, Line, Moved, Unit, deformer_handles,
    pick_deformer, seen,
};
pub use handles::{Handle, HandleId, Handles, Shape, distance, handles, pick};

use crate::columns::{AttrDef, AttrVec};
use crate::config::NodeId;
use crate::math::{Quat, Vec3};
use crate::store::MeshStore;
use crate::transform::Similarity;

/// Where a part's gizmo sits and how it acts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub origin: Vec3,
    /// A unit quaternion; its columns are the gizmo's X, Y and Z.
    pub axes: Quat,
    /// Drags transform the part (true) or only move the pose.
    pub locked: bool,
    /// The pose stays put after a transform (true) or follows what moved.
    pub sticky: bool,
    /// The origin goes to the centre of the unmasked visible points ([`Centre::Unmasked`])
    /// whenever that centre moves (true), as gs-engine's `gizmo` follows it after each change.
    pub follow: bool,
}

impl Default for Pose {
    /// At the scene origin with the scene axes, locked, not sticky, not following the mask.
    fn default() -> Pose {
        Pose {
            origin: Vec3::ZERO,
            axes: Quat::IDENTITY,
            locked: true,
            sticky: false,
            follow: false,
        }
    }
}

/// The axes with Z along `z`: X is the scene axis least aligned with Z (the lowest on a tie)
/// with its Z part taken out, Y is Z cross X. `None` for a zero or not finite `z`.
pub fn along(z: Vec3) -> Option<Quat> {
    let n = z.len2().sqrt();
    if !(n > 0.0 && n.is_finite()) {
        return None;
    }
    let z = z * (1.0 / n);
    let c = z.to_array().map(f32::abs);
    let i = (0..3).fold(0, |best, a| if c[a] < c[best] { a } else { best });
    let mut e = [0.0; 3];
    e[i] = 1.0;
    let e = Vec3::from_array(e);
    let x = (e - z * e.dot(z)).normalized();
    Some(Quat::from_axes(x, z.cross(x), z))
}

/// How far apart a snapped move's stops are, in scene units: with Shift, a move snaps along
/// each of the gizmo's axes to a whole number of these (`gizmo.gizmo-move`'s proposal, decision
/// 0027, section 8).
pub const MOVE_SNAP: f32 = 0.01;

/// `v` with its part along each of `axes` (a unit quaternion's columns) rounded to a whole
/// number of `spacing`s. Snapping again gives the same bits.
pub fn snap_move(v: Vec3, axes: Quat, spacing: f32) -> Vec3 {
    let (x, y, z) = axes.axes();
    let on = |a: Vec3| a * ((v.dot(a) / spacing).round() * spacing);
    on(x) + on(y) + on(z)
}

/// `degrees` rounded to a whole number of `steps` of a full turn.
pub fn snap_turn(degrees: f32, steps: u32) -> f32 {
    let step = 360.0 / steps.max(1) as f32;
    (degrees / step).round() * step
}

/// How the gizmo is sized on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// The gizmo's size in points, 10 to 200.
    pub size_pt: f32,
    /// Pixels per point on this screen.
    pub px_per_pt: f32,
    /// How far from a handle a press still grabs it, in points.
    pub pick_pt: f32,
}

impl Default for Style {
    /// 75 points (Blender's default), one pixel per point, a 10-point pen radius.
    fn default() -> Style {
        Style {
            size_pt: 75.0,
            px_per_pt: 1.0,
            pick_pt: 10.0,
        }
    }
}

/// Which points a pose's origin is centred on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Centre {
    /// Every visible point.
    Visible,
    /// Every visible point whose mask is below 1.
    Unmasked,
}

impl MeshStore {
    /// The centre of the bounding box of the points `which` names, each placed in the scene by
    /// `placed` (the part's transform, decision 0029), or `None` when there are none. A mask
    /// value that is not finite counts as fully masked, as brushes treat it.
    pub fn centre(&self, which: Centre, placed: &Similarity) -> Option<Vec3> {
        let mask = (which == Centre::Unmasked)
            .then(|| self.attr_id(&AttrDef::mask().name))
            .flatten();
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for n in 0..self.node_count() as NodeId {
            let node = self.node(n);
            let m = match mask.and_then(|a| node.attr(a)) {
                Some(AttrVec::F32(m)) => Some(m.as_slice()),
                _ => None,
            };
            for (l, p) in node.pos.iter().enumerate() {
                let held = m.is_some_and(|m| m[l] >= 1.0 || !m[l].is_finite());
                if node.hidden(l) || p.is_dead() || held {
                    continue;
                }
                for (a, c) in placed.point(*p).to_array().into_iter().enumerate() {
                    lo[a] = lo[a].min(c);
                    hi[a] = hi[a].max(c);
                }
            }
        }
        (lo[0] <= hi[0])
            .then(|| Vec3::from_array(std::array::from_fn(|a| f32::midpoint(lo[a], hi[a]))))
    }
}
