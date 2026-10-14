//! The six MCP tools as plain data (no rmcp here): name, description, argument type and an
//! example. Each argument type derives its JSON Schema, so what a tool advertises and what it
//! decodes cannot drift apart.

use gs_protocol::Step;
use schemars::{JsonSchema, SchemaGenerator};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// What a tool does; the server dispatches on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Help,
    Run,
    Batch,
    Query,
    Look,
    Job,
}

/// One tool, as listed.
pub struct Spec {
    pub tool: Tool,
    pub name: &'static str,
    pub description: &'static str,
    /// The input schema.
    pub schema: fn() -> Value,
    /// A valid call, shown when the arguments are wrong.
    pub example: fn() -> Value,
}

/// The longest `gs_run` and `gs_batch` wait, well under Claude Code's 2-minute move to the
/// background.
pub const MAX_RUN_WAIT_MS: u64 = 100_000;
/// The longest `gs_job` wait.
pub const MAX_JOB_WAIT_MS: u64 = 60_000;
/// The default and largest image sizes (docs/spikes/mcp.md: 1024 px costs about 1,370 tokens,
/// and past about 1568 px more pixels buy nothing).
pub const IMAGE_SIZE: u32 = 1024;
pub const MAX_IMAGE_SIZE: u32 = 1568;

/// `gs_help`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelpArgs {
    /// Words to search command names and docs for, for example "sphere" or "undo".
    #[serde(default)]
    pub query: Option<String>,
    /// One exact command name, for example "primitive.sphere": its docs, argument schema and
    /// examples.
    #[serde(default)]
    pub command: Option<String>,
}

/// `gs_run` and `gs_query`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunArgs {
    /// The command, for example `primitive.sphere`. Find commands with `gs_help`.
    pub command: String,
    /// Its arguments as a JSON object; `gs_help` with `command` shows them.
    #[serde(default)]
    pub args: Map<String, Value>,
    /// How long to wait for a long command before answering with its job id, in milliseconds
    /// (default 20000, at most 100000). The job goes on either way.
    #[serde(default = "default_wait")]
    pub wait_ms: u64,
}

/// `gs_batch`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchArgs {
    /// The commands, in order: [{"command": ..., "args": {...}}, ...]. Only edits and queries.
    pub commands: Vec<Step>,
    /// As for `gs_run`.
    #[serde(default = "default_wait")]
    pub wait_ms: u64,
}

/// `gs_look`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookArgs {
    /// The views: each a name (front, back, left, right, top, bottom, iso), a session view
    /// ("main" looks from exactly where the window looks; your own is "agent:<your name>", moved
    /// with the view.* commands), an exact camera as view.get reports it, or an orbit
    /// `{"view": name, "azimuth_deg": a, "elevation_deg": e, "distance": d, "target": [x, y,
    /// z]}`, every field optional. Several views come back as one contact sheet. Default: iso.
    #[serde(default)]
    pub views: Vec<Value>,
    /// Width of the image in pixels, 64 to 1568 (larger is clamped); default 1024.
    #[serde(default = "default_size")]
    pub size: u32,
    /// "matcap" (default), "clay", "zebra" (reflection stripes that show small shape changes)
    /// or "normals".
    #[serde(default)]
    pub shading: Option<String>,
    /// Draw the edges over the surface.
    #[serde(default)]
    pub wireframe: bool,
}

/// `gs_job`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobArgs {
    /// The job's id, from `gs_run` or `gs_batch`.
    pub id: u64,
    /// How long to wait for it to end, in milliseconds (default 0, at most 60000).
    #[serde(default)]
    pub wait_ms: u64,
    /// Cancel it: it ends at once and changes nothing.
    #[serde(default)]
    pub cancel: bool,
}

fn default_wait() -> u64 {
    20_000
}

fn default_size() -> u32 {
    IMAGE_SIZE
}

fn schema<T: JsonSchema>() -> Value {
    SchemaGenerator::default()
        .into_root_schema_for::<T>()
        .to_value()
}

/// Every tool, in the order they are listed.
pub fn all() -> [Spec; 6] {
    [
        Spec {
            tool: Tool::Help,
            name: "gs_help",
            description: "Find GigaSculpt commands and read their docs. With `query`, lists \
                          the commands matching the words; with `command`, shows its docs, \
                          argument schema and examples; with neither, lists every command.",
            schema: schema::<HelpArgs>,
            example: || json!({ "query": "sphere" }),
        },
        Spec {
            tool: Tool::Run,
            name: "gs_run",
            description: "Run one command, as one undo step. Answers with what it did (one \
                          `path: value` line each) or an error with a fix hint. A long command \
                          still running after wait_ms answers with its job id; follow it with \
                          gs_job.",
            schema: schema::<RunArgs>,
            example: || json!({ "command": "primitive.sphere", "args": { "subdivisions": 4 } }),
        },
        Spec {
            tool: Tool::Batch,
            name: "gs_batch",
            description: "Run several edit and query commands in order as one undo step. If \
                          any fails, none of them changes anything.",
            schema: schema::<BatchArgs>,
            example: || {
                json!({ "commands": [
                    { "command": "primitive.sphere", "args": {} },
                    { "command": "mesh.stats", "args": {} }
                ] })
            },
        },
        Spec {
            tool: Tool::Query,
            name: "gs_query",
            description: "Run a query command, which only reads: mesh.stats, scene.describe, \
                          mesh.hash and the like. Refuses commands that could change anything.",
            schema: schema::<RunArgs>,
            example: || json!({ "command": "scene.describe", "args": {} }),
        },
        Spec {
            tool: Tool::Look,
            name: "gs_look",
            description: "See the model: renders the scene offscreen as one PNG, never moving \
                          the window's camera. Several views come back as one labelled contact \
                          sheet.",
            schema: schema::<LookArgs>,
            example: || json!({ "views": ["front", "iso"], "shading": "matcap" }),
        },
        Spec {
            tool: Tool::Job,
            name: "gs_job",
            description: "Follow a long command started with gs_run or gs_batch: how far it \
                          got, or its result once it ended. Can wait up to 60 s, or cancel it.",
            schema: schema::<JobArgs>,
            example: || json!({ "id": 1, "wait_ms": 60_000 }),
        },
    ]
}
