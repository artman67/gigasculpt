//! Scripts and logs: `gs run` replays them and names the first line that goes wrong.

use std::fs;

use serde_json::{Value, json};

use crate::support::Env;

pub fn scripts_and_logs_replay() {
    let env = Env::new();
    env.spawn("first");
    let script = env.file(
        "script.jsonl",
        "{\"command\": \"primitive.sphere\", \"args\": {\"subdivisions\": 3}}\n\n\
         {\"command\": \"mesh.jitter\", \"args\": {\"amount\": \"1%\"}}\n\
         {\"command\": \"mesh.stats\"}\n",
    );
    let ran = env.ok(&["run", &script]);
    let marks: Vec<&str> = ran
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("# "))
        .collect();
    assert_eq!(
        marks,
        ["# primitive.sphere", "# mesh.jitter", "# mesh.stats"],
        "{}",
        ran.stdout
    );
    let as_json = env.file("one.jsonl", "{\"command\": \"mesh.stats\"}\n");
    let envelope = env.ok(&["--json", "run", &as_json]);
    let line = envelope
        .lines()
        .into_iter()
        .find(|l| l.starts_with('{'))
        .unwrap_or_default()
        .to_string();
    assert!(
        serde_json::from_str::<Value>(&line).is_ok_and(|v| v.get("ok").is_some()),
        "{line}"
    );
    env.spawn("second");
    let log = env.log_of("first");
    env.ok(&["-s", "second", "run", &log]);
    assert_eq!(
        env.hash("second"),
        env.hash("first"),
        "replaying the log rebuilds the scene"
    );
}

/// A bad line stops `gs run`, which names the script, the line and why.
pub fn bad_script_lines_are_named() {
    let env = Env::new();
    env.spawn("second");
    env.ok(&["-s", "second", "call", "primitive.cube"]);
    let log = env.log_of("second");
    let broken = env.file(
        "broken.jsonl",
        "{\"command\": \"primitive.cube\"}\nnot json\n",
    );
    let out = env.gs(&["-s", "second", "run", &broken]);
    assert_eq!(out.code, 2, "a bad line: {}", out.stderr);
    assert!(
        out.stderr.starts_with(&format!("{broken}:2: ")),
        "points at the line: {}",
        out.stderr
    );
    let failing = env.file(
        "failing.jsonl",
        "\n{\"command\": \"mesh.stats\"}\n{\"command\": \"primitive.cube\", \"args\": {\"name\": \"cube\"}}\n",
    );
    let two_runs = env.file(
        "two-runs.jsonl",
        &format!(
            "{}\n{{\"command\": \"mesh.stats\"}}\n{}\n",
            header_of(&log),
            header_of(&log)
        ),
    );
    let out = env.gs(&["-s", "second", "run", &two_runs]);
    assert_eq!(out.code, 2, "a second header: {}", out.stderr);
    assert!(
        out.stderr.starts_with(&format!("{two_runs}:3: ")),
        "names the second header's line: {}",
        out.stderr
    );
    let out = env.gs(&["-s", "second", "run", &failing]);
    assert_eq!(out.code, 3, "a failing command's exit code");
    assert!(
        out.stderr
            .starts_with(&format!("{failing}:3: error E_PRECONDITION")),
        "names the failing line: {}",
        out.stderr
    );
    assert_eq!(
        env.gs(&["-s", "second", "run", "/nonexistent.jsonl"]).code,
        2,
        "a missing script"
    );
}

/// C1 and G1 through the real client: a log with a batch and an undo replays to the same
/// scene, a log line that replays differently is named, and a log from another engine or
/// catalog is flagged.
pub fn replays_check_every_line() {
    let env = Env::new();
    env.spawn_with("src", &["--verify"]);
    let script = env.file(
        "batch.jsonl",
        "{\"command\": \"cmd.batch\", \"args\": {\"commands\": [\
           {\"command\": \"primitive.sphere\", \"args\": {\"subdivisions\": 2}}, \
           {\"command\": \"mesh.translate\", \"args\": {\"direction\": [1, 0, 0], \"distance\": 1}}]}}\n\
         {\"command\": \"history.undo\"}\n\
         {\"command\": \"primitive.cube\"}\n",
    );
    env.ok(&["-s", "src", "run", &script]);
    // The session says where its log is: by default in the state folder, not the runtime one.
    let log = env.log_of("src");
    let log_path = std::path::PathBuf::from(&log);
    let logs = env.dir.join("state").join("gigasculpt").join("logs");
    assert_eq!(log_path.parent(), Some(logs.as_path()), "the log: {log}");
    env.spawn("copy");
    let replayed = env.ok(&["-s", "copy", "run", &log]);
    assert!(
        !replayed.stdout.contains("warning"),
        "same engine, same catalog: {}",
        replayed.stdout
    );
    assert_eq!(
        env.hash("copy"),
        env.hash("src"),
        "a batch replays as one step"
    );
    let text = fs::read_to_string(&log_path).unwrap_or_default();
    let tampered = text.replace("\"size\":2.0", "\"size\":3.0");
    assert_ne!(tampered, text, "the log holds the cube's resolved size");
    let path = env.file("tampered.jsonl", &tampered);
    env.spawn("check");
    let out = env.gs(&["-s", "check", "run", &path]);
    let line = 1 + tampered
        .lines()
        .position(|l| l.contains("primitive.cube"))
        .unwrap_or_default();
    assert_eq!(out.code, 1, "a diverging replay fails: {}", out.stderr);
    assert!(
        out.stderr.starts_with(&format!(
            "{path}:{line}: error: the replay diverged at primitive.cube"
        )),
        "names the line and the command: {}",
        out.stderr
    );
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut header: Value = serde_json::from_str(&lines[0]).unwrap_or_default();
    header["catalog_hash"] = json!("0000");
    lines[0] = header.to_string();
    let other = env.file("other.jsonl", &(lines.join("\n") + "\n"));
    env.spawn("warned");
    let out = env.ok(&["-s", "warned", "run", &other]);
    assert!(
        out.stdout
            .contains("warning: the log was written by engine"),
        "flags another catalog: {}",
        out.stdout
    );
}

/// The first line (the header) of a log.
pub fn header_of(log: &str) -> String {
    let text = fs::read_to_string(log).unwrap_or_else(|e| panic!("no log {log}: {e}"));
    text.lines().next().unwrap_or_default().to_string()
}
