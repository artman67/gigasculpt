//! Typed examples: a command's examples are values of the command's own type, so a renamed or
//! mistyped argument is a compile error, not a test failure. Tests run every example.

use gs_protocol::{ErrorCode, ExampleInfo, Step};
use serde::Serialize;
use serde_json::Value;

/// One example call of a command, with the calls that build its starting scene.
pub struct Example<T> {
    summary: &'static str,
    setup: Vec<Step>,
    args: Args<T>,
    fails_with: Option<ErrorCode>,
}

/// An example's arguments: typed, or raw JSON for what a typed value cannot show.
enum Args<T> {
    Typed(T),
    Raw(Value),
}

impl<T: Serialize> Example<T> {
    /// An example with typed arguments. Optional arguments left as `None` are left out.
    pub fn new(summary: &'static str, args: T) -> Self {
        Self::with(summary, Args::Typed(args))
    }

    /// An example with raw JSON arguments: for the minimal call that leaves every default out,
    /// or for a deliberately invalid call.
    pub fn raw(summary: &'static str, args: Value) -> Self {
        Self::with(summary, Args::Raw(args))
    }

    fn with(summary: &'static str, args: Args<T>) -> Self {
        Self {
            summary,
            setup: Vec::new(),
            args,
            fails_with: None,
        }
    }

    /// Runs `command` with `args` first, on top of the setup so far. Setup calls other
    /// commands, so it stays JSON.
    #[must_use]
    pub fn after(mut self, command: &str, args: Value) -> Self {
        self.setup.push(Step {
            command: command.to_string(),
            args,
        });
        self
    }

    /// The example shows a failure, and must fail with `code`.
    #[must_use]
    pub fn fails_with(mut self, code: ErrorCode) -> Self {
        self.fails_with = Some(code);
        self
    }

    /// The example as the catalog shows it.
    pub(crate) fn info(self) -> ExampleInfo {
        let args = match self.args {
            Args::Typed(args) => serde_json::to_value(args).unwrap_or_default(),
            Args::Raw(args) => args,
        };
        ExampleInfo {
            summary: self.summary.to_string(),
            setup: self.setup,
            args,
            fails_with: self.fails_with,
        }
    }
}
