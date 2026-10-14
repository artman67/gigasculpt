//! `gs mcp`: the session as six MCP tools for agents (ARCHITECTURE 7.4, `docs/spikes/mcp.md`).
//!
//! `adapter` is the only file that knows rmcp; `tools` lists the tools as plain data. This
//! file runs each tool call through the code the command line uses (`calls`, `help`, `look`,
//! `output`), so a tool answers with exactly the text `gs` prints: results as `path: value`
//! lines, errors with their code, path, hint and example, and at most one image.
//!
//! The session is found as for any `gs` command, when the first call needs it (Claude Code never
//! calls `initialize`, so nothing happens there). Connections are kept and shared between calls.

mod adapter;
mod tools;

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gs_protocol::{BatchParams, CommandError, ErrorCode, Request, Role};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::calls::{self, Ended};
use crate::connection::{Connection, one};
use crate::failure::Failure;
use crate::look::{self, Look};
use crate::output::{self, Surface, checked};
use crate::{help, session};

pub use tools::{Spec, Tool};

/// How `gs mcp` was started.
pub struct Settings {
    /// `-s NAME`; else `$GS_SESSION`; else the only running session (or, with `spawn`, `main`).
    pub session: Option<String>,
    /// Start the session when it is not running.
    pub spawn: bool,
    pub role: Role,
    /// How long to wait for the session to connect and to answer.
    pub patience: Duration,
}

/// A tool's answer: text, at most one PNG (as base64), and whether it reports an error.
pub struct Reply {
    pub text: String,
    pub png: Option<String>,
    pub is_error: bool,
}

impl Reply {
    fn text(lines: &[String]) -> Self {
        Self {
            text: lines.join("\n"),
            png: None,
            is_error: false,
        }
    }

    fn failed(failure: &Failure) -> Self {
        Self {
            is_error: true,
            ..Self::text(&output::failure(failure))
        }
    }
}

/// Serves MCP on stdin and stdout until the client leaves. The async runtime starts here, so
/// no other `gs` command pays for it.
pub fn serve(settings: Settings) -> Result<(), Failure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::Protocol(format!("cannot start the MCP server: {e}")))?;
    let server = Server {
        settings,
        idle: Mutex::new(Vec::new()),
        opening: Mutex::new(()),
    };
    runtime
        .block_on(adapter::serve(server))
        .map_err(Failure::Protocol)
}

/// The session side of the server, shared by every tool call.
pub struct Server {
    settings: Settings,
    /// Connections not in use.
    idle: Mutex<Vec<Connection>>,
    /// Held while a connection opens, so two calls never spawn the session twice.
    opening: Mutex<()>,
}

impl Server {
    /// Runs one tool call. `progress` hears a line every few seconds while a job runs.
    pub fn call(&self, name: &str, args: Value, progress: &mut dyn FnMut(String)) -> Reply {
        let Some(spec) = tools::all().into_iter().find(|s| s.name == name) else {
            let known: Vec<&str> = tools::all().iter().map(|s| s.name).collect();
            let text = format!("no tool named {name}; the tools are {}", known.join(", "));
            return Reply::failed(&Failure::Usage(text));
        };
        let mut connection = match self.connection() {
            Ok(connection) => connection,
            Err(failure) => return Reply::failed(&failure),
        };
        let mut heard = |report: &Value| {
            let line = output::running(report, Surface::Mcp).into_iter().next();
            progress(line.unwrap_or_default());
        };
        let result = dispatch(&spec, &mut connection, args, &mut heard);
        // A connection that failed itself (not just its command) is dropped.
        if matches!(result, Ok(_) | Err(Failure::Command(_) | Failure::Usage(_))) {
            lock(&self.idle).push(connection);
        }
        result.unwrap_or_else(|failure| Reply::failed(&failure))
    }

    /// An idle connection, or a new one to the session (started first with `--spawn`).
    fn connection(&self) -> Result<Connection, Failure> {
        if let Some(connection) = lock(&self.idle).pop() {
            return Ok(connection);
        }
        let _opening = lock(&self.opening);
        let Settings {
            session,
            spawn,
            role,
            patience,
        } = &self.settings;
        let name = if *spawn {
            let name = session.clone().or_else(|| std::env::var("GS_SESSION").ok());
            let name = name.unwrap_or_else(|| "main".into());
            if !session::is_running(&name)? {
                // As long as anything else may take: an engine that dies fails at once, but a
                // busy machine can take more than 10 s to start one.
                let how = session::Spawn {
                    seed: 0,
                    verify: false,
                    timeout: *patience,
                };
                // The stop key goes to the server's log, not to the agent.
                eprintln!("{}", session::spawn(&name, &how)?.join("\n"));
            }
            name
        } else {
            session::pick(session.clone())?
        };
        session::connect(&name, *role, *patience)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs one tool on a connection.
fn dispatch(
    spec: &Spec,
    connection: &mut Connection,
    args: Value,
    progress: &mut dyn FnMut(&Value),
) -> Result<Reply, Failure> {
    match spec.tool {
        Tool::Help => {
            let a: tools::HelpArgs = decode(spec, args)?;
            let lines = match (a.command, a.query) {
                (Some(name), _) => help::describe(connection, &name)?,
                (None, query) => {
                    let query = query.unwrap_or_default();
                    let mut lines = help::list(connection, &query)?;
                    if lines.is_empty() {
                        lines.push(format!("no command matches '{query}'; try fewer words"));
                    }
                    lines
                }
            };
            Ok(Reply::text(&lines))
        }
        Tool::Run | Tool::Query => {
            let a: tools::RunArgs = decode(spec, args)?;
            if spec.tool == Tool::Query {
                calls::must_be_query(&connection.catalog()?, &a.command)?;
            }
            let request = one(&a.command, Value::Object(a.args));
            finish(connection, request, a.wait_ms, progress)
        }
        Tool::Batch => {
            let a: tools::BatchArgs = decode(spec, args)?;
            let request = Request::Batch(BatchParams {
                commands: a.commands,
                if_rev: None,
            });
            finish(connection, request, a.wait_ms, progress)
        }
        Tool::Look => {
            let a: tools::LookArgs = decode(spec, args)?;
            let look = Look {
                views: a.views,
                size: a.size.clamp(64, tools::MAX_IMAGE_SIZE),
                shading: a.shading,
                wireframe: a.wireframe,
                path: None,
            };
            let response = look::look(connection, &look)?;
            checked(&response)?;
            let result = &response["ok"]["result"];
            let mut reply = Reply::text(&[look::summary(result)]);
            reply.png = result["png_base64"].as_str().map(str::to_string);
            Ok(reply)
        }
        Tool::Job => {
            let a: tools::JobArgs = decode(spec, args)?;
            let wait = Duration::from_millis(a.wait_ms.min(tools::MAX_JOB_WAIT_MS));
            let report = calls::status(connection, &a.id.into(), Duration::ZERO, a.cancel)?;
            let command = report["command"].as_str().unwrap_or_default().to_string();
            ended(&command, calls::follow(connection, report, wait, progress)?)
        }
    }
}

/// Runs a call, waiting at most `wait_ms` for it.
fn finish(
    connection: &mut Connection,
    request: Request,
    wait_ms: u64,
    progress: &mut dyn FnMut(&Value),
) -> Result<Reply, Failure> {
    let wait = Duration::from_millis(wait_ms.min(tools::MAX_RUN_WAIT_MS));
    let command = request.name().to_string();
    ended(&command, calls::run(connection, request, wait, progress)?)
}

/// What a call of `command` came to: its response as `gs call` prints it, or the job still
/// running.
fn ended(command: &str, ended: Ended) -> Result<Reply, Failure> {
    match ended {
        Ended::Done(response) => {
            checked(&response)?;
            Ok(Reply::text(&output::success(command, &response)))
        }
        Ended::Running(report) => Ok(Reply::text(&output::running(&report, Surface::Mcp))),
    }
}

/// A tool's arguments, or `E_ARGS_INVALID` at the bad one with the tool's example.
fn decode<T: DeserializeOwned>(spec: &Spec, args: Value) -> Result<T, Failure> {
    serde_path_to_error::deserialize(args).map_err(|e| {
        let path = e.path().to_string();
        Failure::Command(Box::new(CommandError {
            code: ErrorCode::ArgsInvalid,
            message: format!("{}: {}", spec.name, e.inner()),
            path: (path != ".").then(|| format!("/{}", path.replace('.', "/"))),
            hint: Some(format!("see the input schema of {}", spec.name)),
            suggestions: Vec::new(),
            violations: Vec::new(),
            example: Some((spec.example)()),
            retryable: false,
            retry_after_ms: None,
        }))
    })
}
