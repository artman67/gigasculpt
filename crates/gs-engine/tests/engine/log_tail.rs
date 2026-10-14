//! `log.tail`: the last calls in the session's command log, oldest first, with who ran them,
//! filtered by client or command prefix, and reading the log never changes it or the scene.
//!
//! Each entry carries `seq`, `rev` (after the call, as its response reported), `client`,
//! `role`, `commands` (every command name the call ran, so a batch lists its steps) and
//! `status` (`"ok"` or the error code).

use gs_engine::{EngineConfig, Origin};
use gs_protocol::{CmdResponse, Role};
use gs_testkit::{Session, code, success, text};
use serde_json::{Value, json};

use crate::support::agent;

/// Makes the calls that follow come from `client` on connection `id`.
fn as_client(s: &mut Session, id: u64, client: &str, role: Role) {
    s.origin = Origin {
        client_id: id,
        client: client.into(),
        role,
    };
}

/// The entries of a `log.tail` result.
fn entries(result: &Value) -> Vec<Value> {
    result["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("log.tail gave no entries list: {result}"))
        .clone()
}

/// An entry's client name, with or without the `#connection` suffix the history uses.
fn client(entry: &Value) -> String {
    let label = text(&entry["client"]);
    label.split('#').next().unwrap_or_default().to_string()
}

/// What the log should say about a call: its sequence number, the revision after it, who ran
/// it and which commands it ran.
fn expected(response: &CmdResponse, client: &str, role: &str, commands: &[&str]) -> Value {
    json!({
        "seq": response.seq,
        "rev": response.rev,
        "client": client,
        "role": role,
        "commands": commands,
        "status": "ok",
    })
}

/// The same fields read from an entry, with the client name stripped of its connection.
fn observed(entry: &Value) -> Value {
    json!({
        "seq": entry["seq"],
        "rev": entry["rev"],
        "client": client(entry),
        "role": entry["role"],
        "commands": entry["commands"],
        "status": entry["status"],
    })
}

/// Three edits from two clients: an agent's sphere, the human's jitter, then the agent's
/// batch of two commands.
fn three_edits(s: &mut Session) -> [Value; 3] {
    as_client(s, 2, "sculptor", Role::Agent);
    let sphere = s.call("primitive.sphere", json!({ "name": "head" }));
    let first = expected(&sphere, "sculptor", "agent", &["primitive.sphere"]);
    success(sphere, "sphere");
    as_client(s, 3, "painter", Role::Human);
    let jitter = s.call("mesh.jitter", json!({ "amount": "1%", "seed": 4 }));
    let second = expected(&jitter, "painter", "human", &["mesh.jitter"]);
    success(jitter, "jitter");
    as_client(s, 2, "sculptor", Role::Agent);
    let batch = s.batch(&[
        ("primitive.cube", json!({ "name": "box" })),
        (
            "part.create",
            json!({ "name": "empty", "mesh": { "positions": [], "faces": [] } }),
        ),
    ]);
    let third = expected(
        &batch,
        "sculptor",
        "agent",
        &["primitive.cube", "part.create"],
    );
    success(batch, "batch");
    [first, second, third]
}

#[test]
fn the_last_entries_come_oldest_first_with_who_ran_them() {
    let mut s = agent();
    let [_, second, third] = three_edits(&mut s);
    let tail = entries(&s.query("log.tail", json!({ "n": 2 })));
    let got: Vec<Value> = tail.iter().map(observed).collect();
    assert_eq!(got, [second, third], "the last two calls, oldest first");
}

#[test]
fn filters_keep_only_the_matching_entries() {
    let mut s = agent();
    let [first, second, third] = three_edits(&mut s);
    let sculptor = entries(&s.query("log.tail", json!({ "n": 2, "client": "sculptor" })));
    let got: Vec<Value> = sculptor.iter().map(observed).collect();
    assert_eq!(
        got,
        [first.clone(), third.clone()],
        "the filter picks the entries before n counts them"
    );
    let painter = entries(&s.query("log.tail", json!({ "client": "painter" })));
    let got: Vec<Value> = painter.iter().map(observed).collect();
    assert_eq!(
        got,
        std::slice::from_ref(&second),
        "one client's entries only"
    );
    let meshes = entries(&s.query("log.tail", json!({ "prefix": "mesh." })));
    let got: Vec<Value> = meshes.iter().map(observed).collect();
    assert_eq!(
        got,
        std::slice::from_ref(&second),
        "only calls that ran a mesh. command"
    );
    let nobody = entries(&s.query("log.tail", json!({ "client": "nobody" })));
    assert!(nobody.is_empty(), "an unknown client has no entries");
    let all = entries(&s.query("log.tail", json!({})));
    let got: Vec<Value> = all.iter().map(observed).collect();
    assert_eq!(got, [first, second, third], "no filter: every call");
}

#[test]
fn n_defaults_to_20_and_stops_at_500() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let mut last = 0;
    for _ in 0..24 {
        let moved = s.call(
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 0.1 }),
        );
        last = moved.seq.unwrap_or_default();
        success(moved, "translate");
    }
    let tail = entries(&s.query("log.tail", json!({})));
    assert_eq!(tail.len(), 20, "20 entries by default");
    assert_eq!(tail[19]["seq"], json!(last), "ending with the newest");
    let seqs: Vec<u64> = tail.iter().filter_map(|e| e["seq"].as_u64()).collect();
    assert!(
        seqs.len() == 20 && seqs.windows(2).all(|w| w[0] < w[1]),
        "oldest first: {seqs:?}"
    );
    let every = entries(&s.query("log.tail", json!({ "n": 500 })));
    assert_eq!(every.len(), 25, "500 is allowed and shows the whole log");
    let err = s.err("log.tail", json!({ "n": 501 }));
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        ("E_ARGS_INVALID".to_string(), Some("/n")),
        "more than 500 is refused at /n"
    );
}

#[test]
fn reading_the_log_changes_nothing() {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("log-tail-{}.jsonl", std::process::id()));
    let config = EngineConfig {
        log_path: Some(path.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    three_edits(&mut s);
    let file = std::fs::read(&path).unwrap_or_default();
    let history = s.history();
    let hash = s.hash();
    let first = s.call("log.tail", json!({ "n": 500 }));
    let rev = first.rev;
    let first = success(first, "log.tail");
    let second = s.call("log.tail", json!({ "n": 500 }));
    assert_eq!(second.rev, rev, "the revision stays");
    let second = success(second, "log.tail");
    assert_eq!(
        first.result, second.result,
        "log.tail adds nothing to the log"
    );
    assert_eq!(first.history_id, None, "no undo step");
    assert_eq!(s.hash(), hash, "the scene is unchanged");
    assert_eq!(s.history(), history, "the history is unchanged");
    assert_eq!(
        std::fs::read(&path).unwrap_or_default(),
        file,
        "the log file is unchanged"
    );
    std::fs::remove_file(path).ok();
}
