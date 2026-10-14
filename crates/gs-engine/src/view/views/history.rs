//! Each view's camera history: [`HISTORY`] steps back, and what can be redone.

use gs_protocol::{CommandError, ErrorCode};

use super::{View, Views, locked};
use crate::errors::error;

/// Camera history steps each view keeps.
pub const HISTORY: usize = 50;

impl Views {
    /// Steps view `name` back (`forward` false) or forward through its camera history.
    pub(crate) fn travel(&mut self, name: &str, forward: bool) -> Result<(), CommandError> {
        if self.view(name).locked {
            return Err(locked(name));
        }
        self.close(name);
        let view = self.view_mut(name);
        let (from, to) = if forward {
            (&mut view.redo, &mut view.undo)
        } else {
            (&mut view.undo, &mut view.redo)
        };
        let Some(camera) = from.pop() else {
            let which = if forward { "later" } else { "earlier" };
            return Err(error(
                ErrorCode::Precondition,
                format!("{name} has no {which} camera in its history"),
            ));
        };
        to.push(view.camera);
        view.camera = camera;
        Ok(())
    }
}

impl View {
    /// Puts the camera now on the history, and clears what could be redone.
    pub(super) fn remember(&mut self) {
        if self.undo.len() == HISTORY {
            self.undo.remove(0);
        }
        self.undo.push(self.camera);
        self.redo.clear();
    }
}
