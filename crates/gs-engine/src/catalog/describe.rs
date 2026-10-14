//! `commands.describe`.

use gs_protocol::{CommandError, CommandInfo, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Command, Ctx, Example, errors, register_command};

/// Everything about one command: docs, argument and result schemas, and examples.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandsDescribe {
    /// Command name, for example "primitive.sphere".
    pub name: String,
}

impl Command for CommandsDescribe {
    const NAME: &'static str = "commands.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = CommandInfo;

    fn examples() -> Vec<Example<Self>> {
        let name = "primitive.sphere".into();
        vec![Example::new("describe primitive.sphere", Self { name })]
    }

    fn run(self, cx: &mut Ctx) -> Result<CommandInfo, CommandError> {
        let commands = &cx.catalog.commands;
        commands
            .iter()
            .find(|info| info.name == self.name)
            .cloned()
            .ok_or_else(|| {
                errors::not_found(
                    "command",
                    &self.name,
                    commands.iter().map(|c| c.name.as_str()),
                )
            })
    }
}
register_command!(CommandsDescribe, "Describe command");
