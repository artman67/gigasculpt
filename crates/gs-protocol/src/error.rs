//! Errors with stable codes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A stable machine-readable error code. Codes are only ever added, never renamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ErrorCode {
    /// No command has that name.
    #[serde(rename = "E_UNKNOWN_COMMAND")]
    UnknownCommand,
    /// The arguments do not match the command's schema.
    #[serde(rename = "E_ARGS_INVALID")]
    ArgsInvalid,
    /// A part, checkpoint or other named thing does not exist.
    #[serde(rename = "E_NOT_FOUND")]
    NotFound,
    /// The command cannot run in the current state; the message says why.
    #[serde(rename = "E_PRECONDITION")]
    Precondition,
    /// A human is in the middle of a gesture; retry shortly.
    #[serde(rename = "E_BUSY")]
    Busy,
    /// A human paused agents in this session.
    #[serde(rename = "E_PAUSED")]
    Paused,
    /// The command was cancelled.
    #[serde(rename = "E_CANCELLED")]
    Cancelled,
    /// The command or the connection timed out.
    #[serde(rename = "E_TIMEOUT")]
    Timeout,
    /// A memory or size limit would be exceeded.
    #[serde(rename = "E_LIMIT")]
    Limit,
    /// The document changed since the revision or history step the caller expected.
    #[serde(rename = "E_STALE")]
    Stale,
    /// A bug in the engine. The document was rolled back.
    #[serde(rename = "E_INTERNAL")]
    Internal,
    /// A file is damaged or incomplete; the message names the place (decision 0016).
    #[serde(rename = "E_DAMAGED")]
    Damaged,
}

/// A command failure, written so an agent can fix its call without reading source code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandError {
    /// Stable code.
    pub code: ErrorCode,
    /// One plain sentence.
    pub message: String,
    /// JSON pointer to the offending argument, for example `/subdivisions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// How to fix it, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Did-you-mean candidates or valid values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    /// Every schema violation, when there were several.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<Violation>,
    /// A valid example of the command's arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
    /// True if the same call may succeed later unchanged.
    #[serde(default)]
    pub retryable: bool,
    /// For a retryable error, how long to wait before retrying, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// One schema violation in a command's arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Violation {
    /// JSON pointer to the offending value.
    pub path: String,
    /// What is wrong with it.
    pub message: String,
}
