//! Scene files (decision 0016): `file.save`, `file.quick_save`, `file.save_next`, `file.open`,
//! `file.revert`, `file.new`, `file.recent`, `file.inspect`, `file.recovery` and
//! `file.recover`, autosave and its settings (`file.autosave`), and what lands on the engine
//! thread when a save, an open or a change of settings ends.
//!
//! The format is gs-io's (`gs_io::scene`); this folder turns the document into a file and back
//! (`model`) and writes it (`write`). Saves and opens are background commands: their job runs
//! against the session's copy and hands the engine thread a [`Landing`], which
//! [`Engine::land_all`] applies when the call commits, before its log line is written.

mod autosave;
mod autosave_settings;
mod disk;
mod inspect;
mod model;
mod new;
mod next;
mod open;
mod recent;
mod recover;
mod save;
mod write;

use std::path::{Path, PathBuf};

use gs_protocol::{CommandError, LogRecord, Outcome, Role, Success};
use serde_json::{Value, json};

use crate::document::Document;
use crate::engine::{Done, Engine};
use crate::gizmo::Gizmos;
use crate::history::History;
use crate::material::PaletteState;
use crate::view::saved::{self, Reopened};
use crate::{Ctx, Origin, Unavailable, errors};

pub use autosave::StateFolder;
pub(crate) use autosave::{Autosave, marked_logs};
pub use autosave_settings::AutosaveSettings;
pub use disk::FreeSpace;
pub(crate) use disk::{LowDisk, low_disk};
pub(crate) use recent::{RecentFile, shown_recent};
pub(crate) use recover::newest_autosave;

/// The scene's file, as the session knows it.
#[derive(Clone, Debug, Default)]
pub(crate) struct FileState {
    /// The file the scene was last saved to or opened from, if any.
    pub path: Option<PathBuf>,
    /// The revision that file holds: the scene has no unsaved changes while the document is
    /// at it. `u64::MAX` when no file holds the scene as it is (opened with refused parts,
    /// recovered).
    pub saved_rev: u64,
    /// The scene's id, made at its first save or autosave and kept in its files.
    pub id: Option<String>,
    /// The format version of the file the scene was opened from.
    pub format: Option<u32>,
    /// Counts opens, so a save that an open overtook lands as nothing.
    pub generation: u64,
    /// The palette's step count (`PaletteState::steps`) when the scene was last saved or opened:
    /// a palette step since is an unsaved change too (decision 0043, section 7).
    pub palette_steps: u64,
    /// The Part file saved last in this session, which Save Next for Parts counts from
    /// (section 17, item 8).
    pub parts_path: Option<PathBuf>,
    /// The file a save just wrote, until the window is told (`CallReport::saved`, review 0009,
    /// R11); never autosave's copy.
    pub wrote: Option<PathBuf>,
}

impl FileState {
    /// True if the document at revision `rev` differs from its file.
    pub fn edited(&self, rev: u64) -> bool {
        rev != self.saved_rev
    }

    /// True if the scene differs from its file: the document at revision `rev`, or the palette
    /// at step count `steps`.
    pub fn unsaved(&self, rev: u64, steps: u64) -> bool {
        self.edited(rev) || steps != self.palette_steps
    }

    /// The scene's name: its file's name without the extension, or `untitled`.
    pub fn name(&self) -> String {
        (self.path.as_ref())
            .and_then(|p| p.file_stem())
            .map_or_else(|| "untitled".into(), |s| s.to_string_lossy().into_owned())
    }
}

/// What a background command hands the engine thread.
pub(crate) enum Landing {
    Saved(Saved),
    Opened(Box<Opened>),
    /// Autosave's new settings (`file.autosave`).
    Autosave(StateFolder),
}

/// What a save wrote.
pub(crate) enum SaveKind {
    /// The scene, which now belongs to the file.
    Scene,
    /// Some Parts, as a Part file: the scene keeps its own file.
    Parts,
    /// A copy in the autosave folder: a quick save's, or autosave's own.
    Quick { autosave: bool },
}

pub(crate) struct Saved {
    pub path: PathBuf,
    pub kind: SaveKind,
    /// The revision the save was admitted at: what the file holds.
    pub rev: u64,
    /// The palette's step count then.
    pub palette_steps: u64,
    pub generation: u64,
    pub id: String,
}

pub(crate) struct Opened {
    pub doc: Document,
    /// The file the scene now belongs to, if any.
    pub path: Option<PathBuf>,
    /// True if that file holds the scene as it is now: no unsaved changes.
    pub clean: bool,
    /// A crashed session's marker to remove once this lands (recovery).
    pub forget: Option<PathBuf>,
    /// The session seed and next command number the file stores.
    pub session: Option<(u64, u64)>,
    pub id: Option<String>,
    /// The format version of the file opened; `None` for a new scene.
    pub format: Option<u32>,
    /// The revision the open was admitted at.
    pub rev: u64,
    /// The call let unsaved changes go.
    pub discard: bool,
    /// Log lines to replay once the open is logged (recovery).
    pub replay: Vec<LogRecord>,
    /// The command and arguments to log instead of the call's own.
    pub log_as: Option<(String, Value)>,
    /// The Reference Views the file keeps, in place of the session's.
    pub references: Reopened,
    /// The file to put at the top of the recent files (section 16): the one opened, which a
    /// recovery's autosave copy and a new scene are not.
    pub recent: Option<PathBuf>,
    /// The background the file keeps, in place of the session's (`view.background`).
    pub background: Option<crate::Background>,
    /// The material palette the file opens with, in place of the session's: its own, or the
    /// shipped one (decision 0043, section 7). `None` keeps the session's (`file.new`).
    pub palette: Option<PaletteState>,
}

/// Refuses, saying `why` and `fix`, while the scene has no file of its own (`file.revert` and
/// `file.save_next` need one).
pub(crate) fn has_file(cx: &Ctx, why: &str, fix: &str) -> Result<(), Unavailable> {
    match cx.file.path {
        Some(_) => Ok(()),
        None => Err(Unavailable::new(why).fix(fix)),
    }
}

/// Whether a call may replace the scene: not while it has unsaved changes, its palette's
/// included, unless the call lets them go.
pub(crate) fn replaceable(cx: &Ctx, discard_changes: bool) -> Result<(), Unavailable> {
    ask_first(cx.file.unsaved(cx.rev, cx.palette.steps), discard_changes)
}

/// Refuses while there are `unsaved` changes a call would lose, unless it lets them go. The
/// refusal names `discard_changes`, so the window asks (section 17).
pub(crate) fn ask_first(unsaved: bool, discard_changes: bool) -> Result<(), Unavailable> {
    if unsaved && !discard_changes {
        return Err(Unavailable::new("the scene has unsaved changes")
            .fix("save it first, or pass discard_changes: true to let them go")
            .asks("discard_changes"));
    }
    Ok(())
}

/// Writes `text` as the file `name` in `folder`, made if need be: under a temporary name, then
/// renamed, so a crash never leaves half a file.
pub(crate) fn write_whole(folder: &Path, name: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let temporary = folder.join(format!(".{name}-{}", std::process::id()));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, folder.join(name))
}

impl Engine {
    /// Applies every landing of a finished call, in order, before it commits. Returns the log
    /// lines to replay after the commit (recovery). Fails, applying nothing more, when an open
    /// cannot land.
    pub(crate) fn land_all(&mut self, done: &mut Done) -> Result<Vec<LogRecord>, CommandError> {
        let mut replay = Vec::new();
        for ran in &mut done.ran {
            match ran.landing.take() {
                Some(Landing::Saved(saved)) => self.saved(saved),
                Some(Landing::Opened(opened)) => {
                    let mut opened = *opened;
                    done.log_as = opened.log_as.take();
                    replay = self.opened_scene(opened)?;
                }
                Some(Landing::Autosave(config)) => self.config.autosave = Some(config),
                None => {}
            }
        }
        Ok(replay)
    }

    fn saved(&mut self, s: Saved) {
        let now = self.clock.now();
        let file = &mut self.state.file;
        if s.generation != file.generation {
            return;
        }
        file.id = Some(s.id);
        if !matches!(s.kind, SaveKind::Quick { autosave: true }) {
            file.wrote = Some(s.path.clone());
        }
        let covered = match s.kind {
            SaveKind::Scene => {
                file.path = Some(s.path.clone());
                file.saved_rev = s.rev;
                file.palette_steps = s.palette_steps;
                file.format = Some(gs_io::scene::FORMAT);
                None
            }
            SaveKind::Parts => {
                file.parts_path = Some(s.path.clone());
                None
            }
            SaveKind::Quick { .. } => Some((s.rev, s.palette_steps)),
        };
        self.autosave.saved(now, covered);
        self.mark_session();
        if matches!(s.kind, SaveKind::Scene) {
            self.remember(&s.path);
        }
    }

    /// Puts `path` at the top of the recent files, in a session that keeps them (section 16).
    /// A list that cannot be written shows in the status line; the save or open still stands.
    fn remember(&mut self, path: &Path) {
        let Some(config) = &self.config.autosave else {
            return;
        };
        if let Err(e) = recent::record(&config.state_dir, path) {
            let shown = path.display();
            self.state.sculpt.problem = Some(format!("could not list {shown} as recent: {e}"));
        }
    }

    /// Replaces the session's scene with an opened one (decision 0016, section 6).
    fn opened_scene(&mut self, o: Opened) -> Result<Vec<LogRecord>, CommandError> {
        if self.rev != o.rev && !o.discard {
            // Only file.new lands without a file's format.
            let (command, why) = match o.format {
                Some(_) => ("file.open", "the scene changed while the file loaded"),
                None => ("file.new", "the scene changed"),
            };
            return Err(crate::job::stale(command, why));
        }
        self.install(o.doc);
        if let Some((seed, counter)) = o.session {
            self.config.seed = seed;
            self.next_seq = counter;
        }
        self.rev += 1;
        let file = &mut self.state.file;
        file.saved_rev = if o.clean { self.rev } else { u64::MAX };
        (file.path, file.id, file.format) = (o.path, o.id, o.format);
        file.generation += 1;
        saved::land(&mut self.state.views, o.references);
        if let Some(background) = o.background {
            self.state.display.background = background;
        }
        if let Some(palette) = o.palette {
            self.opened_palette(palette);
        }
        let header = self.log_header();
        let path = self.config.log_path.clone();
        if let Err(e) = self.log.restart(path.as_deref(), &header) {
            self.state.sculpt.problem = Some(format!("could not start a new command log: {e}"));
        }
        let holds = (self.rev, self.state.palette.steps);
        self.autosave.saved(self.clock.now(), Some(holds));
        self.mark_session();
        if let Some(marker) = o.forget {
            self.autosave.recovered();
            recover::forget(&marker);
        }
        if let Some(path) = &o.recent {
            self.remember(path);
        }
        Ok(o.replay)
    }

    /// Replaces the session's palette with an opened file's, its history empty and the user
    /// folders read kept (`Engine::read_folders_again` reads them into a bare one), and hands it to
    /// the renderer. A renderer that cannot draw it says so in the status line; the palette
    /// stays the file's, so the session goes on as a replay of the file would.
    fn opened_palette(&mut self, palette: PaletteState) {
        self.state.palette.open(palette);
        self.state.file.palette_steps = self.state.palette.steps;
        let now = &self.state.palette.now;
        if let Some(Err(e)) = self.state.render.with(|r| r.materials(now)) {
            self.state.sculpt.problem = Some(format!("cannot draw the file's materials: {e}"));
        }
    }

    /// Installs `doc` as the session's document, the one place a document is replaced (`file.open`
    /// and `file.recover` both land here), and resets what the session kept for the old one: the
    /// history starts empty and the gizmos forget their poses, any drag and its queued events,
    /// since a stable id in the new scene names another part (review 0008, M2). Isolate and the
    /// active part are the document's own, set from the file as it loads (decision 0033).
    fn install(&mut self, doc: Document) {
        let old_doc = std::mem::replace(&mut self.state.doc, doc);
        let budget = self.config.history_budget_bytes;
        let old_history = std::mem::replace(&mut self.state.history, History::new(budget));
        self.state.gizmos = Gizmos::default();
        // Freeing a big scene and its history takes a while; the engine thread goes on.
        let dropping = std::thread::Builder::new().name("gs-free".into());
        dropping.spawn(move || drop((old_doc, old_history))).ok();
    }

    /// Replays log lines after a recovery's open, one call at a time, each logged as it runs,
    /// and stops at the first that fails. Adds what happened to the recovery's result.
    pub(crate) fn replay(&mut self, lines: Vec<LogRecord>, success: &mut Success) {
        if lines.is_empty() {
            return;
        }
        let origin = Origin {
            client_id: 0,
            client: "recovery".into(),
            role: Role::Human,
        };
        let total = lines.len();
        let mut ran = 0;
        let mut stopped = None;
        for (i, line) in lines.into_iter().enumerate() {
            let failure = match line.request() {
                Ok(request) => match self.execute(&origin, request).outcome {
                    Outcome::Ok(_) => None,
                    Outcome::Error(e) => Some(e),
                },
                Err(e) => Some(errors::args_invalid("", e)),
            };
            if let Some(e) = failure {
                stopped = Some(format!("line {} ({}): {}", i + 1, line.command, e.message));
                break;
            }
            ran += 1;
        }
        if let Value::Object(result) = &mut success.result {
            result.insert("replayed".into(), json!(ran));
            result.insert("replay_stopped".into(), json!(stopped));
        }
        if let Some(why) = stopped {
            success.warnings.push(format!(
                "the replay stopped after {ran} of {total} lines, keeping the scene as it was \
                 there: {why}"
            ));
        }
    }
}
