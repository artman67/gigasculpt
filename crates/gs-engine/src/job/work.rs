//! A job's work: the copy of the session it takes when admitted, the run on its own thread,
//! and what it hands back.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use gs_protocol::CommandError;

use super::JobCell;
use crate::EngineConfig;
use crate::command::CommandSpec;
use crate::ctx::Origin;
use crate::document::{Document, PartChange, Reads};
use crate::engine::{Call, Env, Histories, Ran, Session, State, run_steps};
use crate::errors;
use crate::history::{HistoryRef, Snapshot};
use crate::log::LogLine;
use crate::registry::Registry;
use crate::sculpt::Sculpting;
use crate::view::own_view;

/// The session as a job sees it (decision 0010): the document shared part by part, the
/// history's listing and checkpoints as they were, the switches and brush settings copied, and
/// the session's own window and renderer.
pub(crate) type JobState = Session<Snapshot>;

impl Histories for Snapshot {
    fn borrowed(&mut self) -> HistoryRef<'_> {
        HistoryRef::Job(self)
    }
}

impl JobState {
    /// A copy of `state` as it is now. Costs reference counts per part column, the parts'
    /// small derived data, and the history's listing.
    pub fn of(state: &State) -> Self {
        Self {
            doc: state.doc.share(),
            history: state.history.snapshot(&state.doc),
            file: state.file.clone(),
            control: state.control,
            window: state.window.clone(),
            render: state.render.clone(),
            sculpt: Sculpting {
                settings: state.sculpt.settings.clone(),
                recorded: state.sculpt.recorded.clone(),
                ..Sculpting::default()
            },
            views: state.views.clone(),
            display: state.display.clone(),
            gizmos: state.gizmos.poses_only(),
            palette: state.palette.without_history(),
            live: state.live.clone(),
        }
    }
}

/// Everything a job needs, taken on the engine thread when the call was admitted.
pub(crate) struct Work {
    pub registry: Arc<Registry>,
    pub config: EngineConfig,
    pub origin: Origin,
    /// The command log's last lines when the call was admitted.
    pub log: VecDeque<LogLine>,
    /// The log's run and lines in it when the call was admitted.
    pub log_at: (u64, u64),
    pub call: Call,
    pub specs: Vec<&'static CommandSpec>,
    pub state: JobState,
    /// Sequence number of the first step.
    pub first: u64,
    /// Document revision when the call was admitted.
    pub rev: u64,
    pub clients: u64,
    pub concurrent_changes: u64,
    pub cell: Arc<JobCell>,
}

/// A job's work, done: what the engine thread needs to commit it.
pub(crate) struct Done {
    pub origin: Origin,
    pub call: Call,
    pub specs: Vec<&'static CommandSpec>,
    pub first: u64,
    pub cell: Arc<JobCell>,
    pub concurrent_changes: u64,
    pub result: Result<Output, CommandError>,
}

/// The steps that ran, what they changed and read, and the job's copy of the document after
/// them, which holds every part they changed as it is now and every other part as it was.
pub(crate) struct Output {
    pub ran: Vec<Ran>,
    pub changes: Vec<PartChange>,
    pub reads: Reads,
    pub doc: Document,
    /// The point the steps set as the caller's last edit, if they set one. The job's views
    /// are a copy, so the commit hands it to the caller's view (review 0003 m2).
    pub last_edit: Option<[f32; 3]>,
}

impl Work {
    /// The job this work belongs to.
    pub fn cell(&self) -> Arc<JobCell> {
        Arc::clone(&self.cell)
    }

    /// Runs every step against the job's copy of the session, through the same step runner as
    /// any call. A panic outside a command (which the runner already catches) becomes
    /// `E_INTERNAL` too, so the job always ends.
    pub fn run(self) -> Done {
        let Self {
            registry,
            config,
            origin,
            log,
            log_at,
            call,
            specs,
            mut state,
            first,
            rev,
            clients,
            concurrent_changes,
            cell,
        } = self;
        let env = Env {
            registry: &registry,
            config: &config,
            origin: &origin,
            log: &log,
            log_at,
            rev,
            clients,
            job: Some(&cell),
        };
        let own = own_view(&origin);
        let run = || -> Result<Output, CommandError> {
            let mut seq = first;
            let before = state.views.get(&own).last_edit;
            let scope = state.scope();
            let done = run_steps(scope, &env, &mut seq, &call, &specs).map_err(|e| e.1)?;
            let after = state.views.get(&own).last_edit;
            Ok(Output {
                ran: done.ran,
                changes: done.changes,
                reads: done.reads,
                doc: std::mem::take(&mut state.doc),
                last_edit: after.filter(|_| after != before),
            })
        };
        let result = catch_unwind(AssertUnwindSafe(run))
            .unwrap_or_else(|_| Err(errors::internal("the job's thread panicked")));
        Done {
            origin,
            call,
            specs,
            first,
            cell,
            concurrent_changes,
            result,
        }
    }
}
