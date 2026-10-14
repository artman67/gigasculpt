//! The material palette (decision 0043), in sessions without a renderer, by step: the palette
//! itself (`palette`: listing and choosing, Reference Views following a material by id, its own
//! undo, the agent rule and the log replaying it), copies and edits (`edits`), the later
//! built-ins and Flat Color (`builtins`), the cavity keys (`cavity`) and the user folder
//! (`folder`). The helpers here are theirs.

mod builtins;
mod cavity;
mod edits;
mod folder;
mod palette;

use gs_testkit::{Session, code};
use serde_json::{Value, json};

/// `material.list`'s names in palette order.
fn names(s: &mut Session) -> Vec<String> {
    let list = s.query("material.list", json!({}));
    (list["looks"].as_array().into_iter().flatten())
        .filter_map(|m| m["name"].as_str().map(str::to_string))
        .collect()
}

/// The active material, as `session.state` names it.
fn look(s: &mut Session) -> Value {
    s.query("session.state", json!({}))["look"].clone()
}

/// The error code of a call that must fail, and its message.
fn refused(s: &mut Session, command: &str, args: Value) -> (String, String) {
    let e = s.err(command, args);
    (code(e.code), e.message)
}

/// Material `place`'s values, as `material.list` reports them.
fn values(s: &mut Session, place: usize) -> Value {
    s.query("material.list", json!({}))["looks"][place]["values"].clone()
}

/// Every material's id, in palette order.
fn ids(s: &mut Session) -> Vec<u64> {
    let list = s.query("material.list", json!({}));
    (list["looks"].as_array().into_iter().flatten())
        .filter_map(|m| m["id"].as_u64())
        .collect()
}

/// A fresh, unused log path under the test target folder.
fn log_path(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("materials-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    dir.join(format!("{name}.jsonl"))
}

fn remove(log: &std::path::Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}
