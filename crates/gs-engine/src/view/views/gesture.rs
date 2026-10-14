//! Camera changes and gestures: a change starts with [`Views::moving`] and ends with
//! [`Views::moved`]; calls sharing a gesture id are one history step, logged once as the camera
//! the gesture ended on.

use gs_core::Quat;
use gs_core::camera::Camera;
use gs_protocol::CommandError;

use super::{Moving, Part, Views, locked};
use crate::ctx::Origin;
use crate::document::Document;
use crate::view::secondary::Touches;

/// A gesture in progress on a view.
#[derive(Clone, Debug)]
pub(super) struct Gesture {
    pub(super) id: u64,
    origin: Origin,
    /// The point it turns about, fixed when it starts.
    pivot: gs_core::Vec3,
    /// The orientation before snapping, while a snapping orbit is held.
    free: Option<Quat>,
    /// It started upside down for its orbit mode, so it turns sideways the other way, fixed
    /// by its first orbit.
    flip: Option<bool>,
}

/// A gesture that ended, or a framing kept, waiting for its one log line.
#[derive(Clone, Debug)]
pub(crate) struct Ended {
    pub view: String,
    pub origin: Origin,
    pub camera: Camera,
    pub viewport: [u32; 2],
    /// It is a step of the camera history (a framing kept is not).
    pub remember: bool,
}

impl Views {
    /// Starts a camera change of view `name`, seen at `viewport`, that moves what `moves`
    /// says: refuses on a locked view and on a Reference View whose link holds what it moves,
    /// takes one history step unless it continues the view's gesture, and opens a gesture when
    /// `part` names a new one. A gesture keeps the pivot it started with; any other change
    /// closes the gesture open on the view first.
    pub(crate) fn moving(
        &mut self,
        (name, moves): (&str, Touches),
        origin: &Origin,
        (viewport, doc): ([u32; 2], &Document),
        part: Part,
        pivot: gs_core::Vec3,
    ) -> Result<Moving, CommandError> {
        if self.view(name).locked {
            return Err(locked(name));
        }
        if let Some(secondary) = self.secondary(name) {
            secondary.allows(name, moves)?;
        }
        let continues = part.gesture.is_some()
            && self.view(name).gesture.as_ref().map(|g| g.id) == part.gesture;
        if !continues {
            self.close(name);
        }
        self.seen_at(name, viewport);
        let camera = self.resolved(name, doc);
        let view = self.view_mut(name);
        if continues {
            let pivot = view.gesture.as_ref().map_or(pivot, |g| g.pivot);
            return Ok(Moving { camera, pivot });
        }
        if part.remember {
            view.remember();
        }
        view.gesture = part.gesture.map(|id| Gesture {
            id,
            origin: origin.clone(),
            pivot,
            free: None,
            flip: None,
        });
        Ok(Moving { camera, pivot })
    }

    /// Ends a camera change: `camera` is the view's from now on, and the gesture ends if
    /// `part` says so.
    pub(crate) fn moved(&mut self, name: &str, camera: Camera, part: Part) {
        self.view_mut(name).camera = Some(camera);
        if part.end {
            self.close(name);
        }
    }

    /// The orientation the snapping orbit of view `name`'s gesture holds before snapping.
    pub(crate) fn held_turn(&self, name: &str) -> Option<Quat> {
        self.list.get(name)?.gesture.as_ref()?.free
    }

    /// Holds `turn` as the gesture's orientation before snapping, or forgets it.
    pub(crate) fn hold_turn(&mut self, name: &str, turn: Option<Quat>) {
        if let Some(g) = self.view_mut(name).gesture.as_mut() {
            g.free = turn;
        }
    }

    /// View `name` may turn now: it is not locked, and no Reference View link holds its turn.
    pub(crate) fn may_turn(&self, name: &str) -> bool {
        let held = self.secondary(name).map(|s| s.allows(name, Touches::TURN));
        !self.view(name).locked && held.is_none_or(|r| r.is_ok())
    }

    /// Whether an orbit of view `name` turns sideways the other way: the gesture's answer,
    /// fixed by its first orbit as `now` (the camera upside down when it starts), else `now`.
    pub(crate) fn flipped(&mut self, name: &str, now: bool) -> bool {
        match self.view_mut(name).gesture.as_mut() {
            Some(g) => *g.flip.get_or_insert(now),
            None => now,
        }
    }

    /// Closes the gesture open on view `name`, if any: it waits for its log line.
    pub(crate) fn close(&mut self, name: &str) {
        let Some(view) = self.list.get_mut(name) else {
            return;
        };
        let Some(gesture) = view.gesture.take() else {
            return;
        };
        if let Some(camera) = view.camera {
            self.ended.push(Ended {
                view: name.to_string(),
                origin: gesture.origin,
                camera,
                viewport: view.viewport,
                remember: true,
            });
        }
    }

    /// The gestures that ended since the last time, oldest first.
    pub(crate) fn take_ended(&mut self) -> Vec<Ended> {
        std::mem::take(&mut self.ended)
    }

    /// The id of the gesture open on view `name`.
    pub(crate) fn gesture(&self, name: &str) -> Option<u64> {
        self.list.get(name)?.gesture.as_ref().map(|g| g.id)
    }
}
