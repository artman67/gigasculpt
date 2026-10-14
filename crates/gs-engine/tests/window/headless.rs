//! A session with no window: pen input moves the session's cursor and nothing else, a human's
//! touch holds agents back, and the commands that need a window say so.

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::json;

use gs_testkit::session::{Session, code};

use crate::As;

fn sample(x: f64, contact: bool) -> serde_json::Value {
    json!({ "x": x, "y": 10.0, "contact": contact, "pressure": if contact { 0.5 } else { 0.0 } })
}

#[test]
fn pen_input_moves_the_cursor_and_leaving_range_clears_it() {
    let mut s = Session::with(EngineConfig::default(), Role::Human);
    let out = s.by(Role::Agent).query(
        "pen.input",
        json!({ "samples": [sample(1.0, false), sample(2.0, true)] }),
    );
    assert_eq!(out["accepted"], json!(2));
    assert_eq!(out["window"], json!(false));
    assert_eq!(out["cursor"]["x"], json!(2.0), "the last sample");
    assert_eq!(
        out["cursor"]["in_proximity"],
        json!(true),
        "defaults to in range"
    );
    let seen = s.by(Role::Observer).query("ui.describe", json!({}));
    assert_eq!(seen["window"], json!(false));
    assert_eq!(
        seen["cursor"]["x"],
        json!(2.0),
        "describe sees the same cursor"
    );
    assert_eq!(seen["panels"], json!([]));
    let away = json!({ "samples": [{ "x": 2.0, "y": 10.0, "in_proximity": false }] });
    let out = s.by(Role::Agent).query("pen.input", away);
    assert_eq!(out["cursor"], json!(null), "out of range: no cursor");
    assert_eq!(
        s.by(Role::Agent).query("ui.describe", json!({}))["cursor"],
        json!(null)
    );
}

#[test]
fn a_humans_touch_holds_agent_edits_and_an_agents_does_not() {
    let mut s = Session::with(EngineConfig::default(), Role::Human);
    s.by(Role::Human)
        .query("pen.input", json!({ "samples": [sample(1.0, true)] }));
    let busy = s.by(Role::Agent).err("primitive.sphere", json!({}));
    assert_eq!(code(busy.code), "E_BUSY", "the pen is down");
    assert!(busy.retryable, "agents retry");
    let state = s.by(Role::Agent).query("session.state", json!({}));
    assert_eq!(state["session.gesture"], json!(true));
    s.by(Role::Human)
        .query("pen.input", json!({ "samples": [sample(1.0, false)] }));
    s.by(Role::Agent).query("primitive.sphere", json!({}));
    s.by(Role::Agent)
        .query("pen.input", json!({ "samples": [sample(1.0, true)] }));
    s.by(Role::Agent).query("primitive.cube", json!({}));
    let state = s.by(Role::Agent).query("session.state", json!({}));
    assert_eq!(
        state["session.gesture"],
        json!(false),
        "an agent's pen is not a human gesture"
    );
}

#[test]
fn window_commands_say_there_is_no_window() {
    let mut s = Session::with(EngineConfig::default(), Role::Human);
    for (command, args) in [
        ("ui.screenshot", json!({})),
        ("ui.palette", json!({ "query": "sphere" })),
    ] {
        let e = s.by(Role::Human).err(command, args);
        assert_eq!(code(e.code), "E_PRECONDITION", "{command}");
        assert_eq!(e.message, "this session has no window", "{command}");
        let hint = e.hint.unwrap_or_default();
        assert!(hint.contains("gigasculpt"), "{command} says how: {hint}");
    }
}

#[test]
fn bad_samples_are_refused_with_their_path() {
    let mut s = Session::with(EngineConfig::default(), Role::Human);
    let too_hard = json!({ "samples": [{ "x": 1.0, "y": 1.0, "pressure": 2.0 }] });
    let e = s.by(Role::Agent).err("pen.input", too_hard);
    assert_eq!(code(e.code), "E_ARGS_INVALID");
    assert_eq!(e.path.as_deref(), Some("/samples/0/pressure"));
    let e = s.by(Role::Agent).err("pen.input", json!({ "samples": [] }));
    assert_eq!(code(e.code), "E_ARGS_INVALID", "at least one sample");
    let many = vec![json!({ "x": 0.0, "y": 0.0 }); 10_001];
    let e = s
        .by(Role::Agent)
        .err("pen.input", json!({ "samples": many }));
    assert_eq!(code(e.code), "E_ARGS_INVALID", "at most 10,000");
    let ten_thousand = vec![json!({ "x": 0.0, "y": 0.0 }); 10_000];
    let out = s
        .by(Role::Agent)
        .query("pen.input", json!({ "samples": ten_thousand }));
    assert_eq!(out["accepted"], json!(10_000));
}

#[test]
fn pen_input_is_written_to_the_log_and_replays() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("window-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let log = dir.join("pen.jsonl");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.by(Role::Agent)
        .query("pen.input", json!({ "samples": [sample(7.0, true)] }));
    s.by(Role::Agent).query("ui.describe", json!({}));
    drop(s);
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("{e}"));
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("\"command\"")).collect();
    assert_eq!(
        lines.len(),
        1,
        "pen.input is logged, the query is not: {text}"
    );
    assert!(
        lines[0].contains("pen.input") && lines[0].contains("\"x\":7.0"),
        "with its samples, resolved: {}",
        lines[0]
    );
    assert!(lines[0].contains("\"tilt_x\":0.0"), "defaults filled in");
}
