//! What a running command can see and touch.

use std::collections::VecDeque;

use gs_core::transform::Similarity;
use gs_protocol::{Catalog, CommandError, Length, Role};

use crate::display::DisplaySettings;
use crate::document::DocTxn;
use crate::file::{FileState, Landing};
use crate::history::HistoryRef;
use crate::job::JobCell;
use crate::log::LogLine;
use crate::mesh::resolve_length;
use crate::sculpt::Sculpting;
use crate::ui::{RenderHost, Shared, WindowHost};
use crate::view::Views;
use crate::{CommandRng, EngineConfig};

/// Who sent a command.
///
/// Socket clients get `Agent` or `Observer` from `engine.hello`; only code inside the engine
/// process (the window, and tests that stand in for it) can make a `Human` origin.
#[derive(Clone, Debug)]
pub struct Origin {
    /// Connection id; edits are counted per id for `concurrent_changes`.
    pub client_id: u64,
    /// Client name, for example `gs` or `claude-code`. Steps are attributed to it.
    pub client: String,
    /// What the client may do.
    pub role: Role,
}

impl Origin {
    /// How the log and the history name this client: `name#connection`.
    pub fn label(&self) -> String {
        format!("{}#{}", self.client, self.client_id)
    }
}

/// Session switches that decide whether agents may edit.
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent session switches, each read by the policy on its own"
)]
pub(crate) struct Control {
    /// A human paused agents.
    pub paused: bool,
    /// A human gesture (pen stroke, slider drag) is in progress.
    pub gesture: bool,
    /// The title of the job holding the scene, while one does: the window's long edit, or a
    /// scene loading to replace this one (decisions 0010 and 0016). Edits and session commands
    /// wait, from every client.
    pub held: Option<&'static str>,
    /// The human lets agents move `main` while the window shows it (decision 0017).
    pub agents_move_view: bool,
    /// Set on a job's copy only: the session held a pen stroke or a gizmo drag when the copy
    /// was taken, which the copy leaves behind (see [`Ctx::settled`]).
    pub unfinished: bool,
}

/// True while the document holds a gesture still in progress: a pen stroke, a gizmo drag or
/// an open deformer applied in place and not yet committed.
pub(crate) fn unfinished(sculpt: &Sculpting, gizmos: &crate::gizmo::Gizmos) -> bool {
    sculpt.live.is_some() || gizmos.unsettled()
}

/// The context of one running command. (`'d` is the document transaction, which can outlive
/// one command in a batch; `'a` is everything else.)
pub struct Ctx<'a, 'd> {
    /// The document, behind write guards that record undo automatically.
    pub doc: DocTxn<'d>,
    /// The session's history, or a job's snapshot of it.
    pub(crate) history: HistoryRef<'a>,
    pub(crate) control: &'a mut Control,
    pub(crate) catalog: &'a Catalog,
    pub(crate) config: &'a EngineConfig,
    pub(crate) origin: &'a Origin,
    /// The command log's last lines, oldest first.
    pub(crate) log: &'a VecDeque<LogLine>,
    /// The command log's run (counting from 1) and lines in it when the call began: where a
    /// save of this state sits in the log, for recovery.
    pub(crate) log_at: (u64, u64),
    /// The scene's file, as the session knows it.
    pub(crate) file: &'a FileState,
    /// What the command hands the engine thread to apply when the call commits.
    pub(crate) landing: Option<Landing>,
    pub(crate) seq: u64,
    pub(crate) rev: u64,
    /// Clients the engine is tracking (connected and introduced, or that ran something).
    pub(crate) clients: u64,
    /// Set by session commands that replace document parts outside the write guards (undo,
    /// redo), so the engine bumps the revision.
    pub(crate) doc_replaced: bool,
    /// Why the command expects to change nothing, if it said so.
    pub(crate) noop_reason: Option<String>,
    /// Sentences the command added to the call's warnings (see `Ctx::warn`).
    pub(crate) warnings: Vec<String>,
    /// Set by a View command that is part of a gesture: the gesture is logged once, as the
    /// camera it ended on, instead of every call.
    pub(crate) in_gesture: bool,
    /// The window this engine runs in, or `NoWindow`. Shared with jobs.
    pub(crate) window: &'a Shared<dyn WindowHost>,
    /// What renders offscreen looks, or `NoRenderer`. Shared with jobs.
    pub(crate) render: &'a Shared<dyn RenderHost>,
    /// The job this command runs in, if it runs as one.
    pub(crate) job: Option<&'a JobCell>,
    /// Brush settings and the window's pen stroke.
    pub(crate) sculpt: &'a mut Sculpting,
    /// The session's views and their cameras.
    pub(crate) views: &'a mut Views,
    /// How the window and the looks draw (decision 0023).
    pub(crate) display: &'a mut DisplaySettings,
    /// Each part's gizmo and the drag in progress (decision 0027).
    pub(crate) gizmos: &'a mut crate::gizmo::Gizmos,
    /// The material palette and its own history (decision 0043).
    pub(crate) palette: &'a mut crate::material::PaletteState,
    /// Live Boolean's held results (decision 0057, section 8).
    pub(crate) live: &'a crate::boolean::Board,
}

impl Ctx<'_, '_> {
    /// This command's random numbers, derived from the session seed and its sequence number.
    pub fn rng(&self) -> CommandRng {
        CommandRng::for_command(self.config.seed, self.seq)
    }

    /// Says why this command will leave the scene as it was, for example "the distance is 0".
    /// Reported as the effect's `noop_reason` if nothing changed; ignored if something did.
    pub fn noop_reason(&mut self, why: impl Into<String>) {
        self.noop_reason = Some(why.into());
    }

    /// Adds a sentence to the call's `warnings`, whether or not anything changes: something the
    /// caller should know, such as a repair a boolean made. A job's answer carries it too.
    pub fn warn(&mut self, sentence: impl Into<String>) {
        self.warnings.push(sentence.into());
    }

    /// Reports how far a long command got (0 to 1, or `None` when unknown) and what it is doing
    /// now. `job.status` shows it; outside a job it goes nowhere.
    pub fn progress(&self, done: Option<f32>, message: impl Into<String>) {
        if let Some(job) = self.job {
            job.progress(done, message.into());
        }
    }

    /// Hands the engine thread `landing`, applied when the call commits (see
    /// `Command::BACKGROUND`).
    pub(crate) fn land(&mut self, landing: Landing) {
        self.landing = Some(landing);
    }

    /// Refuses, for a moment, a command that writes the document out while a pen stroke or a
    /// gizmo drag is still in progress: the file would hold half a gesture (decision 0041,
    /// section 4). Every command that writes the document to a file asks first.
    pub(crate) fn settled(&self) -> Result<(), CommandError> {
        if !(self.control.unfinished || unfinished(self.sculpt, self.gizmos)) {
            return Ok(());
        }
        let message = "a pen stroke or gizmo drag is still in progress";
        Err(CommandError {
            hint: Some("the file would hold half of it; retry once the pen lifts".into()),
            retryable: true,
            retry_after_ms: Some(200),
            ..crate::errors::error(gs_protocol::ErrorCode::Busy, message)
        })
    }

    /// True once the job running this command was cancelled. A long command that checks it
    /// between pieces of work can stop early; its result would be thrown away anyway.
    pub fn cancelled(&self) -> bool {
        self.job.is_some_and(JobCell::cancelled)
    }

    /// A command's length argument for part `name`, a percentage of the part's scene
    /// bounding-box diagonal. A command whose length meets a part takes it from here, or from
    /// [`PartLength::new`] when its percentage is of another shape, so none misses the part's
    /// scale (review 0008, m1).
    pub(crate) fn length(&self, name: &str, length: Length) -> Result<PartLength, CommandError> {
        self.doc.get_part(name).map(|part| part.length(length))
    }
}

/// A length argument for one part, in both spaces (decision 0029, section 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PartLength {
    /// Scene units: as given, or the percentage of the scene diagonal it is of. For what a
    /// command reports or checks.
    pub scene: f32,
    /// The part's own units: `scene` divided by the part's scale. For work on its points.
    pub part: f32,
}

impl PartLength {
    /// `length` for a part placed by `t`, a percentage taken of `diagonal`, a bounding-box
    /// diagonal in the scene, which is only asked for then.
    pub(crate) fn new(length: Length, t: Similarity, diagonal: impl FnOnce() -> f32) -> Self {
        let scene = match length {
            Length::Units(units) => units,
            Length::Percent(_) => resolve_length(length, diagonal()),
        };
        Self {
            scene,
            part: t.part_length(scene),
        }
    }
}
