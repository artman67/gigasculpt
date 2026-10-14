//! The Reference Views among the session's views (decisions 0020 and 0024): which are open,
//! which is active, and opening, closing and resetting one.

use gs_core::camera::Camera;

use super::{DEFAULT_VIEWPORT, View, Views, reference_number};
use crate::document::Document;
use crate::view::secondary::Secondary;

impl Views {
    /// View `name`'s Reference View setup, if it is an open one.
    pub(crate) fn secondary(&self, name: &str) -> Option<&Secondary> {
        self.list.get(name)?.secondary.as_ref()
    }

    /// The open Reference Views, by number.
    pub(crate) fn references(&self) -> Vec<String> {
        let mut open: Vec<(u32, &String)> = (self.list.iter())
            .filter(|(_, v)| v.secondary.is_some())
            .filter_map(|(name, _)| Some((reference_number(name)?, name)))
            .collect();
        open.sort();
        open.into_iter().map(|(_, name)| name.clone()).collect()
    }

    /// The active Reference View: what the `reference.*` commands act on when `view` is left
    /// out, and whose controls the window's header shows (decision 0024). The one opened last
    /// or moved to by `reference.next`, else the lowest-numbered open one.
    pub(crate) fn active_reference(&self) -> Option<String> {
        let kept = (self.active.clone()).filter(|a| self.secondary(a).is_some());
        kept.or_else(|| self.references().into_iter().next())
    }

    /// Makes the open reference after `from`, by number, active, wrapping around; returns it.
    /// `None` when `from` is not open.
    pub(crate) fn next_reference(&mut self, from: &str) -> Option<String> {
        let open = self.references();
        let at = open.iter().position(|r| r == from)?;
        let next = open[(at + 1) % open.len()].clone();
        self.active = Some(next.clone());
        Some(next)
    }

    /// The size a Reference View that opens now is seen at: the window's reference picture as
    /// last shown, else a snapshot's.
    pub(crate) fn new_reference_size(&self) -> [u32; 2] {
        self.reference_seen.unwrap_or(DEFAULT_VIEWPORT)
    }

    /// The lowest-numbered Reference View that is not open, for `reference.open`.
    pub(crate) fn free_reference(&self) -> String {
        // Among ref:1 to ref:(n+1), one is free: at most n are open.
        let open = self.references().len() as u32;
        let free = (1..=open + 1).find(|n| self.secondary(&format!("ref:{n}")).is_none());
        format!("ref:{}", free.unwrap_or(open + 1))
    }

    /// Opens Reference View `name` with `setup`, seen at `viewport`, its own camera `camera`,
    /// and makes it active; one opened before under that name is replaced, history and all.
    pub(crate) fn open(
        &mut self,
        name: &str,
        setup: Secondary,
        viewport: [u32; 2],
        camera: Camera,
    ) {
        let view = View {
            camera: Some(camera),
            viewport,
            secondary: Some(setup),
            ..View::default()
        };
        self.list.insert(name.to_string(), view);
        self.active = Some(name.to_string());
    }

    /// Closes Reference View `name`: its scene and history go.
    pub(crate) fn close_reference(&mut self, name: &str) {
        self.list.remove(name);
        if self.active.as_deref() == Some(name) {
            self.active = None;
        }
    }

    /// Changes Reference View `name`'s setup, keeping the camera it shows now: what `f`
    /// unlinks keeps its current value as the view's own, so nothing jumps.
    pub(crate) fn reset_reference(
        &mut self,
        name: &str,
        doc: &Document,
        f: impl FnOnce(&mut Secondary),
    ) {
        let now = self.resolved(name, doc);
        if let Some(view) = self.list.get_mut(name)
            && let Some(secondary) = view.secondary.as_mut()
        {
            f(secondary);
            view.camera = Some(now);
        }
    }
}
