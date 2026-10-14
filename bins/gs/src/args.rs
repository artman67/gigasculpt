//! Turning command-line words into a command's JSON arguments, using the command's schema from
//! the engine's catalog. Nothing here knows any particular command.
//!
//! Each argument can be given as a flag (`--radius 0.5`), as `key=value`, from a JSON file
//! (`@args.json`), or as an inline JSON object (`'{"radius": 0.5}'`). A value is read as JSON
//! when it parses (`3`, `true`, `[0,1,0]`), as numbers when it is a comma list (`0,1,0`), as
//! the contents of a file when it starts with `@`, and as text otherwise. Arguments whose schema
//! says text are always text.

use std::fs;

use clap::{Arg, ArgAction, Command};
use gs_protocol::CommandInfo;
use serde_json::{Map, Value};

use crate::failure::Failure;

/// The id of the `--json` flag, which may also follow a command's arguments.
const JSON: &str = "gs --json";

/// What parsing produced: arguments to send (and whether `--json` came after them), or help
/// text to print instead.
pub enum Parsed {
    Args(Value, bool),
    Help(String),
}

/// Parses `words` (everything after the command name) against the command's schema.
pub fn parse(info: &CommandInfo, words: Vec<String>) -> Result<Parsed, Failure> {
    let matches = match parser(info).try_get_matches_from(words) {
        Ok(matches) => matches,
        Err(err) if err.use_stderr() => return Err(Failure::Usage(err.to_string())),
        Err(err) => return Ok(Parsed::Help(err.to_string())),
    };
    let mut args = Map::new();
    for word in matches.get_many::<String>("pairs").into_iter().flatten() {
        match word.split_once('=') {
            Some((key, value)) if !word.starts_with(['@', '{']) => {
                args.insert(key.to_string(), value_for(info, key, value)?);
            }
            _ => match read_json(word)? {
                Value::Object(object) => args.extend(object),
                _ => {
                    return Err(Failure::Usage(format!(
                        "'{word}' is not KEY=VALUE, @FILE.json or a JSON object"
                    )));
                }
            },
        }
    }
    for key in properties(info).map(|(key, _)| key) {
        if let Some(value) = matches.get_one::<String>(key) {
            args.insert(key.clone(), value_for(info, key, value)?);
        }
    }
    Ok(Parsed::Args(Value::Object(args), matches.get_flag(JSON)))
}

/// The clap command for one engine command, built from its schema.
fn parser(info: &CommandInfo) -> Command {
    let pairs = Arg::new("pairs")
        .num_args(0..)
        .value_name("KEY=VALUE | @FILE.json | JSON")
        .help("Arguments as key=value pairs, a JSON file, or a JSON object");
    let mut command = Command::new(format!("gs call {}", info.name))
        .no_binary_name(true)
        .about(info.summary.clone())
        .long_about(info.docs.clone())
        .arg(pairs)
        .arg(
            Arg::new(JSON)
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Print the full JSON response"),
        );
    for (key, schema) in properties(info) {
        let help = schema
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut arg = Arg::new(key.clone())
            .long(key.clone())
            .help(help.to_string());
        if allows(info, schema, "boolean") {
            arg = arg
                .num_args(0..=1)
                .default_missing_value("true")
                .action(ArgAction::Set);
        }
        command = command.arg(arg);
    }
    command
}

/// The command's argument names with their schemas.
fn properties(info: &CommandInfo) -> impl Iterator<Item = (&String, &Value)> {
    info.args_schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
}

/// Reads one value for argument `key`.
fn value_for(info: &CommandInfo, key: &str, text: &str) -> Result<Value, Failure> {
    let schema = info.args_schema.pointer(&format!("/properties/{key}"));
    if text.starts_with('@') {
        return read_json(text);
    }
    if schema.is_some_and(|s| allows(info, s, "string") && !allows(info, s, "number")) {
        return Ok(Value::String(text.to_string()));
    }
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let numbers: Option<Vec<Value>> = text
        .split(',')
        .map(|n| serde_json::from_str::<f64>(n.trim()).ok().map(Value::from))
        .collect();
    // A lone number already parsed as JSON above, so this is a comma list.
    match numbers {
        Some(numbers) => Ok(Value::Array(numbers)),
        None => Ok(Value::String(text.to_string())),
    }
}

/// Parses `text` as JSON, or the file it names after an `@`.
fn read_json(text: &str) -> Result<Value, Failure> {
    let contents = match text.strip_prefix('@') {
        Some(path) => fs::read_to_string(path)
            .map_err(|e| Failure::Usage(format!("cannot read {path}: {e}")))?,
        None => text.to_string(),
    };
    serde_json::from_str(&contents)
        .map_err(|e| Failure::Usage(format!("'{text}' is not valid JSON: {e}")))
}

/// True if the schema accepts JSON values of type `kind`, following `$ref`, `anyOf` and `oneOf`.
fn allows(info: &CommandInfo, schema: &Value, kind: &str) -> bool {
    if let Some(target) = schema.get("$ref").and_then(Value::as_str) {
        let target = info.args_schema.pointer(target.trim_start_matches('#'));
        return target.is_some_and(|t| allows(info, t, kind));
    }
    let typed = match schema.get("type") {
        Some(Value::String(t)) => t == kind,
        Some(Value::Array(types)) => types.iter().any(|t| t == kind),
        _ => false,
    };
    let branches = ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|k| schema.get(k).and_then(Value::as_array))
        .flatten();
    typed
        || branches
            .into_iter()
            .any(|branch| allows(info, branch, kind))
}
