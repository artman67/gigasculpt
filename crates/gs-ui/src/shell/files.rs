//! The window's part in scene files (decision 0016, section 17, amended 2026-10-13): its title,
//! closing it, a file dropped on it or named on the command line, and the system's file
//! dialogs, which the window shows and whose answers come back as input.

use std::path::Path;

use gs_platform::{Dialog, Input};
use gs_protocol::Step;

use super::Shell;
use crate::palette::{Then, opened};

impl Shell {
    /// The window's title: the scene's file name, `*` first while it has unsaved changes, then
    /// the program's name; the program's name alone for a new scene with nothing in it to lose.
    pub fn title(&self) -> String {
        let changed = if self.state.unsaved { "*" } else { "" };
        let file = self.state.file.as_deref();
        match file.and_then(|f| Path::new(f).file_name()?.to_str()) {
            Some(name) => format!("{changed}{name} - GigaSculpt"),
            None if self.state.unsaved => "*Untitled - GigaSculpt".into(),
            None => "GigaSculpt".into(),
        }
    }

    /// Takes in what the window heard besides keys and the pen: closing it asks about unsaved
    /// changes first; a file dropped on it opens (asking first too) or is imported; a file
    /// dialog's answer runs its step. What cannot be done, the status line says.
    pub fn window_input(&mut self, inputs: &[Input]) {
        for input in inputs {
            let ran = match input {
                Input::Close => self.palette.first(Then::Close, &self.catalog),
                Input::Dropped(path) => self.dropped(path),
                Input::Dialog(answer) => self.palette.answered(answer.clone(), &self.catalog),
                _ => Ok(None),
            };
            match ran {
                Ok(Some(step)) => self.queue(step),
                Ok(None) => {}
                Err(problem) => self.status.says(&problem),
            }
        }
    }

    /// A file dropped on the window: a scene opens, asking about unsaved changes first; a mesh
    /// file is imported.
    fn dropped(&mut self, path: &Path) -> Result<Option<Step>, String> {
        let step = opened(path)?;
        if step.command == "file.open" {
            return self
                .palette
                .first(Then::Run(step, Vec::new()), &self.catalog);
        }
        Ok(Some(step))
    }

    /// Asks for New, Open or Recover as a control does, asking first about unsaved changes and
    /// then for the arguments named in `also` (Open's file); with nothing to lose and nothing to
    /// ask, it runs at once.
    pub(super) fn ask_dropping(&mut self, step: Step, also: &[String]) -> Result<(), String> {
        let also = (also.iter())
            .filter(|a| *a != "discard_changes")
            .cloned()
            .collect();
        if let Some(now) = self.palette.first(Then::Run(step, also), &self.catalog)? {
            self.queue(now);
        }
        Ok(())
    }

    /// Whether the window may close now: asked to, with nothing to lose or the question
    /// answered.
    pub fn closes(&self) -> bool {
        self.palette.unsaved.closes
    }

    /// The file dialog the interface asked for since the last call, for the window to show.
    pub fn take_dialog(&mut self) -> Option<Dialog> {
        self.palette.take_dialog()
    }

    /// Opens the scene named on the command line as the window starts: it takes the place of
    /// opening the last scene.
    pub fn open_at_start(&mut self, step: Step) {
        self.outbox.retain(|s| s.command != "file.open");
        self.queue(step);
    }
}
