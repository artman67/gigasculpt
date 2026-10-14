//! Jobs: calls with a long command (`Cost::Long`), run on their own thread so the session keeps
//! answering while they work (ARCHITECTURE 7.4 and 8, decision 0010).
//!
//! From the window or the socket, a call whose steps include a long edit or query becomes a job.
//! On the engine thread, the call is admitted as any call is (lookup, session policy, expected
//! revision), gets its sequence numbers, and takes a copy of the session: the document shared
//! part by part, the history's listing and checkpoints, the switches and brush settings, and
//! the session's window and renderer (`work`). The job's thread runs every step against that
//! copy through the same step runner every call uses, reporting progress as it goes. When it
//! finishes, the engine thread commits its result as one step, exactly as if the call had run
//! there, if every part it read or wrote is still as it was; otherwise the job fails with
//! `E_STALE` naming the part, and nothing applies (`commit`). A commit the session policy blocks
//! right then (agents paused, or a human mid-gesture) waits, and is tried again after every
//! call.
//!
//! A job keeps running when its client leaves. `job.status` finds it by id, waits for it, or
//! cancels it; a cancelled job ends at once with `E_CANCELLED` and never commits, even while
//! its thread is still finishing. A finished job keeps its full response until it is delivered,
//! and at most the [`KEPT_RESULTS`] newest in any case; then a summary (`cell`). In-process
//! callers (`Engine::execute`, tests) run other calls at once, long or not.
//!
//! A call with a background command (saves and opens, decision 0016) is a job for every
//! caller: an in-process caller runs it on its own thread, start to landing (`run_inline`).
//! The window gets its job's id at once, watches it with a bar and hears the end through its
//! call report; while it runs, a second long call from the window gets `E_BUSY` (decision 0010
//! as amended 2026-10-13). While the window's job edits, or a job opens a scene, the job holds
//! the scene: edits and session commands from everyone get `E_BUSY` naming it (`busy`).

mod cell;
mod commit;
mod work;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gs_protocol::{CmdResponse, CommandError, Cost, ErrorCode, Kind, Request, Role, Success};
use serde_json::json;

pub(crate) use cell::JobCell;
pub(crate) use work::{Done, Work};

use crate::command::CommandSpec;
use crate::ctx::{Origin, unfinished};
use crate::engine::{Call, Engine};
use crate::errors::{self, error};
use work::JobState;

/// Most jobs whose threads may work at once. Each holds a copy of the session.
pub(crate) const MAX_WORKING: usize = 4;

/// Most finished jobs that keep their full response before it is delivered.
pub(crate) const KEPT_RESULTS: usize = 16;

/// A session's jobs, by id. Ended jobs stay, so `job.status` can answer for them later.
#[derive(Default)]
pub(crate) struct Jobs {
    cells: BTreeMap<u64, Arc<JobCell>>,
    last: u64,
    /// The window's last job, which its bar shows until it ends.
    window: Option<Arc<JobCell>>,
    /// The job holding the scene, while one does (see `Control::held`).
    holding: Option<u64>,
}

impl Jobs {
    /// A new job running `command`, which a person calls `title`, for the client named `owner`,
    /// or `E_LIMIT` while too many jobs work.
    pub fn add(
        &mut self,
        command: &str,
        title: &'static str,
        owner: &str,
    ) -> Result<Arc<JobCell>, CommandError> {
        let working = self.cells.values().filter(|c| c.working()).count();
        if working >= MAX_WORKING {
            return Err(CommandError {
                hint: Some("wait for one with job.status, or cancel one".into()),
                retryable: true,
                ..error(
                    ErrorCode::Limit,
                    format!("{working} jobs are running; at most {MAX_WORKING} run at once"),
                )
            });
        }
        self.last += 1;
        let cell = JobCell::new(self.last, command.to_string(), title, owner.to_string());
        let cell = Arc::new(cell);
        self.cells.insert(self.last, Arc::clone(&cell));
        Ok(cell)
    }

    /// Job `id`, or `E_NOT_FOUND` naming the jobs there are.
    pub fn get(&self, id: u64) -> Result<Arc<JobCell>, CommandError> {
        self.cells.get(&id).cloned().ok_or_else(|| {
            let known: Vec<String> = self.cells.keys().map(u64::to_string).collect();
            CommandError {
                suggestions: known,
                hint: Some("use the id job.start gave".into()),
                ..error(ErrorCode::NotFound, format!("there is no job {id}"))
            }
        })
    }

    /// Every job. (Ending one that already ended changes nothing.)
    pub fn all(&self) -> impl Iterator<Item = &Arc<JobCell>> {
        self.cells.values()
    }

    /// Keeps the full response of at most the [`KEPT_RESULTS`] newest ended jobs; older ones
    /// keep a summary (review 0002 M3).
    pub fn release(&self) {
        let ended = self.cells.values().rev().filter(|c| c.holds_result());
        for cell in ended.skip(KEPT_RESULTS) {
            cell.release();
        }
    }
}

/// One of the window's jobs as its status line's bar shows it (decision 0010 as amended
/// 2026-10-13): reading it never waits on the engine thread and never takes the result.
#[derive(Clone)]
pub struct JobWatch(Arc<JobCell>);

impl JobWatch {
    pub(crate) fn new(cell: Arc<JobCell>) -> Self {
        Self(cell)
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// What a person calls it: its command's title ("Quad remesh").
    pub fn title(&self) -> &'static str {
        self.0.title
    }

    /// How far it got (0 to 1) where its command says, and what it is doing now.
    pub fn progress(&self) -> (Option<f32>, Option<String>) {
        self.0.peek()
    }

    /// True once it ended: committed, failed or cancelled.
    pub fn ended(&self) -> bool {
        self.0.ended()
    }

    /// True while its thread still works, even once it ended: a cancelled job ends at once
    /// and its thread stops when its command next looks.
    pub fn working(&self) -> bool {
        self.0.working()
    }
}

impl std::fmt::Debug for JobWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job {} ({})", self.0.id, self.0.title)
    }
}

/// What a person calls a job running `specs`: its first long or background command's title,
/// or its name where it has none.
fn title(specs: &[&'static CommandSpec]) -> &'static str {
    let first = (specs.iter()).find(|s| s.background || s.cost == Cost::Long);
    let spec = first.or(specs.first());
    spec.map_or(
        "A job",
        |s| if s.title.is_empty() { s.name } else { s.title },
    )
}

/// The refusal while job `title` holds the scene, or while the window's job runs.
pub(crate) fn busy(title: &str) -> CommandError {
    CommandError {
        hint: Some("queries and the view still work; retry this once it ends".into()),
        retryable: true,
        retry_after_ms: Some(500),
        ..error(
            ErrorCode::Busy,
            format!("{title} is running; wait for it to end or cancel it"),
        )
    }
}

/// Why a job's work was not applied: a part it read or wrote changed while it ran.
pub(crate) fn stale(command: &str, why: &str) -> CommandError {
    CommandError {
        hint: Some("look at the document again, then run it again".into()),
        retryable: true,
        ..error(
            ErrorCode::Stale,
            format!("{why} while {command} ran, so nothing was applied"),
        )
    }
}

/// Who runs a call, which decides what becomes a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
    /// A socket client: every long call is a job, so the session keeps answering.
    Socket,
    /// The window: every long call is a job too, one at a time, shown by its bar.
    Window,
    /// Code holding the engine itself (tests, examples, autosave): background calls run as
    /// jobs; the rest at once.
    InProcess,
}

/// What [`Engine::start`] made of a call.
#[expect(
    clippy::large_enum_variant,
    reason = "made once per call and taken apart at once; boxing the answer costs an allocation"
)]
pub(crate) enum Start {
    /// It ran, or was refused, at once.
    Now(CmdResponse),
    /// It is a job: run the work on its own thread, then hand the result to
    /// [`Engine::finish`].
    Job(Box<Work>),
}

impl Engine {
    /// Like [`Engine::execute`], except that a call with a long edit or query becomes a job:
    /// it is admitted here and gets its sequence numbers and a copy of the session, and the
    /// work comes back to run on another thread.
    pub(crate) fn start(&mut self, origin: &Origin, request: Request, caller: Caller) -> Start {
        self.autosave.activity(self.clock.now(), origin);
        let name = request.name().to_string();
        let call = Call::from(request);
        if !self.runs_as_job(&call, caller) {
            return Start::Now(self.run_now(origin, &call));
        }
        let concurrent_changes = self.concurrent_changes(origin);
        let window = caller == Caller::Window;
        let admitted = self.admit(origin, &call).and_then(|specs| {
            let running = self.jobs.window.as_ref().filter(|c| window && !c.ended());
            if let Some(running) = running {
                return Err(busy(running.title));
            }
            let cell = self.jobs.add(&name, title(&specs), &origin.client)?;
            Ok((specs, cell))
        });
        let (specs, cell) = match admitted {
            Ok(admitted) => admitted,
            Err(e) => {
                let reply = Err((None, e));
                return Start::Now(self.respond(origin, &call, concurrent_changes, reply));
            }
        };
        self.end_live_before(&specs);
        self.make_views(origin, &call, &specs);
        self.log_ended_gestures();
        self.hold(window, &specs, &cell);
        let first = self.next_seq;
        self.next_seq += call.steps.len() as u64;
        let mut state = JobState::of(&self.state);
        state.control.unfinished = unfinished(&self.state.sculpt, &self.state.gizmos);
        Start::Job(Box::new(Work {
            registry: Arc::clone(&self.registry),
            config: self.config.clone(),
            origin: origin.clone(),
            log: self.log.recent().clone(),
            log_at: self.log.position(),
            specs,
            state,
            first,
            rev: self.rev,
            clients: self.seen.len() as u64,
            concurrent_changes,
            cell,
            call,
        }))
    }

    /// Marks the window's new job, and holds the scene for it while it edits or opens a scene:
    /// edits and session commands from everyone get `E_BUSY` naming it until it ends.
    fn hold(&mut self, window: bool, specs: &[&'static CommandSpec], cell: &Arc<JobCell>) {
        if window {
            self.jobs.window = Some(Arc::clone(cell));
        }
        let opens = |s: &&CommandSpec| s.background && s.kind == Kind::Session;
        let edits = |s: &&CommandSpec| window && s.kind == Kind::Edit;
        if specs.iter().any(|s| opens(s) || edits(s)) {
            self.jobs.holding = Some(cell.id);
            self.state.control.held = Some(cell.title);
        }
    }

    /// True for a call that runs as a job: not a dry run, every step an edit, a query or a
    /// background command, and one of them a background command, or, from the window or a
    /// socket client, a long one.
    fn runs_as_job(&self, call: &Call, caller: Caller) -> bool {
        let specs: Vec<&CommandSpec> = (call.steps.iter())
            .filter_map(|step| self.registry.find(&step.command).ok())
            .collect();
        let background = specs.iter().any(|s| s.background);
        let long = specs.iter().any(|s| s.cost == Cost::Long);
        !call.dry_run
            && specs.len() == call.steps.len()
            && (specs.iter()).all(|s| s.background || matches!(s.kind, Kind::Edit | Kind::Query))
            && (background || (caller != Caller::InProcess && long))
    }

    /// Runs a job on this thread, start to landing, and gives its answer: how an in-process
    /// caller runs a background call.
    pub(crate) fn run_inline(&mut self, work: Work) -> CmdResponse {
        let cell = work.cell();
        if let Some(held) = self.finish(work.run()) {
            self.drop_job(&held.cell, "its commit was held back");
        }
        let report = cell.wait(Some(Duration::ZERO));
        report
            .response
            .unwrap_or_else(|| self.cancelled("the job ended without an answer".into()))
    }

    /// The answer the window gets at once for a call that became a job: the job's id. The end
    /// comes through the window's call report.
    pub(crate) fn started(&self, work: &Work) -> CmdResponse {
        let (id, command) = (work.cell.id, &work.cell.command);
        let started = Success {
            result: json!({ "job": id, "command": command, "working": true }),
            effect: None,
            history_id: None,
            warnings: vec![format!("{command} runs in the background as job {id}")],
        };
        self.answer(Ok((Some(work.first), started)), 0)
    }

    /// True while job `id` holds the scene.
    pub(crate) fn holds(&self, id: u64) -> bool {
        self.jobs.holding == Some(id)
    }

    /// Job `id` ended, one way or another: if it held the scene, edits may run again.
    pub(crate) fn released(&mut self, id: u64) {
        if self.holds(id) {
            self.jobs.holding = None;
            self.state.control.held = None;
        }
    }

    /// Job `id`, for `job.status`; cancelled first with `cancel`, which only the client that
    /// started it (by name) or the human may do (review 0002 m4).
    pub(crate) fn job(
        &mut self,
        id: u64,
        cancel: bool,
        origin: &Origin,
    ) -> Result<Arc<JobCell>, CommandError> {
        let cell = self.jobs.get(id)?;
        let theirs = origin.role == Role::Human
            || (origin.role == Role::Agent && origin.client == cell.owner);
        if cancel && !theirs {
            let reason = crate::Unavailable::new(format!(
                "job {id} was started by {}; only that client or the human can cancel it",
                cell.owner
            ));
            return Err(errors::unavailable(
                reason.fix("ask its client, or wait for it with job.status"),
            ));
        }
        if cancel {
            let why = format!("job {id} was cancelled; nothing changed");
            cell.cancel(self.cancelled(why));
            self.released(id);
        }
        Ok(cell)
    }

    /// Ends every job that has not ended, as the session stops.
    pub(crate) fn drop_jobs(&mut self) {
        let cells: Vec<Arc<JobCell>> = self.jobs.all().cloned().collect();
        for cell in cells {
            self.drop_job(&cell, "the session stopped before it ended");
        }
    }

    /// Ends a job whose thread will never hand its work back.
    pub(crate) fn drop_job(&mut self, cell: &JobCell, why: &str) {
        cell.returned();
        cell.cancel(self.cancelled(format!("job {}: {why}", cell.id)));
        self.released(cell.id);
    }

    /// The answer a cancelled job ends with.
    fn cancelled(&self, why: String) -> CmdResponse {
        let err = CommandError {
            hint: Some("start it again to redo it".into()),
            ..error(ErrorCode::Cancelled, why)
        };
        self.answer(Err((None, err)), 0)
    }
}
