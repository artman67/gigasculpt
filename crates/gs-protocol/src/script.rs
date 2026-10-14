//! The command log and `gs run` scripts: one JSON object per line.
//!
//! A session's log starts with a header line naming what wrote it, then has one line per call that
//! may have changed the session, in the order the engine ran them: a single command, or a whole
//! batch as one `cmd.batch` line. Replaying the lines in order into a fresh session rebuilds the
//! same scene, because
//!
//! - arguments are resolved: defaults filled in, parts, new part names and random seeds pinned;
//! - one line is one call, so one line is at most one undo step, as in the original session;
//! - undo step ids count undo steps (1, 2, 3...), so an id in an `expect` argument names the same
//!   step in the replay; queries and failed commands are not logged and do not move them.
//!
//! In verify mode each line also carries the scene hash after it, so a replay can stop at the
//! first line that diverges. A hand-written script is the same lines without the header and the
//! bookkeeping fields.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BatchParams, Request, RunParams, method};

/// The first line of every log. A session that restarts writes a new log with a new header.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogHeader {
    /// Log format version, 1.
    pub gigasculpt_log: u32,
    /// Engine version that wrote the log.
    pub engine_version: String,
    /// Wire protocol version.
    pub protocol: u32,
    /// Hash of the command catalog. A replay into an engine with another catalog may differ.
    pub catalog_hash: String,
    /// Session name.
    pub session: String,
    /// Session seed. Random commands pin their own seed, so a replay does not need it.
    pub seed: u64,
    /// True if every line carries the scene hash after it.
    pub verify: bool,
}

/// The log format version this crate writes and reads.
pub const LOG_FORMAT: u32 = 1;

/// One call in a log or script.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRecord {
    /// Sequence number of the call (its first command, for a batch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// The client that ran it, as `name#connection`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Command name, or `cmd.batch` for a batch.
    pub command: String,
    /// Resolved arguments; for a batch, `{"commands": [{"command", "args"}, ...]}`.
    #[serde(default)]
    pub args: Value,
    /// In verify mode, the scene hash (`mesh.hash`) after the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl LogRecord {
    /// The request that replays this line: `cmd.batch` for a batch, `cmd.run` otherwise.
    pub fn request(&self) -> Result<Request, String> {
        if self.command == method::BATCH {
            let batch: BatchParams = serde_json::from_value(self.args.clone())
                .map_err(|e| format!("a cmd.batch line needs {{\"commands\": [...]}}: {e}"))?;
            return Ok(Request::Batch(batch));
        }
        Ok(Request::Run(RunParams::new(
            self.command.clone(),
            self.args.clone(),
        )))
    }
}

/// One line of a log or script.
#[derive(Clone, Debug)]
pub enum ScriptLine {
    /// The header that starts a log.
    Header(LogHeader),
    /// A call.
    Record(LogRecord),
}

impl ScriptLine {
    /// Parses one line: a header if it has a `gigasculpt_log` field, a call otherwise.
    pub fn parse(line: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if value.get("gigasculpt_log").is_some() {
            serde_json::from_value(value)
                .map(Self::Header)
                .map_err(|e| format!("bad log header: {e}"))
        } else {
            serde_json::from_value(value)
                .map(Self::Record)
                .map_err(|e| e.to_string())
        }
    }
}
