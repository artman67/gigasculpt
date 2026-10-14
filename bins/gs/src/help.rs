//! Finding and describing commands from the engine's catalog: `gs commands` and the `gs_help`
//! tool.

use gs_protocol::CommandInfo;
use serde_json::{Value, json};

use crate::connection::Connection;
use crate::failure::Failure;
use crate::output::checked;

/// One line per command matching `query` (every command when it is empty), best first.
pub fn list(connection: &mut Connection, query: &str) -> Result<Vec<String>, Failure> {
    let catalog = connection.catalog()?;
    let names: Vec<String> = if query.trim().is_empty() {
        let all = catalog.commands.iter();
        all.map(|info| info.name.clone()).collect()
    } else {
        let response =
            connection.run("commands.search", json!({ "query": query, "limit": 1000 }))?;
        checked(&response)?;
        let matches = response["ok"]["result"]["matches"].as_array().cloned();
        matches
            .into_iter()
            .flatten()
            .filter_map(|m| m["name"].as_str().map(str::to_string))
            .collect()
    };
    Ok(names
        .iter()
        .filter_map(|name| catalog.commands.iter().find(|info| &info.name == name))
        .map(line)
        .collect())
}

/// Everything about one command: its docs, argument schema and examples. An unknown name is
/// the engine's `E_NOT_FOUND`, with the closest names.
pub fn describe(connection: &mut Connection, name: &str) -> Result<Vec<String>, Failure> {
    let response = connection.run("commands.describe", json!({ "name": name }))?;
    checked(&response)?;
    let info: CommandInfo = serde_json::from_value(response["ok"]["result"].clone())
        .map_err(|e| Failure::Protocol(format!("unexpected command description: {e}")))?;
    let mut lines = vec![line(&info)];
    lines.extend(info.docs.lines().skip(1).map(str::to_string));
    // The schema's own description is the docs above.
    let mut schema = info.args_schema.clone();
    if let Some(schema) = schema.as_object_mut() {
        schema.remove("description");
    }
    lines.push(format!("arguments (JSON Schema): {schema}"));
    if !info.pinned.is_empty() {
        lines.push(format!(
            "filled in when left out: {}",
            info.pinned.join(", ")
        ));
    }
    for example in &info.examples {
        lines.push(format!("example: {}", example.summary));
        for step in &example.setup {
            let first = json!({ "command": step.command, "args": step.args });
            lines.push(format!("  after: {first}"));
        }
        let run = json!({ "command": info.name, "args": example.args });
        lines.push(format!("  run: {run}"));
        if let Some(code) = example.fails_with {
            lines.push(format!("  fails with: {}", text(&json!(code))));
        }
    }
    Ok(lines)
}

/// A command in one line: name, kind, whether only the human may run it, and its summary.
fn line(info: &CommandInfo) -> String {
    let human = if info.human_only {
        "  [human only]"
    } else {
        ""
    };
    format!(
        "{}  ({}){human}  {}",
        info.name,
        text(&json!(info.kind)),
        info.summary
    )
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}
