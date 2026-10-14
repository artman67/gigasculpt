//! The command catalog: what the engine can do, as data any client can read.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ErrorCode;

/// What a command may do. Drives queueing, undo, logging and permissions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Reads state, changes nothing. Not logged.
    Query,
    /// Changes document data. Atomic, and exactly one undo step unless it changed nothing.
    Edit,
    /// Changes the camera or display settings. Logged, not undoable.
    View,
    /// Session control: undo, redo, checkpoints, pausing agents.
    Session,
}

/// How long a command is expected to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cost {
    /// Under 16 ms.
    Instant,
    /// Under 2 s.
    Short,
    /// Longer; will run as a job.
    Long,
}

/// The full command catalog, as served by `catalog.get`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    /// Engine version that produced this catalog.
    pub engine_version: String,
    /// Wire protocol version.
    pub protocol: u32,
    /// Hash of the command list; changes whenever any command, schema or example changes.
    pub hash: String,
    /// Every command, sorted by name.
    pub commands: Vec<CommandInfo>,
}

/// One command in the catalog.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CommandInfo {
    /// Lowercase dotted name, noun first, for example `primitive.sphere`.
    pub name: String,
    /// What the command may do.
    pub kind: Kind,
    /// How long it is expected to take.
    pub cost: Cost,
    /// The first line of the docs.
    pub summary: String,
    /// The full docs.
    pub docs: String,
    /// JSON Schema of the arguments.
    pub args_schema: Value,
    /// JSON Schema of the result.
    pub output_schema: Value,
    /// Examples. Tests run every one, so they cannot rot.
    pub examples: Vec<ExampleInfo>,
    /// True if only the human at the window may run it; agents and observers get
    /// `E_PRECONDITION`.
    #[serde(default)]
    pub human_only: bool,
    /// Arguments the engine fills in when they are left out (the active part, a free part name, a
    /// random seed), so the log records exactly what ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    /// What the command promises beyond the rules every command obeys, for example
    /// `"preserves_topology"`. The test kit (`gs-testkit`, module `claims`) defines the
    /// vocabulary and checks every claim on random command sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<Value>,
    /// The calls that put a part in the state this command needs before it changes anything
    /// (a part with a transform, before `part.apply_transform` bakes it), written as an
    /// example's setup is, without the part. The test kit checks the command's claims on plans
    /// that start with them on every part. Empty for almost every command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<Step>,
}

/// One example of a command, with the commands that set up its starting state.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExampleInfo {
    /// What the example shows.
    pub summary: String,
    /// Commands to run first, on an empty scene.
    #[serde(default)]
    pub setup: Vec<Step>,
    /// The example's arguments.
    pub args: Value,
    /// Set when the example demonstrates a failure, with the error it must produce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fails_with: Option<ErrorCode>,
}

/// One command call: a name and its arguments. Used by examples, batches and scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Command name.
    pub command: String,
    /// Arguments as a JSON object. Missing means none.
    #[serde(default)]
    pub args: Value,
}
