//! Short text output for agents: one `path: value` line per value, so output is easy to read
//! and to grep. `--json` prints the full response instead.
//!
//! The command's own result is under `result.`, so a result field never looks like one of the
//! engine's lines (`effect.`, `warning:`). Text that would read as JSON (`123`, `true`, `null`,
//! `"quoted"`) or is empty is printed JSON-quoted, so the text `123` and the number 123 differ.

use gs_protocol::{CmdResponse, CommandError, Outcome};
use serde_json::Value;

use crate::failure::Failure;

/// Who reads the output: someone at the command line, or an agent through MCP. Only the hints
/// that name the next thing to run differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Cli,
    Mcp,
}

/// A job that is still running, and how to wait for it or cancel it.
pub fn running(report: &Value, surface: Surface) -> Vec<String> {
    let id = &report["id"];
    let command = report["command"].as_str().unwrap_or_default();
    let done = (report["progress"].as_f64()).map(|done| format!(", {:.0}% done", done * 100.0));
    let message = report["message"].as_str().map(|m| format!(", {m}"));
    let status = format!(
        "running: job {id} ({command}){}{}",
        done.unwrap_or_default(),
        message.unwrap_or_default()
    );
    let (wait, cancel) = match surface {
        Surface::Cli => (
            format!("gs job {id} --wait-ms 60000"),
            format!("gs job {id} --cancel"),
        ),
        Surface::Mcp => (
            format!(r#"gs_job {{"id": {id}, "wait_ms": 60000}}"#),
            format!(r#"gs_job {{"id": {id}, "cancel": true}}"#),
        ),
    };
    vec![
        status,
        format!("wait for it: {wait}"),
        format!("cancel it: {cancel}"),
    ]
}

/// Prints the response to a call of `command` (the whole envelope with `json`), and turns a
/// failed one into the failure.
pub fn report_of(
    command: &str,
    response: &Value,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    if json {
        out.push(response.to_string());
    }
    checked(response)?;
    if !json {
        out.extend(success(command, response));
    }
    Ok(())
}

/// [`report_of`] for a response whose command is not at hand (a log line replayed, which is
/// never a query): its result prints flattened.
pub fn report(response: &Value, json: bool, out: &mut Vec<String>) -> Result<(), Failure> {
    report_of("", response, json, out)
}

/// Turns a failed response into the failure.
pub fn checked(response: &Value) -> Result<(), Failure> {
    let parsed: CmdResponse = serde_json::from_value(response.clone())
        .map_err(|e| Failure::Protocol(format!("unexpected response: {e}")))?;
    match parsed.outcome {
        Outcome::Ok(_) => Ok(()),
        Outcome::Error(err) => Err(Failure::Command(Box::new(err))),
    }
}

/// A successful response to a call of `command` as text: a header line, then the effect, the
/// result and warnings, read from the response exactly as the engine sent it so numbers print
/// as the engine wrote them. The result prints in the command's own short form if it has one
/// ([`short_form`]), else flattened.
pub fn success(command: &str, response: &Value) -> Vec<String> {
    let ok = &response["ok"];
    let fields = [
        ("seq", &response["seq"]),
        ("rev", &response["rev"]),
        ("step", &ok["history_id"]),
        ("concurrent_changes", &response["concurrent_changes"]),
    ];
    let mut header = vec!["ok".to_string()];
    header.extend(
        fields
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| format!("{k}={v}")),
    );
    let mut lines = vec![header.join(" ")];
    if let Some(effect) = ok.get("effect") {
        flatten("effect", effect, &mut lines);
    }
    match short_form(command) {
        Some(form) => lines.extend(form(&ok["result"])),
        None => flatten("result", &ok["result"], &mut lines),
    }
    let warnings = ok["warnings"].as_array().into_iter().flatten();
    lines.extend(warnings.map(|w| format!("warning: {}", w.as_str().unwrap_or_default())));
    lines
}

/// The commands whose results print in a short form of their own instead of one line per
/// value. `--json` still gives every field.
fn short_form(command: &str) -> Option<fn(&Value) -> Vec<String>> {
    match command {
        "log.tail" => Some(log_tail),
        _ => None,
    }
}

/// `log.tail`: one line per entry, oldest first, with its sequence number, the revision after
/// it, the client and its role, the commands it ran and `ok` or the error code, as in
/// `seq=4 rev=3 sculptor#2 (agent) primitive.cube, part.create ok`.
fn log_tail(result: &Value) -> Vec<String> {
    let word = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
    let entries = result["entries"].as_array().map_or(&[][..], Vec::as_slice);
    if entries.is_empty() {
        return vec!["no log entries".into()];
    }
    (entries.iter())
        .map(|e| {
            let commands: Vec<String> = (e["commands"].as_array().into_iter().flatten())
                .map(word)
                .collect();
            format!(
                "seq={} rev={} {} ({}) {} {}",
                e["seq"],
                e["rev"],
                word(&e["client"]),
                word(&e["role"]),
                commands.join(", "),
                word(&e["status"])
            )
        })
        .collect()
}

/// Any failure as text: a command error with every detail, or one `error:` line.
pub fn failure(failure: &Failure) -> Vec<String> {
    match failure {
        Failure::Command(err) => error(err),
        Failure::Script { line, failure } => {
            let mut lines = self::failure(failure);
            if let Some(first) = lines.first_mut() {
                *first = format!("{line}: {first}");
            }
            lines
        }
        Failure::Usage(text)
        | Failure::Unreachable(text)
        | Failure::Timeout(text)
        | Failure::Protocol(text)
        | Failure::Refused(text)
        | Failure::Diverged(text) => vec![format!("error: {text}")],
    }
}

/// A command error as text.
pub fn error(err: &CommandError) -> Vec<String> {
    let code = serde_json::to_value(err.code).unwrap_or_default();
    let at = err
        .path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(|path| format!(" at {path}"))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "error {}{at}: {}",
        code.as_str().unwrap_or_default(),
        err.message
    )];
    lines.extend(
        err.violations
            .iter()
            .map(|v| format!("  {}: {}", v.path, v.message)),
    );
    lines.extend(err.hint.iter().map(|hint| format!("hint: {hint}")));
    if !err.suggestions.is_empty() {
        lines.push(format!("did you mean: {}", err.suggestions.join(", ")));
    }
    lines.extend(
        err.example
            .iter()
            .map(|example| format!("example: {example}")),
    );
    lines
}

/// Writes `value` as `path: value` lines. Objects add `.key` to the path and arrays `[i]`;
/// arrays of plain values stay on one line as JSON. Text with line breaks follows its path on
/// lines of its own, indented by two spaces.
pub fn flatten(path: &str, value: &Value, lines: &mut Vec<String>) {
    let label = if path.is_empty() {
        String::new()
    } else {
        format!("{path}: ")
    };
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, item) in map {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                flatten(&child, item, lines);
            }
        }
        Value::Array(items) if items.iter().any(|i| i.is_object() || i.is_array()) => {
            for (i, item) in items.iter().enumerate() {
                flatten(&format!("{path}[{i}]"), item, lines);
            }
        }
        Value::String(text) if text.contains('\n') => {
            lines.push(label.trim_end().to_string());
            lines.extend(text.lines().map(|line| format!("  {line}")));
        }
        Value::String(text) if reads_as_other(text) => lines.push(format!("{label}{value}")),
        Value::String(text) => lines.push(format!("{label}{text}")),
        other => lines.push(format!("{label}{other}")),
    }
}

/// True if `text` printed bare would read as something other than this text: empty, or valid
/// JSON (a number, `true`, or text that is itself quoted).
fn reads_as_other(text: &str) -> bool {
    text.is_empty() || serde_json::from_str::<Value>(text).is_ok()
}
