//! Wire messages: JSON-RPC 2.0, one JSON object per line, over a Unix socket.
//!
//! The whole protocol is seven methods (see `method`). Everything else is a command run through
//! `cmd.run`, `cmd.batch` or `job.start`, which also hands back long calls as jobs.
//!
//! Everything a client sends refuses unknown fields, so a misspelled `dry_run` or `if_rev` is an
//! error instead of a silent default. What the engine sends back stays lenient, so a client built
//! against an older engine still reads a reply that gained a field.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{CommandError, Step, Vec3};

/// Version of this wire protocol. Bumped only for incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Wire method names.
pub mod method {
    /// Introduce the client: name and role. Returns engine and session facts.
    pub const HELLO: &str = "engine.hello";
    /// The command catalog.
    pub const CATALOG: &str = "catalog.get";
    /// Run one command.
    pub const RUN: &str = "cmd.run";
    /// Run several commands as one atomic undo step.
    pub const BATCH: &str = "cmd.batch";
    /// Stop the engine after replying.
    pub const SHUTDOWN: &str = "engine.shutdown";
    /// Start a call (as `cmd.run` or `cmd.batch`) and wait for it at most `wait_ms`. Returns a
    /// `JobReport`: finished, or still running as a job.
    pub const JOB_START: &str = "job.start";
    /// A job's report, after waiting at most `wait_ms` for it to end; `cancel` stops it first.
    pub const JOB_STATUS: &str = "job.status";
}

/// JSON-RPC error codes for protocol-level failures. Command failures are not protocol errors:
/// they come back as a normal result whose outcome is `error`.
pub mod rpc_code {
    /// The line was not valid JSON (or not UTF-8).
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON was not a valid request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Unknown wire method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The method's parameters were malformed.
    pub const INVALID_PARAMS: i64 = -32602;
    /// The client's role does not allow this request.
    pub const REFUSED: i64 = -32001;
    /// The request line is longer than the engine accepts. The connection is closed after it.
    /// The error's `data` is the `E_LIMIT` command error.
    pub const TOO_LARGE: i64 = -32002;
}

/// A JSON-RPC 2.0 request.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Echoed in the response. Left out, the request is a notification and gets no reply.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<Value>,
    /// One of the names in `method`.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
}

/// Reads a field that is present, keeping an explicit `null` as `Some(Null)` so it can be told
/// apart from a missing field.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// A JSON-RPC 2.0 response: exactly one of `result` and `error` is set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request's id.
    pub id: Value,
    /// The method's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// A protocol-level failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcError {
    /// One of `rpc_code`.
    pub code: i64,
    /// What went wrong.
    pub message: String,
    /// Details: for a misspelled field, its JSON pointer and did-you-mean candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Who a client is. Decides what it may do while a human is working.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The human at the window. Never blocked. Only the window, inside the engine process, has
    /// this role; the socket never grants it.
    Human,
    /// An agent or script. Blocked while paused or while a human gesture is in progress.
    #[default]
    Agent,
    /// Read-only: may run query commands only.
    Observer,
}

/// Parameters of `engine.hello`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    /// A short client name, for example `gs` or `claude-code`. Steps are attributed to it, and a
    /// plain `history.undo` from an agent only undoes steps made under the same name.
    pub client: String,
    /// The client's role: `agent` or `observer`.
    #[serde(default)]
    pub role: Role,
    /// The protocol version the client speaks. Missing means the engine's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
}

/// Result of `engine.hello`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelloResult {
    /// Engine version.
    pub engine_version: String,
    /// Wire protocol version.
    pub protocol: u32,
    /// Session name.
    pub session: String,
    /// The id this connection's edits are recorded under.
    pub client_id: u64,
    /// The role the engine granted.
    pub role: Role,
    /// Catalog hash, to cache the catalog.
    pub catalog_hash: String,
}

/// Parameters of `cmd.run`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    /// Command name.
    pub command: String,
    /// Arguments as a JSON object. Missing means none.
    #[serde(default)]
    pub args: Value,
    /// Refuse with `E_STALE` unless the document is still at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_rev: Option<u64>,
    /// Validate and check only; do not run.
    #[serde(default)]
    pub dry_run: bool,
}

impl RunParams {
    /// A run of `command` with `args`, as a client sends it naming only those two: at whatever
    /// revision the document is, and not a dry run. The one way code builds a run request; set
    /// `if_rev` with struct update syntax where a call needs it.
    pub fn new(command: impl Into<String>, args: Value) -> Self {
        Self {
            command: command.into(),
            args,
            if_rev: None,
            dry_run: false,
        }
    }

    /// The same call as a dry run: validated and checked, not run.
    #[must_use]
    pub fn dry(self) -> Self {
        Self {
            dry_run: true,
            ..self
        }
    }
}

/// Parameters of `cmd.batch`: the commands run in order as one undo step, and if any fails the
/// whole batch is rolled back.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchParams {
    /// The commands, at least one. Only edit and query commands are allowed.
    pub commands: Vec<Step>,
    /// Refuse with `E_STALE` unless the document is still at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_rev: Option<u64>,
}

/// Parameters of `engine.shutdown`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShutdownParams {
    /// The stop key the session was started with. `gs spawn` makes one and prints it, so only
    /// the client that spawned a session can stop it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// Parameters of `job.start`: exactly one of `run` and `batch`.
///
/// A call whose steps include a long command (`Cost::Long`) runs as a job: on its own thread,
/// against a copy of the document, while the session keeps answering. It commits as one undo
/// step when it finishes, or fails with `E_STALE` if the document changed meanwhile. Other
/// calls run at once and come back finished.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartParams {
    /// One command, as `cmd.run` takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunParams>,
    /// Several commands as one undo step, as `cmd.batch` takes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<BatchParams>,
    /// How long to wait for the call, in milliseconds, before answering with the job still
    /// running. The job goes on either way.
    #[serde(default)]
    pub wait_ms: u64,
}

impl StartParams {
    /// The request to run.
    pub fn request(self) -> Result<Request, String> {
        match (self.run, self.batch) {
            (Some(run), None) => Ok(Request::Run(run)),
            (None, Some(batch)) => Ok(Request::Batch(batch)),
            _ => Err("give exactly one of run and batch".into()),
        }
    }
}

/// Parameters of `job.status`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobParams {
    /// The job's id, from `job.start`.
    pub id: u64,
    /// How long to wait for the job to end, in milliseconds, before answering.
    #[serde(default)]
    pub wait_ms: u64,
    /// Stop the job. It ends at once with `E_CANCELLED` and never changes the document.
    #[serde(default)]
    pub cancel: bool,
}

/// A call started with `job.start`, or a job looked up with `job.status`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobReport {
    /// The job's id. Missing when the call finished at once without becoming a job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The command it runs, or `cmd.batch`.
    pub command: String,
    /// How far it got, 0 to 1, when the command says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    /// What it is doing now, when the command says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// True while the job's thread is still working, even once the job has ended: a cancelled
    /// job ends at once, and its thread stops when its command next looks.
    #[serde(default)]
    pub working: bool,
    /// The call's response once it has ended, exactly as `cmd.run` or `cmd.batch` gives it.
    /// Missing while it runs. Once delivered, later reports give it with `result` released.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<CmdResponse>,
}

/// What a client asks the engine to run: one command (`cmd.run`) or a batch (`cmd.batch`).
#[derive(Clone, Debug)]
pub enum Request {
    /// Run one command (`cmd.run`).
    Run(RunParams),
    /// Run several commands as one undo step (`cmd.batch`).
    Batch(BatchParams),
}

impl Request {
    /// The command it runs, or `cmd.batch` for a batch.
    pub fn name(&self) -> &str {
        match self {
            Self::Run(params) => &params.command,
            Self::Batch(_) => method::BATCH,
        }
    }

    /// The wire method that carries this request.
    pub fn method(&self) -> &'static str {
        match self {
            Self::Run(_) => method::RUN,
            Self::Batch(_) => method::BATCH,
        }
    }

    /// The request's parameters as JSON.
    pub fn params(&self) -> Value {
        let params = match self {
            Self::Run(params) => serde_json::to_value(params),
            Self::Batch(params) => serde_json::to_value(params),
        };
        params.unwrap_or_default()
    }
}

/// Result of `cmd.run` and `cmd.batch`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CmdResponse {
    /// Global sequence number of the command (the first command, for a batch). Missing when it
    /// was rejected before running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Document revision after the call. Changes whenever the document changes.
    pub rev: u64,
    /// Document changes made by other clients since this client's previous call.
    pub concurrent_changes: u64,
    /// What happened.
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// Success or failure of a command.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It ran.
    Ok(Success),
    /// It failed and the document is unchanged.
    Error(CommandError),
}

/// What a successful command returned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Success {
    /// The command's output (an array of outputs for a batch).
    pub result: Value,
    /// What an edit changed. Set for edit commands and batches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    /// The undo step this call created, if any. Step ids count undo steps (1, 2, 3...), so a
    /// replay of the session's log gives every step the same id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_id: Option<u64>,
    /// Things worth knowing, for example why an edit changed nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What an edit actually changed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Effect {
    /// Parts that changed.
    pub parts: Vec<String>,
    /// Vertices whose position or vertex attributes changed, plus the corners of faces whose
    /// face attributes changed (every vertex of a part whose faces changed). Zero with no
    /// topology change means nothing changed.
    pub verts_touched: u64,
    /// Bounds of the touched vertices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[Vec3; 2]>,
    /// True if faces were added, removed or rewired.
    pub topology_changed: bool,
    /// What changed in each part, by part name, in the names agents read: `positions` (its
    /// points, in its own space), `faces` (added, removed or rewired), each attribute column
    /// by the name `part.get` gives it (`mask`, `group`, `color`, `hide`, `uv0.u` ...), its
    /// detail `levels`, its `transform`, `rank`, eye (`visible`) and `voxel_mode`, and its Live
    /// Boolean `operation` and `start`. A part added or
    /// removed, or whose faces changed, lists its positions, faces, every column and its
    /// levels. Parts that did not change are left out; so is the whole field when none did.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub changed: BTreeMap<String, Vec<String>>,
    /// Why the edit changed nothing. Set exactly when nothing changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noop_reason: Option<String>,
}
