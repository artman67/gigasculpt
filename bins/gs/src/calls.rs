//! Running calls and following jobs: the one path every `gs` subcommand and every MCP tool
//! takes to the engine.
//!
//! A call goes out as `job.start`. Short calls come back finished; a long one becomes a job,
//! which this follows with `job.status` in slices of at most [`SLICE`], telling the caller after
//! each slice, until it ends or the time allowed runs out.

use std::time::Duration;

use gs_protocol::{Catalog, JobParams, Kind, Request, StartParams, method};
use serde_json::{Value, json};

use crate::connection::Connection;
use crate::failure::Failure;

/// The longest single wait for a job, so progress is heard every few seconds.
pub const SLICE: Duration = Duration::from_secs(2);

/// How a call ended up.
pub enum Ended {
    /// It ended: its response exactly as the engine sent it.
    Done(Value),
    /// It still runs as a job after the time allowed: the job's report.
    Running(Value),
}

/// Runs `request`, waiting for it at most `patience`. `progress` hears each report of a job
/// that is still running. The engine is asked to wait at most half of `patience` per request,
/// so its answer has the other half to arrive: on the command line `patience` is also how long
/// the connection waits for any answer, and a wait as long as that lost the race on a busy
/// machine ("did not answer job.start") instead of naming the job.
pub fn run(
    connection: &mut Connection,
    request: Request,
    patience: Duration,
    progress: &mut dyn FnMut(&Value),
) -> Result<Ended, Failure> {
    let first = (patience / 2).min(SLICE);
    let params = match request {
        Request::Run(run) => StartParams {
            run: Some(run),
            wait_ms: millis(first),
            ..StartParams::default()
        },
        Request::Batch(batch) => StartParams {
            batch: Some(batch),
            wait_ms: millis(first),
            ..StartParams::default()
        },
    };
    let report = connection.call(method::JOB_START, json!(params))?;
    follow(connection, report, patience.saturating_sub(first), progress)
}

/// Follows a job's report for at most `left` more, until the job ends.
pub fn follow(
    connection: &mut Connection,
    mut report: Value,
    mut left: Duration,
    progress: &mut dyn FnMut(&Value),
) -> Result<Ended, Failure> {
    loop {
        match report.get("response") {
            Some(response) => return Ok(Ended::Done(response.clone())),
            _ if left.is_zero() => return Ok(Ended::Running(report)),
            _ => progress(&report),
        }
        let wait = left.min(SLICE);
        report = status(connection, &report["id"], wait, false)?;
        left -= wait;
    }
}

/// A job's report, after waiting at most `wait` for it to end; cancelled first with `cancel`.
pub fn status(
    connection: &mut Connection,
    id: &Value,
    wait: Duration,
    cancel: bool,
) -> Result<Value, Failure> {
    let id = id
        .as_u64()
        .ok_or_else(|| Failure::Protocol(format!("a job without an id: {id}")))?;
    let params = JobParams {
        id,
        wait_ms: millis(wait),
        cancel,
    };
    connection.call(method::JOB_STATUS, json!(params))
}

/// Refuses a command that is not a query, before it runs. Unknown names pass, for the engine to
/// answer with did-you-mean suggestions.
pub fn must_be_query(catalog: &Catalog, name: &str) -> Result<(), Failure> {
    match catalog.commands.iter().find(|info| info.name == name) {
        Some(info) if info.kind != Kind::Query => Err(Failure::Usage(format!(
            "{name} is not a query, so it may change the scene; run it with `gs call` (gs_run)"
        ))),
        _ => Ok(()),
    }
}

fn millis(wait: Duration) -> u64 {
    u64::try_from(wait.as_millis()).unwrap_or(u64::MAX)
}
