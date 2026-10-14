//! `commands.search`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Command, Ctx, Example, register_command};

/// Find commands whose name or docs contain every word of a query. A command named exactly the
/// query comes first, then commands with every word in their name, then by name.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandsSearch {
    /// Words to look for, for example "undo" or "sphere". Case does not matter.
    pub query: String,
    /// Most matches to return. Left out, 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<u32>,
}

/// Matching commands, best first.
#[derive(Serialize, JsonSchema)]
pub struct SearchOutput {
    /// The matches.
    pub matches: Vec<Match>,
}

/// One matching command.
#[derive(Serialize, JsonSchema)]
pub struct Match {
    /// Command name.
    pub name: String,
    /// What it may do.
    pub kind: Kind,
    /// First line of its docs.
    pub summary: String,
}

impl Command for CommandsSearch {
    const NAME: &'static str = "commands.search";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = SearchOutput;

    fn examples() -> Vec<Example<Self>> {
        let query = "undo".into();
        vec![Example::new(
            "find the undo commands",
            Self { query, limit: None },
        )]
    }

    fn run(self, cx: &mut Ctx) -> Result<SearchOutput, CommandError> {
        let query = self.query.to_lowercase();
        let words: Vec<&str> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let mut found: Vec<(u8, &gs_protocol::CommandInfo)> = cx
            .catalog
            .commands
            .iter()
            .filter_map(|info| {
                let (name, docs) = (info.name.to_lowercase(), info.docs.to_lowercase());
                let in_name = words.iter().all(|w| name.contains(w));
                let anywhere = words.iter().all(|w| name.contains(w) || docs.contains(w));
                let rank = if name == query {
                    0
                } else {
                    1 + u8::from(!in_name)
                };
                anywhere.then_some((rank, info))
            })
            .collect();
        found.sort_by_key(|(rank, info)| (*rank, info.name.clone()));
        let matches = found
            .into_iter()
            .take(self.limit.unwrap_or(10) as usize)
            .map(|(_, info)| Match {
                name: info.name.clone(),
                kind: info.kind,
                summary: info.summary.clone(),
            })
            .collect();
        Ok(SearchOutput { matches })
    }
}
register_command!(CommandsSearch, "Search commands");
