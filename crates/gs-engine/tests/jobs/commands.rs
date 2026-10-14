//! Test-only long commands, registered in this test program only, so their slowness never
//! reaches another test's catalog.

use std::thread;
use std::time::Duration;

use gs_core::Vec3;
use gs_engine::errors::error;
use gs_engine::{Command, Ctx, Example, PartArg, register_command};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Waits `steps` times `step_ms`, reporting progress after each wait, then moves every vertex
/// of the part by `offset` along x, in place. Stops early when cancelled, unless
/// `ignore_cancel` is set.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestSlow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    pub steps: u32,
    pub step_ms: u64,
    pub offset: f32,
    #[serde(default)]
    pub ignore_cancel: bool,
}

impl Command for TestSlow {
    const NAME: &'static str = "test.slow";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    type Output = u32;

    fn examples() -> Vec<Example<Self>> {
        Vec::new()
    }

    fn run(self, cx: &mut Ctx) -> Result<u32, CommandError> {
        for step in 1..=self.steps {
            if cx.cancelled() && !self.ignore_cancel {
                return Err(error(ErrorCode::Cancelled, "stopped"));
            }
            thread::sleep(Duration::from_millis(self.step_ms));
            let message = format!("step {step} of {}", self.steps);
            cx.progress(Some(step as f32 / self.steps as f32), message);
        }
        let name = self.part.map(|p| p.0).unwrap_or_default();
        let mut part = cx.doc.write_part(&name)?;
        let nodes = part.all_nodes();
        let offset = Vec3::new(self.offset, 0.0, 0.0);
        part.write_positions(&nodes).each(|node| {
            for p in node.positions() {
                *p += offset;
            }
        });
        Ok(self.steps)
    }
}
register_command!(TestSlow);

/// A long session command: never a job, since a job only runs edits and queries.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestLongSession {}

impl Command for TestLongSession {
    const NAME: &'static str = "test.long_session";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Long;
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        Vec::new()
    }

    fn run(self, _cx: &mut Ctx) -> Result<(), CommandError> {
        Ok(())
    }
}
register_command!(TestLongSession);
