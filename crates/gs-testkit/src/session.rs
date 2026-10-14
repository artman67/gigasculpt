//! An in-process session for tests: runs commands by name with JSON arguments, exactly as socket
//! clients do, and reads results back through commands.
//!
//! ```text
//! let mut s = Session::headless_seeded(1);
//! s.run("primitive.sphere", json!({"subdivisions": 5}))?;
//! let before = s.hash();
//! s.run("mesh.jitter", json!({"amount": "1%"}))?;
//! assert_ne!(s.hash(), before, "the jitter moved something");
//! s.run("history.undo", json!({}))?;
//! assert_eq!(s.hash(), before, "undo put it back");
//! ```
//!
//! (Shown as text so `cargo test` and mutation runs do not pay for a doctest build; the same
//! calls run in `tests/session.rs`.)

use std::collections::BTreeMap;

use gs_engine::{Engine, EngineConfig, Origin, Request};
use gs_protocol::{
    BatchParams, CmdResponse, CommandError, ErrorCode, Outcome, PlainMesh, Role, RunParams, Step,
    Success,
};
use serde_json::{Value, json};

/// Every part of a scene, by name.
pub type Scene = BTreeMap<String, PlainMesh>;

/// One engine with one client, driven in-process.
pub struct Session {
    /// The engine.
    pub engine: Engine,
    /// The client every call comes from.
    pub origin: Origin,
    transcript: Option<Vec<Exchange>>,
}

/// One call and the engine's response, serialized, as a recording session keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    /// Command name.
    pub command: String,
    /// Arguments as sent.
    pub args: Value,
    /// The whole response as JSON text, so equal text means every number is bit-identical
    /// (floats print in their shortest exact form, and `-0.0` prints as `-0.0`).
    pub response: String,
}

impl Session {
    /// A session with seed 0, driven by a human client (so every command is allowed).
    pub fn new() -> Self {
        Self::headless_seeded(0)
    }

    /// A session with the given seed, driven by a human client.
    pub fn headless_seeded(seed: u64) -> Self {
        let config = EngineConfig {
            seed,
            ..EngineConfig::default()
        };
        Self::with(config, Role::Human)
    }

    /// A session with any configuration and client role.
    pub fn with(config: EngineConfig, role: Role) -> Self {
        let engine = Engine::new(config).unwrap_or_else(|e| panic!("engine did not start: {e}"));
        let origin = Origin {
            client_id: 1,
            client: "test".into(),
            role,
        };
        Self {
            engine,
            origin,
            transcript: None,
        }
    }

    /// This session, recording every call made through [`Session::call`] and everything
    /// built on it (not [`Session::request`] or [`Session::batch`]).
    #[must_use]
    pub fn recording(mut self) -> Self {
        self.transcript = Some(Vec::new());
        self
    }

    /// The calls recorded so far, oldest first.
    pub fn transcript(&self) -> &[Exchange] {
        self.transcript.as_deref().unwrap_or_default()
    }

    /// Takes the calls recorded so far, oldest first, and records on from empty: a long run
    /// hands its record on as it goes instead of holding it whole.
    pub fn take_transcript(&mut self) -> Vec<Exchange> {
        self.transcript
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Runs one command and returns the raw response.
    pub fn call(&mut self, command: &str, args: Value) -> CmdResponse {
        let params = RunParams::new(command, args);
        let response = self
            .engine
            .execute(&self.origin, Request::Run(params.clone()));
        if let Some(transcript) = &mut self.transcript {
            transcript.push(Exchange {
                command: params.command,
                args: params.args,
                response: serde_json::to_string(&response).unwrap_or_default(),
            });
        }
        response
    }

    /// Sends any request and returns the raw response.
    pub fn request(&mut self, request: Request) -> CmdResponse {
        self.engine.execute(&self.origin, request)
    }

    /// Runs commands as one batch (one undo step).
    pub fn batch(&mut self, steps: &[(&str, Value)]) -> CmdResponse {
        let commands = steps
            .iter()
            .map(|(command, args)| Step {
                command: (*command).into(),
                args: args.clone(),
            })
            .collect();
        self.request(Request::Batch(BatchParams {
            commands,
            if_rev: None,
        }))
    }

    /// Runs one command: its success, or its error.
    pub fn run(&mut self, command: &str, args: Value) -> Result<Success, CommandError> {
        match self.call(command, args).outcome {
            Outcome::Ok(success) => Ok(success),
            Outcome::Error(err) => Err(err),
        }
    }

    /// Runs a command that must succeed.
    pub fn ok(&mut self, command: &str, args: Value) -> Success {
        success(self.call(command, args), command)
    }

    /// Runs a command that must fail, and checks it left the scene exactly as it was.
    pub fn err(&mut self, command: &str, args: Value) -> CommandError {
        let before = self.hash();
        let err = failure(self.call(command, args), command);
        assert_eq!(
            self.hash(),
            before,
            "{command} failed but changed the scene"
        );
        err
    }

    /// The result of a command that must succeed.
    pub fn query(&mut self, command: &str, args: Value) -> Value {
        self.ok(command, args).result
    }

    /// The engine's hash of the whole scene (`mesh.hash`).
    pub fn hash(&mut self) -> String {
        text(&self.query("mesh.hash", json!({}))["hash"])
    }

    /// A part's mesh data (`part.get`), without the Live Boolean marks and Show Paint switch it
    /// gives beside it.
    pub fn mesh(&mut self, part: &str) -> PlainMesh {
        let mut value = self.query("part.get", json!({ "part": part }));
        if let Some(fields) = value.as_object_mut() {
            fields.remove("operation");
            fields.remove("start");
            fields.remove("paint_visible");
        }
        serde_json::from_value(value).unwrap_or_else(|e| panic!("part.get gave bad data: {e}"))
    }

    /// Every part, read back through `mesh.stats` and `part.get`.
    pub fn scene(&mut self) -> Scene {
        let stats = self.query("mesh.stats", json!({}));
        let names: Vec<String> = stats["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| text(&p["part"]))
            .collect();
        names
            .into_iter()
            .map(|name| (name.clone(), self.mesh(&name)))
            .collect()
    }

    /// Adds a part made from plain mesh data (`part.create`).
    pub fn add_part(&mut self, name: &str, mesh: &PlainMesh) {
        self.ok("part.create", json!({ "name": name, "mesh": mesh }));
    }

    /// The history as `history.list` reports it.
    pub fn history(&mut self) -> Value {
        self.query("history.list", json!({ "limit": 1000 }))
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// The success in a response, or a panic naming `what` failed and why.
pub fn success(response: CmdResponse, what: &str) -> Success {
    match response.outcome {
        Outcome::Ok(success) => success,
        Outcome::Error(err) => panic!("{what} failed: {} {}", code(err.code), err.message),
    }
}

/// The error in a response, or a panic saying `what` should have failed.
pub fn failure(response: CmdResponse, what: &str) -> CommandError {
    match response.outcome {
        Outcome::Ok(success) => panic!("{what} succeeded but should fail: {}", success.result),
        Outcome::Error(err) => err,
    }
}

/// The wire name of an error code, such as `E_NOT_FOUND`.
pub fn code(code: ErrorCode) -> String {
    text(&json!(code))
}

/// A JSON string's text, or a panic if the value is not a string.
pub fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected text, got {value}"))
        .to_string()
}
