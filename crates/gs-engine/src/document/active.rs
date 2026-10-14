//! The active part (decision 0033): the part a command works on when none is named, and the one
//! that always shows. Session state kept by stable id, as Isolate is (decision 0029, section
//! 9): in no undo step, put back by a rollback, saved in a scene file's session section.
//!
//! A part entering the document with a new id becomes active ([`Document::swap`]), and
//! `part.select` makes one active. When a call takes the active part away, the part after it
//! in the list takes its place (else the last part), and the call's end keeps that choice
//! ([`Document::settle`]). An undo step records the active part on each side of it, and undo
//! and redo put that back ([`Document::reselect`]).

use super::{Document, Isolate, Part, Showing};

impl Document {
    /// The active part's id: the part made active last while it is in the document; else, after
    /// a call took it away, the part after it in the list, or the last; else the first part.
    /// `None` only in an empty scene.
    pub(crate) fn active_id(&self) -> Option<u64> {
        if let Some(id) = self.active
            && self.parts.values().any(|p| p.props.id == Some(id))
        {
            return Some(id);
        }
        let list = self.list();
        let key = |p: &Part| (p.props.rank, p.props.id.unwrap_or_default());
        let next = match self.gone {
            Some(gone) => (list.iter()).find(|(_, p)| key(p) > gone).or(list.last()),
            None => list.first(),
        };
        next.and_then(|(_, p)| p.props.id)
    }

    /// The active part's name; `None` only in an empty scene.
    pub(crate) fn active(&self) -> Option<&str> {
        let id = self.active_id()?;
        (self.parts.iter())
            .find(|(_, p)| p.props.id == Some(id))
            .map(|(name, _)| name.as_str())
    }

    /// What decides whether a part shows: the isolation in effect and the active part.
    pub(crate) fn showing(&self) -> Showing<'_> {
        Showing {
            isolation: self.isolation(),
            active: self.active_id(),
        }
    }

    /// Makes the part with id `id` active (`None`: the first part).
    pub(crate) fn set_active(&mut self, id: Option<u64>) {
        self.active = id;
        self.gone = None;
    }

    /// Makes part `name` active, if there is one.
    pub(crate) fn select(&mut self, name: &str) {
        if let Some(part) = self.parts.get(name) {
            self.set_active(part.props.id);
        }
    }

    /// Makes the part with id `id` active again if it is in the document and Isolate does not
    /// set it aside (the active part always shows): an undo or redo putting back the active
    /// part of its side of the step.
    pub(crate) fn reselect(&mut self, id: Option<u64>) {
        let back = |p: &Part| p.props.id == id && self.isolation.mark(p) != Isolate::Aside;
        if id.is_some() && self.parts.values().any(|p| back(p)) {
            self.set_active(id);
        }
    }

    /// Notes that `part` left the document: if it was the active part, where it sat.
    pub(super) fn left(&mut self, part: &Part) {
        if part.props.id.is_some() && part.props.id == self.active {
            self.gone = Some((part.props.rank, part.props.id.unwrap_or_default()));
        }
    }

    /// Keeps the active part as it is now, at the end of a call: a part that took the place of
    /// one the call took away stays active even if an undo brings the old one back.
    pub(crate) fn settle(&mut self) {
        let id = self.active_id();
        self.set_active(id);
    }

    /// The active part and where it left, to put back after a failed call.
    pub(super) fn active_state(&self) -> (Option<u64>, Option<(u64, u64)>) {
        (self.active, self.gone)
    }

    /// Puts back what [`Document::active_state`] gave.
    pub(super) fn restore_active(&mut self, (active, gone): (Option<u64>, Option<(u64, u64)>)) {
        self.active = active;
        self.gone = gone;
    }
}
