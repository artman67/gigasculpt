//! What `gs` prints: text that keeps every value, errors with every detail, listings and
//! help.

use serde_json::{Value, json};

use crate::support::{Env, Out};

/// Every leaf of a JSON value with the path `gs` prints for it.
pub fn leaves(path: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };
    match value {
        Value::Object(map) if !map.is_empty() => {
            map.iter().for_each(|(k, v)| leaves(&join(k), v, out));
        }
        Value::Array(items) if items.iter().any(|i| i.is_object() || i.is_array()) => {
            items
                .iter()
                .enumerate()
                .for_each(|(i, v)| leaves(&format!("{path}[{i}]"), v, out));
        }
        _ => out.push((path.to_string(), value.clone())),
    }
}

/// The text output names every value of the JSON output, one `path: value` line each. Queries
/// run twice get different sequence numbers, so `same_seq` says whether to compare them.
pub fn assert_lossless(text: &Out, json: &Value, same_seq: bool) {
    let ok = &json["ok"];
    let header = text.lines()[0].to_string();
    assert!(header.starts_with("ok "), "{header}");
    let seq = if same_seq { &json["seq"] } else { &Value::Null };
    for (key, value) in [
        ("seq", seq),
        ("rev", &json["rev"]),
        ("step", &ok["history_id"]),
    ] {
        assert!(
            value.is_null() || header.contains(&format!("{key}={value}")),
            "{key} in {header}"
        );
    }
    assert!(
        header.contains(&format!(
            "concurrent_changes={}",
            json["concurrent_changes"]
        )),
        "{header}"
    );
    let mut expected = Vec::new();
    leaves("result", &ok["result"], &mut expected);
    if let Some(effect) = ok.get("effect") {
        leaves("effect", effect, &mut expected);
    }
    for (path, value) in expected {
        let shown = match &value {
            Value::String(s) if s.contains('\n') => {
                s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>()
            }
            Value::String(s) if s.is_empty() || serde_json::from_str::<Value>(s).is_ok() => {
                vec![format!("{path}: {value}")]
            }
            Value::String(s) => vec![format!("{path}: {s}")],
            other => vec![format!("{path}: {other}")],
        };
        for line in shown {
            assert!(
                text.lines().contains(&line.as_str()),
                "missing {line:?} in:\n{}",
                text.stdout
            );
        }
    }
    for warning in ok["warnings"].as_array().into_iter().flatten() {
        assert!(
            text.stdout.contains(&format!(
                "warning: {}",
                warning.as_str().unwrap_or_default()
            )),
            "warning"
        );
    }
}

pub fn text_output_keeps_every_value() {
    let env = Env::new();
    for session in ["text", "json"] {
        env.spawn(session);
        env.ok(&["-s", session, "call", "primitive.sphere", "subdivisions=1"]);
        env.ok(&["-s", session, "call", "history.checkpoint", "name=base"]);
    }
    let edits: [&[&str]; 3] = [
        &["call", "mesh.translate", "direction=1,0,0", "distance=2"],
        &["call", "mesh.translate", "direction=1,0,0", "distance=0"],
        &["call", "mesh.jitter", "amount=0.1"],
    ];
    for args in edits {
        let text = env.ok(&[&["-s", "text"], args].concat());
        let json = env.ok(&[&["-s", "json", "--json"], args].concat()).json();
        assert_lossless(&text, &json, true);
    }
    let queries: [&[&str]; 5] = [
        &["query", "mesh.stats"],
        &["query", "history.list"],
        &["query", "scene.describe"],
        &["query", "part.get"],
        &["query", "commands.describe", "name=scene.describe"],
    ];
    for args in queries {
        let text = env.ok(&[&["-s", "text"], args].concat());
        let json = env.ok(&[&["-s", "text", "--json"], args].concat()).json();
        assert_lossless(&text, &json, false);
    }
    let moved = env.ok(&[
        "-s",
        "text",
        "call",
        "mesh.translate",
        "direction=0,1,0",
        "distance=1",
    ]);
    assert!(
        moved
            .lines()
            .contains(&r#"effect.changed.sphere: ["positions"]"#),
        "the effect names what changed: {}",
        moved.stdout
    );
    log_tail_is_one_line_per_entry(&env);
    let unit = env.ok(&["-s", "text", "query", "session.info"]);
    assert!(
        unit.lines()
            .iter()
            .any(|l| l.starts_with("result.session: text")),
        "{}",
        unit.stdout
    );
    let typed = env.ok(&["-s", "text", "call", "primitive.cube", "name=123"]);
    assert!(
        typed.lines().contains(&"result.part: \"123\"")
            && typed.lines().contains(&"result.vertices: 8"),
        "text that reads as a number is quoted, numbers are not: {}",
        typed.stdout
    );
}

/// `log.tail` prints one line per entry (its sequence number, revision, client, role, commands
/// and status), and `--json` keeps every field.
fn log_tail_is_one_line_per_entry(env: &Env) {
    let text = env.ok(&["-s", "text", "query", "log.tail", "n=3"]);
    let json = env.ok(&["-s", "text", "--json", "query", "log.tail", "n=3"]);
    let json = json.json();
    let entries = json["ok"]["result"]["entries"].as_array().cloned();
    let entries = entries.unwrap_or_default();
    assert_eq!(entries.len(), 3, "{json}");
    let word = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let lines: Vec<String> = (entries.iter())
        .map(|e| {
            let fields = ["seq", "rev", "client", "role", "commands", "status"];
            assert!(fields.iter().all(|f| e.get(f).is_some()), "{e}");
            let commands = e["commands"].as_array().cloned().unwrap_or_default();
            let commands: Vec<String> = commands.iter().map(word).collect();
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
        .collect();
    assert_eq!(text.lines()[1..], lines, "{}", text.stdout);
}

pub fn errors_keep_every_detail() {
    let env = Env::new();
    env.spawn("errors");
    let args = [
        "call",
        "primitive.sphere",
        "subdivisions=0",
        "radius=-1",
        "subdivisons=4",
    ];
    let text = env.gs(&args);
    let json = env.gs(&[&args[..], &["--json"]].concat());
    assert_eq!(
        (text.code, json.code),
        (2, 2),
        "the same exit code with --json"
    );
    let error = &json.json()["error"];
    let first = text.stderr.lines().next().unwrap_or_default();
    assert!(
        first.starts_with(&format!(
            "error {} at {}: ",
            error["code"].as_str().unwrap_or("?"),
            error["path"].as_str().unwrap_or("?")
        )),
        "{first}"
    );
    assert!(
        first.ends_with(error["message"].as_str().unwrap_or("?")),
        "{first}"
    );
    for violation in error["violations"].as_array().into_iter().flatten() {
        let line = format!(
            "  {}: {}",
            violation["path"].as_str().unwrap_or("?"),
            violation["message"].as_str().unwrap_or("?")
        );
        assert!(
            text.stderr.lines().any(|l| l == line),
            "missing {line:?} in {}",
            text.stderr
        );
    }
    assert!(
        text.stderr
            .contains(&format!("hint: {}", error["hint"].as_str().unwrap_or("?"))),
        "hint"
    );
    assert!(
        text.stderr
            .contains(&format!("example: {}", error["example"])),
        "example"
    );
    let misspelled = env.gs(&["call", "mesh.stat"]);
    assert!(
        misspelled.stderr.contains("did you mean: mesh.stats, "),
        "{}",
        misspelled.stderr
    );
    let first = misspelled.stderr.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("error E_UNKNOWN_COMMAND: "),
        "no path, no ' at': {first}"
    );
    assert!(
        text.stdout.is_empty() && !json.stdout.is_empty(),
        "errors go to stderr, --json to stdout"
    );
}

pub fn listing_and_help() {
    let env = Env::new();
    env.spawn("list");
    let info = env.ok(&["query", "session.info", "--json"]).json();
    let all = env.ok(&["commands"]);
    assert_eq!(
        json!(all.lines().len()),
        info["ok"]["result"]["commands"],
        "one line per command"
    );
    assert!(
        all.lines()
            .iter()
            .any(|l| l.starts_with("mesh.stats  (query)  ")),
        "{}",
        all.stdout
    );
    assert!(
        all.lines()
            .iter()
            .any(|l| l.starts_with("primitive.cube  (edit)  ")),
        "{}",
        all.stdout
    );
    assert!(
        all.lines()
            .iter()
            .any(|l| l.starts_with("session.control  (session)  [human only]  ")),
        "marks what only the human may run: {}",
        all.stdout
    );
    let found = env.ok(&["commands", "sphere"]);
    assert!(
        found.lines()[0].starts_with("primitive.sphere  (edit)  Add a sphere"),
        "{}",
        found.stdout
    );
    assert!(
        found.lines().len() < all.lines().len(),
        "a search narrows the list"
    );
    let help = env.ok(&["call", "primitive.sphere", "--help"]);
    for flag in [
        "--name",
        "--subdivisions",
        "--radius",
        "--center",
        "Add a sphere",
    ] {
        assert!(
            help.stdout.contains(flag),
            "help shows {flag}:\n{}",
            help.stdout
        );
    }
    assert_eq!(
        env.gs(&["call", "primitive.sphere", "--nope"]).code,
        2,
        "unknown flag"
    );
}
