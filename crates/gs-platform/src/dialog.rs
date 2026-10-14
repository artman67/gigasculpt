//! The system's file dialogs, through SDL (decision 0016, section 17, amended 2026-10-13): open a
//! file, save a file, or pick a folder. SDL shows the desktop portal's dialog on Linux (zenity
//! where there is no portal) and the system's own on Windows and macOS. A dialog never blocks:
//! its answer arrives later from [`Dialogs::answers`], which the window hands the interface as
//! an [`Input::Dialog`](crate::Input::Dialog) with the rest of its input. Where no dialog can
//! show, the answer is a failure at once, and the interface asks in its own form instead.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use sdl3::dialog::{self, DialogError, DialogFileFilter};

use crate::window::Platform;

/// Which dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Pick a file that exists.
    Open,
    /// Name a file to write; the dialog asks before it picks one that exists.
    Save,
    /// Pick a folder.
    Folder,
}

/// A filter a file dialog offers: its name, and the extensions it shows with `;` between them,
/// or `*` for every file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub pattern: String,
}

/// A dialog to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub pick: Pick,
    /// The filters, in the order offered; none for a folder.
    pub filters: Vec<Filter>,
    /// The folder it starts in.
    pub folder: Option<PathBuf>,
    /// The file name a save suggests.
    pub name: Option<String>,
}

/// What a dialog answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The file or folder picked.
    Chosen(PathBuf),
    /// Closed with nothing picked.
    Cancelled,
    /// No dialog could show, and why.
    Failed(String),
}

/// The dialogs' answers, as they arrive.
pub struct Dialogs {
    sender: Sender<Answer>,
    receiver: Receiver<Answer>,
}

impl Default for Dialogs {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver }
    }
}

impl Dialogs {
    /// Shows `dialog` over the window. Its answer comes from [`Dialogs::answers`], at once as a
    /// failure where no dialog can show.
    pub fn show(&self, platform: &Platform, dialog: &Dialog) {
        let sender = self.sender.clone();
        // SDL calls back once, on whichever thread the dialog ran on.
        let callback: dialog::DialogCallback = Box::new(move |result, _| {
            sender.send(answer(result)).ok();
        });
        let filters: Vec<DialogFileFilter> = (dialog.filters.iter())
            .map(|f| DialogFileFilter {
                name: &f.name,
                pattern: &f.pattern,
            })
            .collect();
        let (at, window) = (start(dialog), platform.sdl_window());
        let shown = match dialog.pick {
            Pick::Open => dialog::show_open_file_dialog(&filters, at, false, window, callback),
            Pick::Save => dialog::show_save_file_dialog(&filters, at, window, callback),
            Pick::Folder => {
                dialog::show_open_folder_dialog(at, false, window, callback);
                Ok(())
            }
        };
        if let Err(e) = shown {
            self.sender.send(Answer::Failed(e.to_string())).ok();
        }
    }

    /// The answers that arrived since the last call, oldest first.
    pub fn answers(&self) -> Vec<Answer> {
        self.receiver.try_iter().collect()
    }
}

/// What a dialog's result says: its first file, or cancelled with none.
fn answer(result: Result<Vec<PathBuf>, DialogError>) -> Answer {
    match result {
        Ok(files) => files
            .into_iter()
            .next()
            .map_or(Answer::Cancelled, Answer::Chosen),
        Err(DialogError::Canceled) => Answer::Cancelled,
        Err(e) => Answer::Failed(e.to_string()),
    }
}

/// Where `dialog` starts: a save's folder with its suggested name, else its folder. `None` for
/// a path SDL cannot take (not UTF-8, or holding a NUL), which it would panic on.
fn start(dialog: &Dialog) -> Option<PathBuf> {
    let at = match (&dialog.name, dialog.pick) {
        (Some(name), Pick::Save) => Some(dialog.folder.clone().unwrap_or_default().join(name)),
        _ => dialog.folder.clone(),
    };
    at.filter(|p| p.to_str().is_some_and(|s| !s.contains('\0')))
}
