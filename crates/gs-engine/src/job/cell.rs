//! One job's shared state: what its thread reports, the engine's final answer, and the waiting.
//!
//! The job's thread writes progress; the engine thread ends the job (commit, failure or
//! cancel), only ever once; connection threads wait for the end and read the report.
//!
//! A finished job keeps its full response until a waiter has received it, or until it is no
//! longer among the newest finished jobs (`Jobs::release`). After that it keeps a summary: the
//! response with its `result` released and a warning saying so (review 0002 M3). The effect,
//! step id and any error stay.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gs_protocol::{CmdResponse, JobReport, Outcome};
use serde_json::Value;

/// One job.
pub(crate) struct JobCell {
    pub id: u64,
    /// The command it runs, or `cmd.batch`.
    pub command: String,
    /// What a person calls it: its long or background command's title ("Quad remesh").
    pub title: &'static str,
    /// The name of the client that started it.
    pub owner: String,
    cancel: AtomicBool,
    state: Mutex<State>,
    ended: Condvar,
}

#[derive(Default)]
struct State {
    progress: Option<f32>,
    message: Option<String>,
    response: Option<CmdResponse>,
    /// The response still holds its full result.
    full: bool,
    /// The job's thread has handed its work back, so it no longer counts as running.
    returned: bool,
}

impl JobCell {
    pub fn new(id: u64, command: String, title: &'static str, owner: String) -> Self {
        Self {
            id,
            command,
            title,
            owner,
            cancel: AtomicBool::new(false),
            state: Mutex::default(),
            ended: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How far the job got (0 to 1, or unknown) and what it is doing.
    pub fn progress(&self, done: Option<f32>, message: String) {
        let mut state = self.lock();
        state.progress = done.map(|d| d.clamp(0.0, 1.0));
        state.message = Some(message);
    }

    /// True once someone cancelled the job.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Cancels the job: it ends now with `response`, unless it already ended.
    pub fn cancel(&self, response: CmdResponse) {
        self.cancel.store(true, Ordering::SeqCst);
        self.end(response);
    }

    /// Ends the job with `response`, unless it already ended, and wakes everyone waiting.
    pub fn end(&self, response: CmdResponse) {
        let mut state = self.lock();
        if state.response.is_none() {
            state.full = true;
            state.response = Some(response);
            self.ended.notify_all();
        }
    }

    /// The job's thread handed its work back.
    pub fn returned(&self) {
        self.lock().returned = true;
    }

    /// True while the job's thread is still working, even on a job that already ended.
    pub fn working(&self) -> bool {
        !self.lock().returned
    }

    /// True once the job ended: committed, failed or cancelled.
    pub fn ended(&self) -> bool {
        self.lock().response.is_some()
    }

    /// How far the job got and what it is doing, as its thread last said, without taking the
    /// result (see [`JobCell::wait`]).
    pub fn peek(&self) -> (Option<f32>, Option<String>) {
        let state = self.lock();
        (state.progress, state.message.clone())
    }

    /// True if the job ended and still holds its full result.
    pub fn holds_result(&self) -> bool {
        self.lock().full
    }

    /// Keeps a summary of the response instead of the full result.
    pub fn release(&self) {
        release(&mut self.lock());
    }

    /// The report, after waiting at most `limit` for the job to end (`None`: until it ends).
    /// The first report with the response delivers the full result; later ones get the
    /// summary.
    pub fn wait(&self, limit: Option<Duration>) -> JobReport {
        let running = |state: &mut State| state.response.is_none();
        let state = self.lock();
        let mut state = match limit {
            Some(limit) => {
                self.ended
                    .wait_timeout_while(state, limit, running)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => self
                .ended
                .wait_while(state, running)
                .unwrap_or_else(PoisonError::into_inner),
        };
        let report = JobReport {
            id: Some(self.id),
            command: self.command.clone(),
            progress: state.progress,
            message: state.message.clone(),
            working: !state.returned,
            response: state.response.clone(),
        };
        release(&mut state);
        report
    }
}

/// Drops the full result of an ended job's response, keeping a summary.
fn release(state: &mut State) {
    if !state.full {
        return;
    }
    state.full = false;
    if let Some(CmdResponse {
        outcome: Outcome::Ok(success),
        ..
    }) = &mut state.response
        && !success.result.is_null()
    {
        success.result = Value::Null;
        success.warnings.push(
            "this job's result was released once delivered; the effect and step stay, so run a \
             query to look at the scene"
                .into(),
        );
    }
}
