//! The View commands (decision 0017): `view.get`, `view.set`, the moves (`view.orbit`,
//! `view.roll`, `view.pan`, `view.dolly`), `view.frame`, `view.align`, `view.level`,
//! `view.lens`, `view.lock`, `view.settings`, `view.undo` and `view.redo`, and `view.look` (how
//! any view is drawn). Every camera change goes through [`change`], so locking, the agent policy, the camera
//! history and gestures work the same for all of them.

mod frame;
mod get;
mod history;
mod look;
mod moves;
pub(crate) mod pin;
mod set;
mod setup;

use gs_core::camera::Camera;
use gs_protocol::{CommandError, Vec3};
use schemars::JsonSchema;
use serde::Serialize;

use super::secondary::Touches;
use super::spec::{ExactCamera, ViewName};
use super::views::{Part, Views};
use crate::document::Document;
use crate::{Ctx, Unavailable, errors};

/// A view after a command changed it.
#[derive(Serialize, JsonSchema)]
pub struct Moved {
    /// Its name.
    pub view: String,
    /// Its camera now, exactly.
    pub camera: ExactCamera,
}

/// The view a command names, after pinning (the caller's own when it was left out).
fn name(view: Option<&ViewName>) -> Result<&str, CommandError> {
    view.map(|v| v.0.as_str())
        .ok_or_else(|| errors::internal("the view argument was not pinned"))
}

/// The image size a command's view is seen at, after pinning, or why it is none.
pub(super) fn size(viewport: Option<[u32; 2]>) -> Result<[u32; 2], CommandError> {
    let [w, h] = viewport.ok_or_else(|| errors::internal("the viewport was not pinned"))?;
    if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
        let message = "the viewport must be 1 to 16384 pixels each way";
        return Err(errors::args_invalid("/viewport", message));
    }
    Ok([w, h])
}

/// Why the caller cannot change `view` now.
fn may_change(cx: &Ctx, view: Option<&ViewName>) -> Result<(), Unavailable> {
    match view {
        Some(v) => Views::may_change(cx, &v.0),
        None => Ok(()),
    }
}

/// The arguments every camera change shares, pinned.
struct Change<'a> {
    view: Option<&'a ViewName>,
    viewport: Option<[u32; 2]>,
    pivot: Option<Vec3>,
    gesture: Option<u64>,
    end: bool,
    /// False for a change that is no step of the camera history.
    remember: bool,
    /// What it moves, which a Reference View's link may hold (decision 0020).
    moves: Touches,
}

/// Changes the command's view with `f`, which gets the camera, the pivot, the views and the
/// scene the view shows: one camera history step, or one call of a gesture (not logged by
/// itself; the engine logs the gesture once it ends, as the camera it ended on).
fn change(
    cx: &mut Ctx,
    c: &Change,
    f: impl FnOnce(&mut Camera, gs_core::Vec3, &mut Views, &Document),
) -> Result<Moved, CommandError> {
    let view = name(c.view)?;
    let viewport = size(c.viewport)?;
    let part = Part {
        gesture: c.gesture,
        end: c.end,
        remember: c.remember,
    };
    let pivot = c
        .pivot
        .map_or(gs_core::Vec3::ZERO, |p| gs_core::Vec3::from_array(p.0));
    let doc = cx.doc.document();
    let shows = cx.views.secondary(view).map(|s| s.scene.clone());
    let scene = shows.as_ref().map_or(doc, |s| s.doc(doc));
    let moving = (cx.views).moving((view, c.moves), cx.origin, (viewport, doc), part, pivot)?;
    let mut camera = moving.camera;
    f(&mut camera, moving.pivot, cx.views, scene);
    if !camera_is_finite(&camera) {
        let message = "the move would leave the camera without a finite position";
        return Err(errors::args_invalid("", message));
    }
    cx.views.moved(view, camera, part);
    cx.in_gesture = part.gesture.is_some();
    Ok(Moved {
        view: view.to_string(),
        camera: camera.into(),
    })
}

/// Every number of the camera is finite and its distance positive.
fn camera_is_finite(c: &Camera) -> bool {
    let q = c.orientation.to_array();
    let rest = [c.distance, c.lens.focal_mm];
    let mut numbers = q.iter().chain(&c.target).chain(&rest);
    numbers.all(|v| v.is_finite()) && c.distance > 0.0
}
