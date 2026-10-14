//! `history.list`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Command, Ctx, Example, register_command};

/// List the undo steps, newest first, and the saved checkpoints.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryList {
    /// How many steps to show, newest first. Left out, 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<u32>,
}

/// The history.
#[derive(Serialize, JsonSchema)]
pub struct HistoryOutput {
    /// Steps, newest first.
    pub steps: Vec<StepInfo>,
    /// Steps that can be undone.
    pub undoable: u64,
    /// Steps that can be redone.
    pub redoable: u64,
    /// Bytes the steps hold that the scene does not, each allocation counted once. The budget
    /// limits this.
    pub bytes: u64,
    /// The byte budget; the oldest steps are dropped beyond it.
    pub budget_bytes: u64,
    /// Bytes checkpoints keep alive that neither the scene nor a step holds. Not limited.
    pub checkpoint_bytes: u64,
    /// Saved checkpoints, by name.
    pub checkpoints: Vec<CheckpointInfo>,
}

/// One undo step.
#[derive(Serialize, JsonSchema)]
pub struct StepInfo {
    /// The step id. Steps are numbered 1, 2, 3... in the order they were made.
    pub id: u64,
    /// The command (`cmd.batch` for a batch).
    pub command: String,
    /// Its resolved arguments.
    pub args: Value,
    /// The client that ran it, as `name#connection`.
    pub client: String,
    /// Bytes this step holds that no newer step and not the scene holds: what dropping it and
    /// every older step frees. The steps' bytes add up to `bytes`.
    pub bytes: u64,
    /// False if the step is undone and can be redone.
    pub done: bool,
}

/// One checkpoint.
#[derive(Serialize, JsonSchema)]
pub struct CheckpointInfo {
    /// Its name.
    pub name: String,
    /// Sequence number of the call that saved it.
    pub seq: u64,
}

impl Command for HistoryList {
    const NAME: &'static str = "history.list";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = HistoryOutput;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("show the last 5 steps", Self { limit: Some(5) })
                .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<HistoryOutput, CommandError> {
        let listing = cx.history.listing(&cx.doc);
        let steps = (listing.steps.into_iter().rev())
            .take(self.limit.unwrap_or(20) as usize)
            .map(|row| StepInfo {
                id: row.id,
                command: row.command,
                args: (*row.args).clone(),
                client: row.client,
                bytes: row.bytes,
                done: row.done,
            })
            .collect();
        let checkpoints = listing.checkpoints.into_iter();
        Ok(HistoryOutput {
            steps,
            undoable: listing.undoable as u64,
            redoable: listing.redoable as u64,
            bytes: listing.bytes,
            budget_bytes: listing.budget_bytes,
            checkpoint_bytes: listing.checkpoint_bytes,
            checkpoints: checkpoints
                .map(|(name, seq)| CheckpointInfo { name, seq })
                .collect(),
        })
    }
}
register_command!(HistoryList, "List history");
