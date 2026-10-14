//! What the pen's press does to the gizmo, sample by sample: on a handle it holds it and its
//! motion becomes `gizmo.drag` updates (`drag`), on the model it puts the gizmo there and a drag
//! aims it, and the lift or Esc ends either. The open deformer's handles come first
//! (`deforming`).

use gs_core::gizmo::{Pose, along, handles, pick};
use gs_core::{Quat, Vec3};
use gs_engine::PenReads;
use gs_protocol::{PenSample, Step};
use serde_json::json;

use super::drag::{Grab, place};
use super::{PenPress, Rectangle, Seen, Took, Tool};
use crate::pieces::ListedPart;
use crate::record::with_args;

/// A press on the model, until the pen lifts (decision 0027, section 14): the press put the
/// gizmo on the surface along its normal; once the pen moves farther than a pick radius, the
/// gizmo's Z aims from there toward the pen, on the plane through that point facing the screen.
/// The lift sends that aim as one `gizmo.place`. With Alt held, a pen beyond the radius off the
/// model squares the gizmo to the screen (X right, Y up, Z at the viewer), so its axes never
/// roll or flip, and with Shift too to the scene axes; over the model it aims as without Alt.
pub(super) struct Aim {
    pub(super) part: String,
    /// The pose the press put on the surface.
    placed: Pose,
    /// Where the press began, in the view's pixels.
    start: [f32; 2],
    /// The point Z aims at, while the pen is beyond the pick radius.
    toward: Option<Vec3>,
    /// The axes an Alt drag squares the gizmo to, while it does.
    square: Option<Quat>,
}

impl Aim {
    /// The pose as drawn: squared, aimed, or as the press put it.
    pub(super) fn pose(&self) -> Pose {
        let aimed = self.toward.and_then(|t| along(t - self.placed.origin));
        self.square.or(aimed).map_or(self.placed, |axes| Pose {
            axes,
            ..self.placed
        })
    }

    /// The pen at `at` (the view's pixels); `reads` says whether the model is under it.
    fn follow(&mut self, at: [f32; 2], seen: &Seen, reads: &PenReads) {
        let radius = seen.style.pick_pt * seen.style.px_per_pt;
        let far = libm::hypotf(at[0] - self.start[0], at[1] - self.start[1]) > radius;
        let (e, d) = seen.camera.ray(at[0], at[1], seen.size);
        let forward = seen.camera.axes().2;
        let facing = forward.dot(d);
        self.toward = (far && facing.abs() > 1e-6)
            .then(|| e + d * (forward.dot(self.placed.origin - e) / facing));
        self.square = match (far && seen.alt, seen.shift) {
            (false, _) => None,
            (true, true) => Some(Quat::IDENTITY),
            (true, false) => (reads.under)(e, d)
                .is_none()
                .then_some(seen.camera.orientation),
        };
    }
}

impl Tool {
    /// One pen sample at `s`, `over_view` when the 3D view is what is under it.
    pub fn sample(
        &mut self,
        s: &PenSample,
        over_view: bool,
        seen: &Seen,
        reads: &PenReads,
        parts: &[ListedPart],
    ) -> Took {
        let touching = s.in_proximity && s.contact && s.buttons & 1 == 0;
        let barrel = s.in_proximity && s.buttons & 1 != 0;
        let at = [s.x as f32 - seen.corner[0], s.y as f32 - seen.corner[1]];
        let mut took = Took {
            gizmo: true,
            ..Took::default()
        };
        match std::mem::take(&mut self.press) {
            PenPress::Up if touching && over_view && self.on => {
                self.show(parts, reads.pose);
                return self.press_at(at, seen, reads);
            }
            PenPress::Up if touching && self.on => {
                self.press = PenPress::Away;
                took.gizmo = false;
            }
            PenPress::Up => took.gizmo = false,
            PenPress::Away => {
                if s.in_proximity && s.contact {
                    self.press = PenPress::Away;
                }
                took.gizmo = false;
            }
            // The barrel button cancels, as Esc does.
            PenPress::Grab(g) if barrel => {
                took.steps.extend(cancel(&g));
                self.press = PenPress::Done;
            }
            PenPress::Grab(mut g) if touching => {
                let snap = seen.shift.then_some(seen.prefs.snap);
                if let Some(op) = g.follow(at, snap)
                    && g.transforms
                {
                    took.steps.push(with_args("gizmo.drag", g.update(op, snap)));
                    g.sent = true;
                }
                self.press = PenPress::Grab(g);
            }
            PenPress::Grab(g) => took.steps = self.release(*g),
            // The barrel button keeps the gizmo on the surface, as the press put it.
            PenPress::Aim(_) if barrel => self.press = PenPress::Done,
            PenPress::Aim(mut a) if touching => {
                a.follow(at, seen, reads);
                self.press = PenPress::Aim(a);
            }
            PenPress::Aim(a) => {
                let origin = a.placed.origin.to_array();
                let args = match (a.square, a.toward) {
                    (Some(axes), _) => Some(json!({ "origin": origin, "axes": axes.to_array() })),
                    (None, Some(t)) => Some(json!({ "origin": origin, "toward": t.to_array() })),
                    (None, None) => None,
                };
                if let Some(mut args) = args {
                    args["part"] = json!(a.part);
                    took.steps.push(with_args("gizmo.place", args));
                    self.shown = Some((a.part.clone(), a.pose()));
                }
            }
            PenPress::Hold(h) => took.steps = self.hold(*h, at, (touching, barrel), seen),
            PenPress::Rect(r) => self.rectangle(*r, at, (touching, barrel), seen),
            PenPress::Done if s.in_proximity && s.contact => self.press = PenPress::Done,
            // The lift that ends it.
            PenPress::Done => {}
        }
        took
    }

    /// A press at `at`, the view's pixels: on the open deformer's handles it holds one
    /// (`deforming`); on the gizmo's it holds it (with Ctrl, to move a copy, on any handle but
    /// an axis box); on the model it puts the gizmo there, along the surface, for a drag to aim,
    /// or draws the pen rectangle while a cage or a curve is open; on empty canvas it navigates.
    fn press_at(&mut self, at: [f32; 2], seen: &Seen, reads: &PenReads) -> Took {
        if let Some(took) = self.press_deformer(at, seen) {
            return took;
        }
        let took = Took {
            gizmo: true,
            ..Took::default()
        };
        if let Some((part, pose)) = self.shown.clone() {
            let drawn = handles(&pose, seen.camera, seen.size, &seen.style);
            let radius = seen.style.pick_pt * seen.style.px_per_pt;
            if let Some(id) = pick(&drawn.handles, at, radius) {
                let transforms = pose.locked && !seen.alt;
                let copy = transforms && seen.ctrl && id.kind() != "box";
                // A copy moves whole, with no mirror, so radial symmetry does not stop it.
                if seen.brush.symmetry.radial.is_some() && transforms && !copy {
                    // As `transform.apply` refuses it: never a drag that silently does not copy.
                    self.press = PenPress::Done;
                    let why = "radial symmetry is on; transforms do not support it yet";
                    return Took {
                        problem: Some(why.into()),
                        ..took
                    };
                }
                let view = (seen.camera, seen.size, &drawn);
                let how = (seen.brush, seen.prefs);
                let grab = Grab::new((id, part, pose), (transforms, copy), view, at, how);
                self.press = PenPress::Grab(Box::new(grab));
                return took;
            }
        }
        let (o, d) = seen.camera.ray(at[0], at[1], seen.size);
        let Some(hit) = (reads.under)(o, d) else {
            self.press = PenPress::Away;
            return Took {
                canvas: true,
                ..Took::default()
            };
        };
        if self.rectangles() {
            let r = Rectangle { start: at, end: at };
            self.press = PenPress::Rect(Box::new(r));
            return took;
        }
        // Alt snaps the origin to the corner of the face hit nearest the press.
        let point = if seen.alt { hit.corner } else { hit.point };
        let args = json!({
            "part": hit.part,
            "origin": point.to_array(),
            "normal": hit.normal.to_array(),
        });
        // Shown at once as `gizmo.place` will put it, by the same rule.
        let before = (reads.pose)(&hit.part).unwrap_or_default();
        let on_surface = placed(before, point, hit.normal);
        self.shown = Some((hit.part.clone(), on_surface));
        self.press = PenPress::Aim(Box::new(Aim {
            part: hit.part,
            placed: on_surface,
            start: at,
            toward: None,
            square: None,
        }));
        Took {
            steps: vec![with_args("gizmo.place", args)],
            ..took
        }
    }

    /// The pen lifted off held handle `g`: a locked drag ends, and the gizmo follows what moved
    /// unless it is sticky; an unlocked one (or with Alt) puts the gizmo where it was dragged.
    /// A Ctrl drag's end adds the copy, the active part, whose gizmo goes where the drag left
    /// it; the part's own stays.
    fn release(&mut self, g: Grab) -> Vec<Step> {
        let mut steps = Vec::new();
        if g.transforms && g.sent {
            steps.push(with_args("gizmo.drag", json!({ "phase": "end" })));
        }
        let moved = g.drawn();
        if g.copy && g.sent {
            let args = json!({
                "origin": moved.origin.to_array(),
                "axes": moved.axes.to_array(),
                "locked": moved.locked,
                "sticky": moved.sticky,
                "follow": moved.follow,
            });
            steps.push(with_args("gizmo.place", args));
            // Drawn where the drag left it until the session hands over the copy's.
            self.shown = Some((g.part, moved));
            self.to_active = true;
        } else if moved != g.pose {
            steps.push(with_args("gizmo.place", place(&g.part, moved)));
            // Shown at once where it was put, so it does not jump back for a frame.
            self.shown = Some((g.part, moved));
        }
        steps
    }

    /// Esc: lets go of a held handle, putting what it dragged back; a press on the model keeps
    /// the gizmo on the surface, unaimed; the pen rectangle selects nothing.
    pub fn escape(&mut self) -> Option<Step> {
        match std::mem::take(&mut self.press) {
            PenPress::Grab(g) => {
                self.press = PenPress::Done;
                cancel(&g)
            }
            PenPress::Hold(h) => {
                self.press = PenPress::Done;
                h.cancel()
            }
            PenPress::Aim(_) | PenPress::Rect(_) => {
                self.press = PenPress::Done;
                None
            }
            other => {
                self.press = other;
                None
            }
        }
    }
}

/// What letting go of `g` without keeping it sends: a cancel, if a locked drag went out.
fn cancel(g: &Grab) -> Option<Step> {
    (g.transforms && g.sent).then(|| with_args("gizmo.drag", json!({ "phase": "cancel" })))
}

/// `before` put at `origin` with its Z along `normal`, as `gizmo.place` does.
fn placed(before: Pose, origin: Vec3, normal: Vec3) -> Pose {
    Pose {
        origin,
        axes: along(normal).unwrap_or(before.axes),
        ..before
    }
}
