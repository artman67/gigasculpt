//! `session.info`.

use gs_protocol::{CommandError, Cost, Kind, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Command, Ctx, Example, register_command};

/// Facts about this session: engine version, seed, log file, history budget, clients. What can
/// change during it (the brush, the layout, whether agents are paused) is in `session.state`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {}

/// Session facts.
#[derive(Serialize, JsonSchema)]
pub struct Info {
    /// Session name.
    pub session: String,
    /// Engine version.
    pub engine_version: String,
    /// Wire protocol version.
    pub protocol: u32,
    /// Hash of the command catalog.
    pub catalog_hash: String,
    /// Number of commands.
    pub commands: u64,
    /// Seed of every random number in the session.
    pub seed: u64,
    /// Sequence number of this call.
    pub seq: u64,
    /// Document revision.
    pub rev: u64,
    /// Clients connected to the session, counting this one.
    pub clients: u64,
    /// The command log file, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// Undo history byte budget.
    pub history_budget_bytes: u64,
}

impl Command for SessionInfo {
    const NAME: &'static str = "session.info";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Info;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("show the session facts", Self {})]
    }

    fn run(self, cx: &mut Ctx) -> Result<Info, CommandError> {
        Ok(Info {
            session: cx.config.session.clone(),
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            catalog_hash: cx.catalog.hash.clone(),
            commands: cx.catalog.commands.len() as u64,
            seed: cx.config.seed,
            seq: cx.seq,
            rev: cx.rev,
            clients: cx.clients,
            log_path: cx.config.log_path.as_ref().map(|p| p.display().to_string()),
            history_budget_bytes: cx.history.budget_bytes(),
        })
    }
}
register_command!(SessionInfo, "Session info");
