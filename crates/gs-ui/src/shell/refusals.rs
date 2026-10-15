//! What a person sees when the engine says no (decision 0025, section 2, amended 2026-10-14).
//! The engine writes its refusals for agents: a code, the argument at fault, and a hint that
//! names arguments and values. The window says them in a person's words, from one table
//! ([`ROWS`]): by the refusal's code, and its command, argument or words where those matter,
//! the engine's sentence without its code (or other words where that sentence names what a
//! person never sees), what to do in a person's words in place of the engine's hint, and where
//! it shows: under the field of the form that sent it, the form kept open as typed; or on the
//! status line. The engine's own messages and hints stay as they are, for agents. The window
//! says the system's refusals of its own files in the engine's words, which the table finds
//! (`gs_engine::file_refused`).

use gs_protocol::{CommandError, ErrorCode, Step};

use super::Shell;

/// Where a refusal shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shows {
    /// Under the field of the form that sent the step, kept open as typed; on the status line
    /// where no form did.
    Field,
    /// On the status line: nothing typed caused it.
    Line,
    /// Nowhere: the person asked for it (Cancel).
    Nothing,
}

/// One kind of refusal, and what a person sees of it.
struct Row {
    code: ErrorCode,
    /// The commands it is about; empty for any.
    commands: &'static [&'static str],
    /// The argument it names (`/format`); empty for any.
    at: &'static str,
    /// Words in the engine's message; empty for any.
    has: &'static str,
    /// What it says in place of the engine's message, where that names what a person never
    /// sees; `None` keeps the message.
    says: Option<&'static str>,
    /// What to do, in a person's words: no argument names, no `true`.
    hint: &'static str,
    shows: Shows,
}

/// A row that fits every refusal of its code, for the rows below to narrow.
const ANY: Row = Row {
    code: ErrorCode::Internal,
    commands: &[],
    at: "",
    has: "",
    says: None,
    hint: "",
    shows: Shows::Field,
};

/// The file commands a person names a file for.
const FILES: &[&str] = &[
    "file.open",
    "file.import",
    "file.save",
    "file.export",
    "reference.open",
    "brush.load",
    "brush.save",
    "curve.load",
    "curve.save",
];

/// Every kind of refusal a person meets with words of its own, the first that fits winning.
/// One no row fits shows its message alone, under its field.
const ROWS: &[Row] = &[
    Row {
        code: ErrorCode::ArgsInvalid,
        commands: &["file.import"],
        at: "/format",
        hint: "GigaSculpt imports OBJ, PLY, STL, ZTL and ZPR files.",
        ..ANY
    },
    Row {
        code: ErrorCode::ArgsInvalid,
        commands: &["file.export"],
        at: "/format",
        says: Some("GigaSculpt exports OBJ, PLY and STL files"),
        hint: "End the name in .obj, .ply or .stl, or leave the extension off for OBJ.",
        ..ANY
    },
    Row {
        code: ErrorCode::ArgsInvalid,
        commands: &["file.save"],
        at: "/path",
        hint: "Scene files end in .gss. To write an OBJ, PLY or STL file, use File > Export.",
        ..ANY
    },
    Row {
        code: ErrorCode::NotFound,
        commands: FILES,
        hint: "Check the file's name and its folder.",
        ..ANY
    },
    // A settings file to import that is not there, as the engine's check finds it (gs-engine
    // `ui/layout.rs` `check_import`, in `file::io_error`'s words), or gone after that check,
    // before the window reads it, which the window says in the same words
    // (`gs_engine::file_refused`): matched on those words, as `ui.layout`'s other refusals of
    // a name not found are no file's.
    Row {
        code: ErrorCode::NotFound,
        commands: &["ui.layout"],
        has: "there is no file at",
        hint: "Check the file's name and its folder.",
        ..ANY
    },
    // Matched on the engine's words (`file::has_folder`, which every save asks): naming its
    // path would make Ctrl+S, which gives none, ask where to save. Pinned by the test
    // `refusals::a_good_save_after_a_refused_one_clears_the_line`, which fails if they change.
    Row {
        code: ErrorCode::Precondition,
        has: "no folder",
        hint: "Check the folder's name, or make the folder first.",
        ..ANY
    },
    // A folder where a file is wanted (`file::not_a_folder`), as a form run with only the
    // folder it starts at gives one. Matched on the engine's words, as the brush commands name
    // their path for every refusal, a reference names its `file` and a settings import its
    // `import`; for any command, as only file commands say them. Pinned by the tests
    // `refusals::a_file_form_run_with_no_file_name_asks_for_one` and those in `refused_words`,
    // which fail if they change.
    Row {
        code: ErrorCode::Precondition,
        has: "\" is a folder",
        hint: "Type a file name after the folder.",
        ..ANY
    },
    // A file the system will not let GigaSculpt write: a folder or a disk that may not be
    // written, or a file that may not be replaced, in the words every file refusal has
    // (`gs_engine::file_refused`), the window's own files' too, each coded as a precondition
    // (`ui.layout`'s settings file as well). The keymap's file is the window's own, not one a
    // person picked. Matched on the words; pinned by the tests in `refused_words`, which fail
    // if they change.
    Row {
        code: ErrorCode::Precondition,
        commands: &["ui.keymap"],
        has: "is not allowed to write",
        hint: "Check that the file and its folder are not read-only, then try again.",
        ..ANY
    },
    Row {
        code: ErrorCode::Precondition,
        has: "is not allowed to write",
        hint: "Pick a folder you can write to, or check that the file is not read-only.",
        ..ANY
    },
    // A part carried so far from the origin that f32 would flatten it (gs-engine
    // `document/sound.rs`), as a gizmo drag meets it when it lets go: said with no numbers.
    // Matched on the engine's words; pinned by the test
    // `refused_words::a_gizmo_drag_refused_as_it_ends_reads_as_a_sentence`.
    Row {
        code: ErrorCode::ArgsInvalid,
        has: "f32 rounding would flatten its shape",
        says: Some("That would put the part too far from the scene's origin to keep its shape"),
        hint: "Move it closer to the origin, or scale it up first.",
        ..ANY
    },
    // Nothing to undo or redo, their only unavailable refusal (`history/undo.rs`, `check`):
    // the gray button says so already (review 0009, R2).
    Row {
        code: ErrorCode::Precondition,
        commands: &["history.undo", "history.redo"],
        shows: Shows::Nothing,
        ..ANY
    },
    // A name another part or checkpoint has (`part::taken`, `history.checkpoint`).
    Row {
        code: ErrorCode::Precondition,
        at: "/name",
        hint: "Pick another name.",
        ..ANY
    },
    // A part's levels as Freeze and Unfreeze find them (`level/freeze.rs`): said with no part
    // name in quotes, as the stepper's ends are (review 0009, Refusals-notes (2)), and gray
    // with these words before a tap. Matched on the engine's words; pinned by the test
    // `detail::a_level_pick_that_cannot_run_is_gray_with_the_reason`.
    Row {
        code: ErrorCode::Precondition,
        commands: &["level.freeze"],
        has: "frozen already",
        says: Some("This part's levels are frozen already"),
        hint: "Unfreeze brings them back.",
        ..ANY
    },
    Row {
        code: ErrorCode::Precondition,
        commands: &["level.freeze"],
        has: "has one level",
        says: Some("This part has only one level, so there is none to freeze"),
        hint: "Subdivide adds levels.",
        ..ANY
    },
    Row {
        code: ErrorCode::Precondition,
        commands: &["level.unfreeze"],
        has: "is not frozen",
        says: Some("This part has no frozen levels to bring back"),
        hint: "Freeze sets them aside first.",
        ..ANY
    },
    Row {
        code: ErrorCode::Precondition,
        commands: &["ui.keymap"],
        at: "/replace",
        hint: "Replace gives the key to the new command, and what holds it now loses it.",
        ..ANY
    },
    // A person meets Busy only from a job (a pen stroke's refusal too): a save while the pen
    // holds the session waits for the lift (`gizmo::deformer::held_back`), and a stroke under
    // way holds back agents only.
    Row {
        code: ErrorCode::Busy,
        hint: "Its Cancel is beside its bar on the status line.",
        ..ANY
    },
    Row {
        code: ErrorCode::Timeout,
        hint: "Try again.",
        ..ANY
    },
    Row {
        code: ErrorCode::Stale,
        hint: "The scene changed while it ran: try again.",
        ..ANY
    },
    Row {
        code: ErrorCode::Internal,
        hint: "This is a fault in GigaSculpt, not in what you did.",
        shows: Shows::Line,
        ..ANY
    },
    Row {
        code: ErrorCode::UnknownCommand,
        hint: "This version of GigaSculpt has no such command.",
        shows: Shows::Line,
        ..ANY
    },
    Row {
        code: ErrorCode::Cancelled,
        shows: Shows::Nothing,
        ..ANY
    },
];

impl Row {
    /// Whether it is about the refusal `e` of `command`.
    fn fits(&self, command: &str, e: &CommandError) -> bool {
        self.code == e.code
            && (self.commands.is_empty() || self.commands.contains(&command))
            && (self.at.is_empty() || e.path.as_deref() == Some(self.at))
            && (self.has.is_empty() || e.message.contains(self.has))
    }
}

/// The refusal `e` of `step` (`None` for the pen's samples) in a person's words, and where it
/// shows: its sentence, the names it suggests for a name not found, then what to do.
pub(crate) fn words(step: Option<&Step>, e: &CommandError) -> (String, Shows) {
    let command = step.map_or("", |s| s.command.as_str());
    let row = ROWS.iter().find(|r| r.fits(command, e));
    let said = row.and_then(|r| r.says).unwrap_or(&e.message);
    let mut text = sentence(said);
    if e.code == ErrorCode::NotFound && !e.suggestions.is_empty() {
        let names: Vec<String> = e.suggestions.iter().map(|s| format!("'{s}'")).collect();
        text = joined(&text, &format!("Did you mean {}?", names.join(" or ")));
    }
    if let Some(hint) = row.map(|r| r.hint).filter(|h| !h.is_empty()) {
        text = joined(&text, hint);
    }
    (text, row.map_or(Shows::Field, |r| r.shows))
}

/// `first` then `then`, a full stop between them unless `first` ends a sentence already.
fn joined(first: &str, then: &str) -> String {
    let stop = if first.ends_with(['.', '?', '!']) {
        ""
    } else {
        "."
    };
    format!("{first}{stop} {then}")
}

/// `text` as a sentence starts: trimmed, its first letter a capital.
pub(crate) fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut letters = text.chars();
    let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
    first.unwrap_or_default() + letters.as_str()
}

/// Steps that only open a piece of the window, move the view or follow a held key: sending one
/// leaves the status line's problem, which the person may still be reading.
const QUIET: &[&str] = &[
    "ui.palette",
    "ui.menu",
    "ui.quick",
    "ui.wheel",
    "ui.library",
    "ui.gizmo",
    "ui.curve",
    "brush.slot",
];

/// Whether sending `step` is the person's next step, which clears the problem line: any but
/// [`QUIET`]'s and a move of the view.
pub(crate) fn moves_on(step: &Step) -> bool {
    let command = step.command.as_str();
    !QUIET.contains(&command) && !command.starts_with("view.")
}

impl Shell {
    /// A step the window ran (`step`; `None` for the pen's samples) was refused with `e`, said
    /// in a person's words ([`words`]). When the refusal asks for an argument the step does not
    /// give, the palette asks for it instead (decision 0016, section 17, amended 2026-10-13):
    /// Open with unsaved changes asks whether to save them, a first Save where, a save onto
    /// another file whether to replace it, a taken key whether to replace its holder; a
    /// question that does not say why itself says it in place of its line. Otherwise the form
    /// that sent the step opens again as typed, with why under its field, or the status line
    /// says why. A save refused while the pen holds the session waits for the lift instead
    /// (`gizmo::deformer::held_back`).
    pub fn refused(&mut self, step: Option<Step>, e: &CommandError) {
        let (waiting, gesture) = (&mut self.status.waiting, self.state.gesture);
        if crate::gizmo::deformer::held_back(waiting, step.as_ref(), e, gesture) {
            return;
        }
        let (said, shows) = words(step.as_ref(), e);
        let person = CommandError {
            message: said.clone(),
            ..e.clone()
        };
        if step
            .as_ref()
            .is_some_and(|s| self.adjust.refused(s, &person))
        {
            return;
        }
        let asked = (step.as_ref()).and_then(|s| gs_engine::errors::asks(e, &s.args));
        match (step, asked.map(String::from)) {
            (Some(step), Some(asked)) => {
                if !self.palette.open {
                    self.palette.at = self.pointer;
                }
                if let Err(problem) = self.palette.refused(step, &self.catalog, asked, &said) {
                    self.status.says(&problem);
                }
            }
            (step, _) => self.failed(step.as_ref(), e.path.as_deref(), &said, shows),
        }
    }

    /// A failure, not a question, said as `said` where `shows` puts it, `at` the argument it
    /// names; a save the question before unsaved changes are lost waits for goes with it.
    fn failed(&mut self, step: Option<&Step>, at: Option<&str>, said: &str, shows: Shows) {
        self.palette.unsaved.stop();
        let pointer = self.pointer;
        match shows {
            Shows::Nothing => {}
            Shows::Field if step.is_some_and(|s| self.palette.reopen(s, at, said)) => {
                self.palette.at = pointer;
            }
            Shows::Field | Shows::Line => self.status.says(said),
        }
    }
}
