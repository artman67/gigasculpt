//! The executor's step pipeline: the only way anything changes. See report 04, section 4.2.
//!
//! A single command (`cmd.run`) is a call of one step and a batch (`cmd.batch`) a call of
//! several. Both go through the same pipeline, so no rule can apply to one and not the other:
//!
//! 1. Admit: look up every command (a batch takes only edit and query commands), then apply the
//!    session policy (roles, pause, human gestures) to every step, then the expected revision.
//! 2. Run every step in order inside one transaction: pin context-dependent arguments, validate
//!    them (reporting every violation), check, run. An error or panic in any step rolls the
//!    whole call back. A step that is not an edit may not write parts.
//! 3. Commit: at most one undo step for the whole call (none, with a reason, when nothing
//!    changed), its effect, and one log line with the resolved arguments, so one log line is
//!    one call, exactly as history sees it.
//!
//! A call with a long command can run step 2 on another thread, against a shared copy of the
//! session, as a job (see `job`); steps 1 and 3 always run here. Which revision each client saw,
//! the window and the window's pen strokes are the session's bookkeeping (`engine::session`).

mod session;

pub(crate) use session::Adopted;

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use gs_protocol::{
    Catalog, CmdResponse, CommandError, ErrorCode, Kind, LOG_FORMAT, LogHeader, LogRecord, Outcome,
    PROTOCOL_VERSION, Request, Role, Step, Success, method,
};
use serde_json::{Value, json};

use crate::boolean::{Board, Live};
use crate::clock::{Clock, SystemClock};
use crate::command::{CommandSpec, ErasedCommand};
use crate::ctx::{Control, Ctx, Origin};
use crate::display::DisplaySettings;
use crate::document::{DocTxn, Document, PartChange, Reads};
use crate::effect::effect_of;
use crate::errors::{self, error};
use crate::file::{Autosave, FileState, FreeSpace, Landing, StateFolder, marked_logs};
use crate::gizmo::Gizmos;
use crate::history::{Entry, History, HistoryRef};
use crate::job::{Caller, JobCell, Jobs, Start};
use crate::log::{CommandLog, LogLine, StandardLog};
use crate::material::PaletteState;
use crate::mesh::document_digest;
use crate::pinning::pin;
use crate::registry::Registry;
use crate::sculpt::{Sculpting, carry_presets};
use crate::ui::{NoRenderer, NoWindow, RenderHost, Shared, WindowHost};
use crate::view::Views;

/// How to start an engine.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Session name.
    pub session: String,
    /// Seed of every random number in the session.
    pub seed: u64,
    /// Undo history byte budget. The newest step is always kept.
    pub history_budget_bytes: u64,
    /// Where to write the command log, if anywhere.
    pub log_path: Option<PathBuf>,
    /// Set when the log is the standard one (`with_log`): its folder is pruned when the session
    /// starts. Never set for a log someone chose, whose folder may hold other files.
    pub standard_log: Option<StandardLog>,
    /// Write the scene hash after every logged call, so a replay can name the first call that
    /// diverges. Costs one pass over the scene per call.
    pub verify: bool,
    /// The state folder: autosave, the crash marker and the recent files list (decision 0016,
    /// section 10). Off by default, so tests and examples never write into the home folder; the
    /// window and `serve` turn it on.
    pub autosave: Option<StateFolder>,
    /// Where the free space the low disk warning and the save's check read comes from: the
    /// file system, unless a test fixes it (decision 0016, section 17, item 9).
    pub free_space: FreeSpace,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            session: "main".into(),
            seed: 0,
            history_budget_bytes: 2 << 30,
            log_path: None,
            standard_log: None,
            verify: false,
            autosave: None,
            free_space: FreeSpace::default(),
        }
    }
}

impl EngineConfig {
    /// The command log: `chosen` when given, used as it is; else the standard log in the state
    /// folder (`gs_protocol::state_dir`). The one rule `gigasculpt serve` and the window share.
    pub fn with_log(self, chosen: Option<PathBuf>) -> Result<Self, String> {
        match chosen {
            Some(path) => Ok(Self {
                log_path: Some(path),
                standard_log: None,
                ..self
            }),
            None => Ok(self.with_standard_log(gs_protocol::state_dir()?)),
        }
    }

    /// The standard log in state folder `state`: `<state>/logs/<session>-<pid>.jsonl`, with that
    /// folder pruned at start (decision 0016, section 14).
    #[must_use]
    pub fn with_standard_log(self, state: PathBuf) -> Self {
        let standard = StandardLog::new(state);
        Self {
            log_path: Some(standard.path(&self.session)),
            standard_log: Some(standard),
            ..self
        }
    }
}

/// What running commands may change: the session's own (`State`), or a job's copy of it, whose
/// history `H` is a read-only snapshot (`job::JobState`).
pub(crate) struct Session<H> {
    pub doc: Document,
    pub history: H,
    /// The scene's file: its path, the revision it holds, the scene id.
    pub file: FileState,
    pub control: Control,
    pub window: Shared<dyn WindowHost>,
    pub render: Shared<dyn RenderHost>,
    pub sculpt: Sculpting,
    pub views: Views,
    /// How the window and the looks draw (decision 0023).
    pub display: DisplaySettings,
    /// Each part's gizmo and the drag in progress (decision 0027).
    pub gizmos: Gizmos,
    /// The material palette and its own history (decision 0043).
    pub palette: PaletteState,
    /// Live Boolean's held runs and results (decision 0057, section 8), which commands read.
    pub live: Board,
}

/// The parts of the engine a running command may change.
pub(crate) type State = Session<History>;

/// A history a call can run against: the session's, or a job's snapshot.
pub(crate) trait Histories {
    fn borrowed(&mut self) -> HistoryRef<'_>;
}

impl Histories for History {
    fn borrowed(&mut self) -> HistoryRef<'_> {
        HistoryRef::Session(self)
    }
}

impl<H: Histories> Session<H> {
    /// What a call's steps work on.
    pub fn scope(&mut self) -> Scope<'_> {
        Scope {
            doc: &mut self.doc,
            history: self.history.borrowed(),
            file: &self.file,
            control: &mut self.control,
            window: &self.window,
            render: &self.render,
            sculpt: &mut self.sculpt,
            views: &mut self.views,
            display: &mut self.display,
            gizmos: &mut self.gizmos,
            palette: &mut self.palette,
            live: &self.live,
        }
    }
}

/// What a call's steps run against: the session's state, or a job's copy of it.
pub(crate) struct Scope<'s> {
    pub doc: &'s mut Document,
    pub history: HistoryRef<'s>,
    pub file: &'s FileState,
    pub control: &'s mut Control,
    pub window: &'s Shared<dyn WindowHost>,
    pub render: &'s Shared<dyn RenderHost>,
    pub sculpt: &'s mut Sculpting,
    pub views: &'s mut Views,
    pub display: &'s mut DisplaySettings,
    pub gizmos: &'s mut Gizmos,
    pub palette: &'s mut PaletteState,
    pub live: &'s Board,
}

/// What a call's steps read besides the state they change.
pub(crate) struct Env<'a> {
    pub registry: &'a Registry,
    pub config: &'a EngineConfig,
    pub origin: &'a Origin,
    /// The command log's last lines.
    pub log: &'a VecDeque<LogLine>,
    /// The log's run and lines in it when the call started.
    pub log_at: (u64, u64),
    /// Document revision when the call started.
    pub rev: u64,
    pub clients: u64,
    /// The job the call runs in, if it runs as one.
    pub job: Option<&'a JobCell>,
}

/// One engine: a document, its history and its log. Single-threaded; the socket server runs it
/// on one thread and queues every client's requests.
pub struct Engine {
    pub(crate) registry: Arc<Registry>,
    pub(crate) config: EngineConfig,
    pub(crate) state: State,
    pub(crate) log: CommandLog,
    pub(crate) next_seq: u64,
    pub(crate) rev: u64,
    /// The revision each client last saw, for `concurrent_changes`.
    pub(crate) seen: BTreeMap<u64, u64>,
    pub(crate) jobs: Jobs,
    /// Reads the time, for autosave only.
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) autosave: Autosave,
    /// Live Boolean's imports and the runs it waits on (decision 0057, section 8).
    pub(crate) live: Live,
}

/// A failure, with the sequence number if the call got as far as running.
pub(crate) type Failure = (Option<u64>, CommandError);
pub(crate) type Reply = Result<(Option<u64>, Success), Failure>;

/// One call as the pipeline sees it.
pub(crate) struct Call {
    pub steps: Vec<Step>,
    pub if_rev: Option<u64>,
    pub dry_run: bool,
    pub batch: bool,
}

impl From<Request> for Call {
    fn from(request: Request) -> Self {
        match request {
            Request::Run(params) => Self {
                steps: vec![Step {
                    command: params.command,
                    args: params.args,
                }],
                if_rev: params.if_rev,
                dry_run: params.dry_run,
                batch: false,
            },
            Request::Batch(params) => Self {
                steps: params.commands,
                if_rev: params.if_rev,
                dry_run: false,
                batch: true,
            },
        }
    }
}

impl Call {
    /// Points an error from step `index` of a batch into `/commands/index`.
    fn at(&self, index: usize, err: CommandError) -> CommandError {
        if !self.batch {
            return err;
        }
        let path = format!("/commands/{index}{}", err.path.as_deref().unwrap_or(""));
        CommandError {
            path: Some(path),
            message: format!("step {index}: {}", err.message),
            ..err
        }
    }

    /// The command names it runs, in order.
    pub fn commands(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.command.clone()).collect()
    }
}

/// A step that ran, waiting to be committed.
pub(crate) struct Ran {
    pub spec: &'static CommandSpec,
    /// Resolved arguments.
    pub args: Value,
    pub output: Value,
    pub noop_reason: Option<String>,
    /// The sentences the step added to the warnings (see `Ctx::warn`).
    pub warnings: Vec<String>,
    /// What the step hands the engine thread (see `Ctx::land`).
    pub landing: Option<Landing>,
    /// The call is logged when one of its steps is: a logged command not inside a gesture.
    pub logged: bool,
}

/// What running a call's steps left to commit.
#[derive(Default)]
pub(crate) struct Done {
    pub ran: Vec<Ran>,
    pub changes: Vec<PartChange>,
    pub doc_replaced: bool,
    /// What the steps read of the document.
    pub reads: Reads,
    /// The command and arguments to log instead of the call's own (a recovery is logged as
    /// the open of its autosave).
    pub log_as: Option<(String, Value)>,
    /// The active part's id before the call, for its undo step (decision 0033).
    pub active: Option<u64>,
}

impl Engine {
    /// A new engine with an empty document. Fails only if the log cannot be started.
    pub fn new(config: EngineConfig) -> io::Result<Self> {
        let registry = shared_registry();
        let header = log_header(&registry, &config);
        if let Some(standard) = &config.standard_log {
            standard.prune(&marked_logs(&standard.state_dir));
            fs::create_dir_all(standard.folder())?;
        }
        let clock: Box<dyn Clock> = Box::new(SystemClock::new());
        let engine = Self {
            log: CommandLog::start(config.log_path.as_deref(), &header)?,
            registry,
            state: State {
                doc: Document::default(),
                history: History::new(config.history_budget_bytes),
                file: FileState::default(),
                control: Control::default(),
                window: Shared::new(Box::new(NoWindow::default())),
                render: Shared::new(Box::new(NoRenderer)),
                sculpt: Sculpting::default(),
                views: Views::default(),
                display: DisplaySettings::default(),
                gizmos: Gizmos::default(),
                palette: PaletteState::default(),
                live: Board::default(),
            },
            next_seq: 1,
            rev: 0,
            seen: BTreeMap::new(),
            jobs: Jobs::default(),
            autosave: Autosave::new(clock.now()),
            clock,
            config,
            live: Live::default(),
        };
        engine.mark_session();
        Ok(engine)
    }

    /// The header a new log run starts with.
    pub(crate) fn log_header(&self) -> LogHeader {
        log_header(&self.registry, &self.config)
    }

    /// The command catalog.
    pub fn catalog(&self) -> &Catalog {
        &self.registry.catalog
    }

    /// Runs a command or a batch for a client, to the end: a background command's job runs
    /// here, on the caller's thread (decision 0016).
    pub fn execute(&mut self, origin: &Origin, request: Request) -> CmdResponse {
        match self.start(origin, request, Caller::InProcess) {
            Start::Now(response) => response,
            Start::Job(work) => self.run_inline(*work),
        }
    }

    pub(crate) fn run_now(&mut self, origin: &Origin, call: &Call) -> CmdResponse {
        let concurrent_changes = self.concurrent_changes(origin);
        let reply = self.call(origin, call);
        self.seen.insert(origin.client_id, self.rev);
        self.respond(origin, call, concurrent_changes, reply)
    }

    /// Tells the window what a call did, and answers it.
    pub(crate) fn respond(
        &mut self,
        origin: &Origin,
        call: &Call,
        concurrent_changes: u64,
        reply: Reply,
    ) -> CmdResponse {
        self.look_live();
        self.report(origin, &call.commands(), reply.is_ok());
        let reply = self.bundle_panic(call, reply);
        self.answer(reply, concurrent_changes)
    }

    /// A call's answer at the session's revision. Every answer is built here: a call run here
    /// or as a job, a job's start, and a cancelled job.
    pub(crate) fn answer(&self, reply: Reply, concurrent_changes: u64) -> CmdResponse {
        let (seq, outcome) = match reply {
            Ok((seq, success)) => (seq, Outcome::Ok(success)),
            Err((seq, err)) => (seq, Outcome::Error(err)),
        };
        CmdResponse {
            seq,
            rev: self.rev,
            concurrent_changes,
            outcome,
        }
    }

    fn call(&mut self, origin: &Origin, call: &Call) -> Reply {
        let specs = self.admit(origin, call).map_err(|e| (None, e))?;
        self.end_live_before(&specs);
        if !call.dry_run {
            self.sync_views();
            self.make_views(origin, call, &specs);
        }
        self.log_ended_gestures();
        // The views are part of the call's transaction, as the document is: a call that fails,
        // or a dry run, leaves every view as it was, a gesture it would close included (review
        // 0003 M5). The closed gestures' lines go in once it succeeded, before its own line.
        let views = self.state.views.clone();
        let poses = self.state.gizmos.saved();
        let closed = self.end_gestures_before(origin, call, &specs);
        let lines = self.next_seq;
        self.next_seq += if call.dry_run { 0 } else { closed.len() as u64 };
        let first = self.next_seq;
        let env = Env {
            registry: &self.registry,
            config: &self.config,
            origin,
            log: self.log.recent(),
            log_at: self.log.position(),
            rev: self.rev,
            clients: self.seen.len() as u64,
            job: None,
        };
        let scope = self.state.scope();
        let done = match run_steps(scope, &env, &mut self.next_seq, call, &specs) {
            Ok(done) if !call.dry_run => done,
            other => {
                self.state.views = views;
                self.state.gizmos.put_back(poses);
                return other.map(|_| (None, dry_run()));
            }
        };
        self.log_view_lines(closed, lines);
        // Pen samples that sculpt reach the log in the stroke they make, not as pen input.
        let strokes = self.state.window.with(WindowHost::take_strokes);
        let success = self.commit(origin, call, first, done, strokes.is_empty());
        self.pen_strokes(origin, strokes);
        self.gizmo_drags();
        self.log_ended_gestures();
        self.follow_masks();
        Ok((Some(first), success))
    }

    /// Looks up every command, then applies the session policy to every step, then the
    /// expected revision. Nothing has run yet.
    pub(crate) fn admit(
        &self,
        origin: &Origin,
        call: &Call,
    ) -> Result<Vec<&'static CommandSpec>, CommandError> {
        if call.batch && call.steps.is_empty() {
            return Err(CommandError {
                hint: Some("list at least one {\"command\": ..., \"args\": ...}".into()),
                ..errors::args_invalid("/commands", "a batch needs at least one command")
            });
        }
        let mut specs = Vec::new();
        for (i, step) in call.steps.iter().enumerate() {
            let spec = self
                .registry
                .find(&step.command)
                .map_err(|e| call.at(i, e))?;
            if call.batch && !matches!(spec.kind, Kind::Edit | Kind::Query) {
                let e = errors::args_invalid(
                    "/command",
                    "only edit and query commands can run in a batch",
                );
                return Err(call.at(i, e));
            }
            specs.push(spec);
        }
        for (i, spec) in specs.iter().enumerate() {
            policy(self.state.control, origin, spec).map_err(|e| call.at(i, e))?;
        }
        match call.if_rev {
            Some(expected) if expected != self.rev => Err(CommandError {
                hint: Some("look at the document again, then retry with the new rev".into()),
                ..error(
                    ErrorCode::Stale,
                    format!("the document is at rev {}, not {expected}", self.rev),
                )
            }),
            _ => Ok(specs),
        }
    }

    /// Records a finished call: one log line if any step is logged, one history step if it
    /// changed anything, and its effect.
    pub(crate) fn commit(
        &mut self,
        origin: &Origin,
        call: &Call,
        first: u64,
        done: Done,
        may_log: bool,
    ) -> Success {
        let edits = call.batch || done.ran.iter().any(|r| r.spec.kind == Kind::Edit);
        let effect = edits.then(|| effect_of(&done.changes, noop_reason(call, &done.ran)));
        let mut warnings: Vec<String> = effect
            .iter()
            .filter_map(|e| e.noop_reason.clone())
            .collect();
        warnings.extend(warned(call, &done.ran));
        let logged = may_log && done.ran.iter().any(|r| r.logged);
        let (mut command, mut args, result) = summarize(call, done.ran);
        if let Some((as_command, as_args)) = done.log_as {
            (command, args) = (as_command, as_args);
        }
        // A line logged while a pen stroke or drag is open (a brush.set keyed mid-stroke) would
        // hash the half-made edit, which its replay never holds: it goes without a hash.
        let settled = !crate::ctx::unfinished(&self.state.sculpt, &self.state.gizmos);
        let hash = (logged && self.config.verify && settled)
            .then(|| document_digest(&self.state.doc).to_hex().to_string());
        let changed = !done.changes.is_empty();
        self.rev += u64::from(changed || done.doc_replaced);
        let record = LogRecord {
            seq: Some(first),
            client: Some(origin.label()),
            command,
            args,
            hash,
        };
        if logged {
            let line = LogLine {
                seq: first,
                rev: self.rev,
                client: origin.label(),
                role: origin.role,
                commands: call.commands(),
                status: "ok".into(),
                run: 0,
                record: "".into(),
            };
            let log = &mut self.log;
            let recorded = &self.state.sculpt.recorded;
            let carried = carry_presets(&record, recorded, |hash| log.first_use(hash));
            warnings.extend(log.append(carried.as_ref().unwrap_or(&record), line));
        }
        let history_id = changed.then(|| {
            let changes = done.changes.into_iter().map(|c| c.change).collect();
            let mut entry = Entry::new(origin, &record.command, record.args, changes);
            entry.active = (done.active, self.state.doc.active_id());
            self.state.history.push(entry, &mut self.state.doc)
        });
        Success {
            result,
            effect,
            history_id,
            warnings,
        }
    }
}

/// Runs every step in one transaction. Any failure rolls back every step. `seq` is the first
/// step's sequence number, and moves past every step that runs.
pub(crate) fn run_steps(
    scope: Scope,
    env: &Env,
    seq: &mut u64,
    call: &Call,
    specs: &[&'static CommandSpec],
) -> Result<Done, Failure> {
    let first = (!call.dry_run).then_some(*seq);
    let Scope {
        doc,
        mut history,
        file,
        control,
        window,
        render,
        sculpt,
        views,
        display,
        gizmos,
        palette,
        live,
    } = scope;
    let active = doc.active_id();
    let mut txn = DocTxn::new(doc);
    let mut done = Done {
        active,
        ..Done::default()
    };
    for (i, (spec, step)) in specs.iter().zip(&call.steps).enumerate() {
        let mut cx = Ctx {
            doc: txn,
            history: history.reborrow(),
            control: &mut *control,
            catalog: &env.registry.catalog,
            config: env.config,
            origin: env.origin,
            log: env.log,
            log_at: env.log_at,
            file,
            landing: None,
            seq: *seq,
            rev: env.rev,
            clients: env.clients,
            doc_replaced: false,
            noop_reason: None,
            warnings: Vec::new(),
            in_gesture: false,
            window,
            render,
            job: env.job,
            sculpt: &mut *sculpt,
            views: &mut *views,
            display: &mut *display,
            gizmos: &mut *gizmos,
            palette: &mut *palette,
            live,
        };
        *seq += u64::from(!call.dry_run);
        let result = run_step(env.registry, spec, step.args.clone(), &mut cx, call.dry_run);
        done.doc_replaced |= cx.doc_replaced;
        let logged = spec.logged && !cx.in_gesture;
        txn = cx.doc;
        match result {
            Ok(Some((args, output))) => done.ran.push(Ran {
                spec,
                args,
                output,
                noop_reason: cx.noop_reason,
                warnings: cx.warnings,
                landing: cx.landing,
                logged,
            }),
            Ok(None) => {}
            Err(e) => {
                txn.rollback();
                return Err((first, call.at(i, e)));
            }
        }
    }
    if call.dry_run {
        txn.rollback();
    } else {
        (done.changes, done.reads) = txn.finish().map_err(|e| (first, e))?;
    }
    Ok(done)
}

/// A call's name, resolved arguments and result: the command's own for a single command;
/// `cmd.batch` with every step, and the array of outputs, for a batch.
fn summarize(call: &Call, ran: Vec<Ran>) -> (String, Value, Value) {
    if !call.batch {
        if let Some(one) = ran.into_iter().next() {
            return (one.spec.name.to_string(), one.args, one.output);
        }
        return (String::new(), Value::Null, Value::Null);
    }
    let steps: Vec<Value> = ran
        .iter()
        .map(|r| json!({ "command": r.spec.name, "args": r.args }))
        .collect();
    let outputs = ran.into_iter().map(|r| r.output).collect();
    let args = json!({ "commands": steps });
    (method::BATCH.to_string(), args, Value::Array(outputs))
}

/// The reasons the steps gave for changing nothing.
fn noop_reason(call: &Call, ran: &[Ran]) -> Option<String> {
    let reasons: Vec<String> = ran
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let reason = r.noop_reason.as_ref()?;
            Some(if call.batch {
                format!("step {i}: {reason}")
            } else {
                reason.clone()
            })
        })
        .collect();
    (!reasons.is_empty()).then(|| format!("nothing changed: {}", reasons.join("; ")))
}

/// The sentences the steps added with `Ctx::warn`, each after its step's number in a batch.
fn warned(call: &Call, ran: &[Ran]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, r) in ran.iter().enumerate() {
        for w in &r.warnings {
            out.push(if call.batch {
                format!("step {i}: {w}")
            } else {
                w.clone()
            });
        }
    }
    out
}

/// Who may run what right now: some commands are for the human only; observers only query;
/// agents wait while a human paused them or is mid-gesture; the human is blocked only while a
/// job holds the scene, and then only from its steps (an unlogged switch such as Pause agents
/// is not one).
pub(crate) fn policy(
    control: Control,
    origin: &Origin,
    spec: &CommandSpec,
) -> Result<(), CommandError> {
    let human = origin.role == Role::Human;
    if let Some(job) = control.held
        && matches!(spec.kind, Kind::Edit | Kind::Session)
        && spec.logged
    {
        return Err(crate::job::busy(job));
    }
    if spec.human_only && !human {
        let reason = crate::Unavailable::new(format!(
            "only the human at the window can run {}",
            spec.name
        ));
        return Err(errors::unavailable(reason.fix(
            "this is the human's control; queries still work, and edits wait for the human",
        )));
    }
    if spec.kind == Kind::Query || human {
        return Ok(());
    }
    if origin.role == Role::Observer {
        let reason = crate::Unavailable::new("this client is an observer and may only run queries");
        return Err(errors::unavailable(
            reason.fix("use query commands, for example scene.describe"),
        ));
    }
    if control.paused {
        return Err(retry_later(
            ErrorCode::Paused,
            "a human paused agents in this session",
            2000,
        ));
    }
    if control.gesture {
        return Err(retry_later(
            ErrorCode::Busy,
            "a human is in the middle of a stroke",
            200,
        ));
    }
    Ok(())
}

fn retry_later(code: ErrorCode, message: &str, after_ms: u64) -> CommandError {
    CommandError {
        hint: Some("queries still work; retry this in a moment".into()),
        retryable: true,
        retry_after_ms: Some(after_ms),
        ..error(code, message)
    }
}

/// Prepares one step and, unless it is a dry run, runs it. Returns the resolved arguments and
/// the output.
fn run_step(
    registry: &Registry,
    spec: &CommandSpec,
    args: Value,
    cx: &mut Ctx,
    dry_run: bool,
) -> Result<Option<(Value, Value)>, CommandError> {
    let command = prepare(registry, spec, args, cx)?;
    if dry_run {
        return Ok(None);
    }
    let resolved = command.to_json();
    let writes = cx.doc.writes;
    let output = run_guarded(command, cx)?;
    if spec.kind != Kind::Edit && cx.doc.writes != writes {
        return Err(errors::internal("only edit commands may change parts"));
    }
    Ok(Some((resolved, output)))
}

/// Pins context-dependent arguments, validates and decodes them, and checks the command can
/// run now.
fn prepare(
    registry: &Registry,
    spec: &CommandSpec,
    args: Value,
    cx: &Ctx,
) -> Result<Box<dyn ErasedCommand>, CommandError> {
    let mut args = if args.is_null() { json!({}) } else { args };
    pin(spec, cx, &mut args)?;
    let mut command = registry.decode(spec, args)?;
    command.pin(cx)?;
    command.check(cx).map_err(errors::unavailable)?;
    Ok(command)
}

/// Runs a command, turning a panic into `E_INTERNAL`.
fn run_guarded(command: Box<dyn ErasedCommand>, cx: &mut Ctx) -> Result<Value, CommandError> {
    catch_unwind(AssertUnwindSafe(|| command.run(cx))).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(errors::internal(&format!(
            "{}{message}",
            crate::repro::PANICKED
        )))
    })
}

fn dry_run() -> Success {
    Success {
        result: json!({ "dry_run": true }),
        effect: None,
        history_id: None,
        warnings: vec![
            "dry run: the arguments are valid and the command can run now; nothing changed".into(),
        ],
    }
}

/// The command registry, built by the first engine in the process and shared by every engine
/// after it (decision 0008, amendment F8c). The commands are compiled in and the registry is
/// never changed, so every engine would build the same one.
fn shared_registry() -> Arc<Registry> {
    static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();
    Arc::clone(REGISTRY.get_or_init(|| Arc::new(Registry::load())))
}

/// The header every log run starts with.
fn log_header(registry: &Registry, config: &EngineConfig) -> LogHeader {
    LogHeader {
        gigasculpt_log: LOG_FORMAT,
        engine_version: registry.catalog.engine_version.clone(),
        protocol: PROTOCOL_VERSION,
        catalog_hash: registry.catalog.hash.clone(),
        session: config.session.clone(),
        seed: config.seed,
        verify: config.verify,
    }
}
