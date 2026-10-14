//! The Reference View's commands (decisions 0020, 0024): `reference.open`, `reference.link`,
//! `reference.align`, `reference.next`, `reference.close` and `reference.list`; `view.look`
//! (decision 0025) sets its look, as any view's. They set up a secondary view
//! (`super::secondary`); the `view.*` commands move what of its camera is its own. All but the
//! list are View commands: logged, never undoable, and the document never changes. Agents may
//! run them all (setting a reference up for the human is the point); moving a reference's
//! camera follows `main`'s rule (`Views::may_change`). A saved scene keeps the open references'
//! setups and brings them back (`saved`, decision 0024 section 8).

mod align;
mod close;
mod link;
mod next;
mod open;
pub(crate) mod saved;

use gs_core::camera::{Camera, Projection};
use gs_core::{Aabb, Vec3};
use gs_protocol::CommandError;
use schemars::JsonSchema;
use serde::Serialize;

use super::frame::{POINT_REACH, scene_box};
use super::secondary::{RefLook, ReferenceInfo};
use super::spec::{ExactCamera, ViewName};
use crate::document::Document;
use crate::errors;
use crate::{Ctx, Unavailable};

/// A Reference View after a command set it up.
#[derive(Serialize, JsonSchema)]
pub struct ReferenceShown {
    /// Its name, for example `ref:1`.
    pub view: String,
    /// Its camera now, exactly: its own, with what is linked taken from main.
    pub camera: ExactCamera,
    /// Its setup.
    pub reference: ReferenceInfo,
}

/// The reference a command names, after pinning.
fn name(view: Option<&ViewName>) -> Result<&str, CommandError> {
    view.map(|v| v.0.as_str())
        .ok_or_else(|| errors::internal("the view argument was not pinned"))
}

/// The check of a command on an open reference: pinning leaves `view` out when none is open.
fn one_open(view: Option<&ViewName>) -> Result<(), Unavailable> {
    match view {
        Some(_) => Ok(()),
        None => Err(Unavailable::new("no Reference View is open")
            .fix("open one with reference.open, for example scene=true")),
    }
}

/// Reference View `view` as it is now.
fn shown(cx: &Ctx, view: &str) -> Result<ReferenceShown, CommandError> {
    let doc = cx.doc.document();
    let info = cx.views.info(view, doc);
    let reference = info
        .reference
        .ok_or_else(|| errors::internal("the reference view is not open"))?;
    Ok(ReferenceShown {
        view: view.to_string(),
        camera: info.camera,
        reference,
    })
}

/// The look name meaning "the one the main view draws".
pub(in crate::view) const SAME_AS_MAIN: &str = "main";

/// `look` with its material (`#id`, `@place` or name) found in the palette and kept by name,
/// or refused naming the palette's; `main` means the main view's look (decision 0043).
pub(in crate::view) fn checked_look(cx: &Ctx, mut look: RefLook) -> Result<RefLook, CommandError> {
    if look.matcap.as_deref() == Some(SAME_AS_MAIN) {
        look.matcap = None;
    }
    let Some(wanted) = look.matcap.as_deref() else {
        return Ok(look);
    };
    let found = cx.palette.now.find(wanted).map_err(|e| CommandError {
        hint: Some("material.list lists every material; main means the main view's".into()),
        ..e
    })?;
    look.matcap = Some(found.name.clone());
    Ok(look)
}

/// The sphere around the parts of `doc` (decision 0024, section 5): centred on their box,
/// reaching their farthest point. One pass over the points; the corners of the chunks' boxes
/// would be quicker but reach so far past a round scene that it filled half its picture.
/// `None` for a scene with no parts.
pub(crate) fn sphere(doc: &Document) -> Option<(Vec3, f32)> {
    let b = scene_box(doc);
    if b == Aabb::EMPTY {
        return None;
    }
    let center = (b.lo + b.hi) * 0.5;
    let shown = doc.shown();
    // Each part measured in its own space from the centre mapped in, then scaled out
    // (decision 0029).
    let reach = shown.map(|(_, part)| {
        let (store, t) = (part.store(), part.transform());
        let c = t.part_point(center);
        let far = (0..store.node_count() as u32).map(|n| {
            // A dead slot is NaN, which `f32::max` passes over; a hidden point never counts.
            let node = store.node(n);
            let points = node
                .pos
                .iter()
                .enumerate()
                .filter(|(l, _)| !node.hidden(*l));
            points.map(|(_, &p)| (p - c).len2()).fold(0.0, f32::max)
        });
        t.length(far.fold(0.0, f32::max).sqrt())
    });
    Some((center, reach.fold(0.0, f32::max)))
}

/// The share of the half-picture a framed sphere may fill: the 5% border `Camera::fit` keeps.
const SPHERE_MARGIN: f32 = 0.9;

/// Places `camera` so the sphere `(center, radius)` fills its picture, `aspect` wide per high,
/// up to a 5% border, keeping its turn, lens and projection (a lens shift goes back to zero).
/// How far a sphere needs the camera does not depend on the turn, so the framing holds however
/// main turns a linked reference.
pub(crate) fn fit_sphere(camera: &mut Camera, (center, radius): (Vec3, f32), aspect: f32) {
    // A sphere of no size needs some distance too (review 0008, m3).
    let radius = if radius > 0.0 { radius } else { POINT_REACH };
    camera.lens.shift = [0.0; 2];
    let [tx, ty] = camera.lens.tan_half(aspect);
    let t = tx.min(ty) * SPHERE_MARGIN;
    camera.target = center.to_array();
    camera.distance = match camera.projection {
        // The sphere's outline is a cone of half-angle a around the view axis, sin a being
        // radius / distance: make tan a = t, so sin a = t / sqrt(1 + t^2).
        Projection::Perspective => radius * (1.0 + t * t).sqrt() / t,
        Projection::Orthographic => radius / t,
    };
}
