//! `log.tail`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::LogLine;
use crate::{Command, Ctx, Example, register_command};

/// The last lines of the command log, oldest first: each call's sequence number, the revision
/// after it, who ran it and the commands it ran. The log records edits and session commands
/// that succeeded, not queries or failures.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogTail {
    /// How many lines to show, the newest last. Left out, 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 500))]
    pub n: Option<u32>,
    /// Only calls from the client with this name (without `#connection`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Only calls that ran a command whose name starts with this, for example `mesh.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

/// The lines that matched.
#[derive(Serialize, JsonSchema)]
pub struct TailOutput {
    /// The last `n` matching lines, oldest first.
    pub entries: Vec<LogLine>,
}

impl LogTail {
    fn matches(&self, line: &LogLine) -> bool {
        let client = (self.client.as_deref()).is_none_or(|name| line.client_name() == name);
        let prefix = (self.prefix.as_deref())
            .is_none_or(|prefix| line.commands.iter().any(|c| c.starts_with(prefix)));
        client && prefix
    }
}

impl Command for LogTail {
    const NAME: &'static str = "log.tail";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = TailOutput;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "the last 5 calls that ran a mesh command",
                Self {
                    n: Some(5),
                    client: None,
                    prefix: Some("mesh.".into()),
                },
            )
            .after("primitive.sphere", json!({}))
            .after("mesh.jitter", json!({ "amount": "1%" })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<TailOutput, CommandError> {
        let matching: Vec<&LogLine> = cx.log.iter().filter(|line| self.matches(line)).collect();
        let skip = matching.len().saturating_sub(self.n.unwrap_or(20) as usize);
        Ok(TailOutput {
            entries: matching[skip..].iter().map(|&line| line.clone()).collect(),
        })
    }
}
register_command!(LogTail, "Read command log");
