//! The drag in progress (decision 0027, section 7): a drag keeps its part's undo step open
//! across many calls, so it runs beside the call transaction, as the pen's stroke does
//! (`sculpt::live`). `gizmo.drag` queues its events; after the call the engine goes back to the
//! drag's start (`revert_step`, or the transform the part had) and applies each update, so every
//! update starts from the same part. An update keeps its result where `transform.apply` would
//! (decision 0029, section 6): a whole-part move, turn or even scale changes the part's
//! transform in place, anything else its points. The end commits as one `transform.apply` line
//! and one undo step that replay and undo exactly; a cancel leaves every bit as it was.
//!
//! A Ctrl drag's live copy (decision 0027, section 14) is a share of the part under the copy's
//! name, holding the id the copy will get. The end takes it away and runs the `part.duplicate`
//! call that adds it for good, so the copy is one log line and one undo step; a cancel takes it
//! away and logs nothing.

use gs_core::NodeId;
use gs_core::transform::Transform;
use gs_protocol::{Outcome, Step};
use serde_json::json;

use crate::Origin;
use crate::document::Props;
use crate::engine::{Adopted, Call, Engine};
use crate::transform::{self, NO_CHANGE, Route, TransformApply};

/// A drag or deformer event, for the engine to apply after the call that queued it.
#[derive(Clone)]
pub(crate) enum Event {
    /// Transform the part from where it was when the drag began.
    Update(Box<Update>),
    End,
    Cancel,
    /// A `gizmo.deformer` call.
    Deformer(super::deformer::Change),
}

/// An update, checked and ready to apply.
#[derive(Clone)]
pub(crate) struct Update {
    pub(super) origin: Origin,
    pub(super) args: TransformApply,
    /// The transform in the scene.
    pub(super) t: Transform,
    /// The part's bounding-box diagonal in the scene when the drag began.
    pub(super) diagonal: f32,
    /// A copy's update: the copy's name.
    pub(super) copy: Option<String>,
}

/// A drag in progress. Each update starts again from where the part was when the drag began,
/// and keeps its result where `transform.apply` would (decision 0029, section 6): in the
/// part's transform, changed in place, or in its points, inside the part's open undo step.
#[derive(Clone)]
pub(crate) struct Drag {
    origin: Origin,
    /// The part it moves: for a copy, the live copy.
    pub(super) part: String,
    /// The transform applied now, as the call that replays it.
    pub(super) args: TransformApply,
    pub(super) diagonal: f32,
    /// The part's properties when the drag began.
    start: Props,
    /// The last update edited the points: the part's step is open.
    points: bool,
    /// Every node any update wrote since the step opened, ascending.
    written: Vec<NodeId>,
    /// A Ctrl drag's copy: what it copies.
    pub(super) copy: Option<Copied>,
}

/// What a Ctrl drag copies.
#[derive(Clone)]
pub(super) struct Copied {
    /// The part copied.
    pub(super) source: String,
    /// The part active before the drag, active again when the live copy goes.
    active: Option<u64>,
}

impl Engine {
    /// Applies the drag and deformer events the last call queued, then accepts an open deformer
    /// if the call picked another tool.
    pub(crate) fn gizmo_drags(&mut self) {
        for event in std::mem::take(&mut self.state.gizmos.queued) {
            match event {
                Event::Update(update) => self.drag_to(*update),
                Event::End => self.end_drag(),
                Event::Cancel => self.cancel_drag(),
                Event::Deformer(change) => self.deformer_change(change),
            }
        }
        self.accept_on_another_tool();
    }

    /// Ends the gizmo's gestures before a call that edits or moves through history, or a pen
    /// stroke: commits a held drag and cuts it, so its later updates change nothing until the
    /// window lets go (decision 0041, amended 2026-10-12 for DF4, section 1), and accepts an
    /// open deformer.
    pub(crate) fn end_gizmo(&mut self) {
        self.state.gizmos.cut |= self.state.gizmos.drag.is_some();
        self.end_drag();
        self.accept_deformer();
    }

    /// Transforms the drag's part to `update`, from where it was when the drag began, keeping
    /// the result where `transform.apply` would. The first update ends a pen stroke and accepts
    /// an open deformer; an update for another part commits the drag on the first. An update
    /// that keeps its result elsewhere than the last one first puts both the transform and the
    /// points back.
    fn drag_to(&mut self, update: Update) {
        let Update {
            origin,
            mut args,
            t,
            diagonal,
            copy,
        } = update;
        let source = args.part.clone().map(|p| p.0).unwrap_or_default();
        let part = copy.clone().unwrap_or_else(|| source.clone());
        let open = (self.state.gizmos.drag.as_ref())
            .map(|d| d.part == part && d.copy.is_some() == copy.is_some());
        if open == Some(false) {
            self.end_drag();
        }
        let mut copied = None;
        if open != Some(true) {
            self.end_live();
            self.accept_deformer();
            if copy.is_some() {
                let Some(shown) = self.show_copy(source, &part) else {
                    return;
                };
                copied = Some(shown);
            }
        }
        let Some(found) = self.state.doc.get_mut(&part) else {
            return;
        };
        let mut drag = self.state.gizmos.drag.take().unwrap_or_else(|| Drag {
            origin,
            part,
            args: args.clone(),
            diagonal,
            start: found.props(),
            points: false,
            written: Vec::new(),
            copy: copied,
        });
        let route = match drag.copy {
            // A copy moves whole, whatever the weights say; `update` refused anything else.
            Some(_) => Route::Placed(
                transform::placed(drag.start.transform, &t).unwrap_or(drag.start.transform),
            ),
            None => transform::route(found, drag.start.transform, &t),
        };
        match route {
            Route::Placed(placed) => {
                if drag.points {
                    found.store_mut().cancel_step();
                    drag.points = false;
                    drag.written.clear();
                }
                let mut props = Props {
                    transform: placed,
                    ..drag.start
                };
                found.swap_props(&mut props);
            }
            Route::Points(t) => {
                if drag.points {
                    found.store_mut().revert_step();
                } else {
                    let mut props = drag.start;
                    found.swap_props(&mut props);
                    found.store_mut().begin_step();
                    drag.points = true;
                }
                // Measured on the surface the drag began on, and logged, as a replay does.
                let place = drag.start.transform;
                let t = (args.settle(found.store(), place, || drag.diagonal)).unwrap_or(t);
                drag.written.extend(transform::apply(found.store_mut(), &t));
                drag.written.sort_unstable();
                drag.written.dedup();
            }
        }
        drag.args = args;
        self.state.gizmos.drag = Some(drag);
    }

    /// Puts a live copy of part `source` in the document as `name`, for a Ctrl drag to move: a
    /// share holding the id the copy will get, beside the part in the list, made the active
    /// part. Gives the source and the part that was active, or `None` when there is no part
    /// `source`.
    fn show_copy(&mut self, source: String, name: &str) -> Option<Copied> {
        let doc = &mut self.state.doc;
        let part = doc.get(&source)?;
        let id = doc.next_id();
        let live = part.share_with(Props {
            id: Some(id),
            ..part.props().as_copy()
        });
        let active = doc.active_id();
        doc.swap(name, Some(Box::new(live)));
        doc.set_active(Some(id));
        Some(Copied { source, active })
    }

    /// Takes a Ctrl drag's live copy `name` away and makes the part active before it active
    /// again, so the document is as it was before the drag.
    fn drop_copy(&mut self, name: &str, copied: &Copied) {
        self.state.doc.swap(name, None);
        self.state.doc.set_active(copied.active);
    }

    /// Ends the drag in progress, if there is one, and commits it as the `transform.apply` call
    /// that replays it: one log line and one undo step, of the points or of the part's
    /// properties. A Ctrl drag's copy is added for good by the `part.duplicate` call that
    /// replays it, run as the drag's client.
    pub(crate) fn end_drag(&mut self) {
        let Some(drag) = self.state.gizmos.drag.take() else {
            return;
        };
        self.state.control.gesture = false;
        if let Some(copied) = &drag.copy {
            self.drop_copy(&drag.part, copied);
            let args = json!({
                "part": copied.source,
                "name": drag.part,
                "transform": { "frame": drag.args.frame, "op": drag.args.op },
            });
            let call = Call {
                steps: vec![Step {
                    command: "part.duplicate".into(),
                    args,
                }],
                if_rev: None,
                dry_run: false,
                batch: false,
            };
            if let Outcome::Error(e) = self.run_now(&drag.origin, &call).outcome {
                self.state.sculpt.problem = Some(e.message);
            }
            return;
        }
        let leveled = self.state.doc.get(&drag.part).and_then(|p| p.levels());
        if drag.points && leveled.is_some_and(|l| l.active() < l.top_level()) {
            // Put back, then run as the call, which may run on the top (decision 0041, DF8).
            let args = serde_json::to_value(&drag.args).unwrap_or_default();
            let (origin, command) = (drag.origin.clone(), "transform.apply".into());
            self.state.gizmos.drag = Some(drag);
            self.cancel_drag();
            let call = Call {
                steps: vec![Step { command, args }],
                if_rev: None,
                dry_run: false,
                batch: false,
            };
            if let Outcome::Error(e) = self.run_now(&origin, &call).outcome {
                self.state.sculpt.problem = Some(e.message);
            }
            return;
        }
        let output = drag.args.applied(drag.part.clone());
        let adopted = Adopted {
            command: "transform.apply",
            args: serde_json::to_value(&drag.args).unwrap_or_default(),
            output: output
                .ok()
                .and_then(|o| serde_json::to_value(o).ok())
                .unwrap_or_default(),
            why: NO_CHANGE.into(),
            part: drag.part,
            written: drag.written,
            props: (!drag.points).then_some(drag.start),
        };
        if let Err(e) = self.commit_in_place(&drag.origin, adopted) {
            self.state.sculpt.problem = Some(e.message);
        }
    }

    /// Ends the drag in progress, if there is one, putting its part back as it was (a Ctrl
    /// drag: taking its copy away).
    fn cancel_drag(&mut self) {
        let Some(drag) = self.state.gizmos.drag.take() else {
            return;
        };
        self.state.control.gesture = false;
        if let Some(copied) = &drag.copy {
            self.drop_copy(&drag.part, copied);
            return;
        }
        if let Some(part) = self.state.doc.get_mut(&drag.part) {
            if drag.points {
                part.store_mut().cancel_step();
            }
            let mut props = drag.start;
            part.swap_props(&mut props);
        }
    }
}
