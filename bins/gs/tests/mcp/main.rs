//! `gs mcp` end to end: a real headless session, the real `gs mcp` as a child process, and MCP
//! JSON-RPC over its stdio, the way Claude Code speaks it (docs/spikes/mcp.md).
//!
//! This test program doubles as the engine program: started as `<this program> serve ...` it
//! runs `gs_engine::cli::main_with` with gs-ui's offscreen renderer, the code the `gigasculpt`
//! binary runs, with one test-only long command (`test.wait`) registered, so a job can be
//! watched and cancelled.

#[path = "../common/mod.rs"]
mod common;
mod support;

use std::ffi::OsString;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use gs_engine::errors::error;
use gs_engine::{Command, Ctx, Example, register_command};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use libtest_mimic::{Arguments, Trial};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use support::Env;

/// Waits `steps` times `step_ms`, reporting progress after each wait; stops when cancelled.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TestWait {
    steps: u32,
    step_ms: u64,
}

impl Command for TestWait {
    const NAME: &'static str = "test.wait";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    type Output = u32;

    fn examples() -> Vec<Example<Self>> {
        let quick = Self {
            steps: 1,
            step_ms: 1,
        };
        vec![Example::new("wait a moment", quick)]
    }

    fn run(self, cx: &mut Ctx) -> Result<u32, CommandError> {
        for step in 1..=self.steps {
            if cx.cancelled() {
                return Err(error(ErrorCode::Cancelled, "stopped"));
            }
            thread::sleep(Duration::from_millis(self.step_ms));
            let message = format!("step {step} of {}", self.steps);
            cx.progress(Some(step as f32 / self.steps as f32), message);
        }
        Ok(self.steps)
    }
}
register_command!(TestWait);

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "serve") {
        return gs_engine::cli::main_with(args, gs_ui::Offscreen::host);
    }
    let cases: [(&str, fn()); 7] = [
        (
            "six_tools_that_answer_like_gs",
            six_tools_that_answer_like_gs,
        ),
        (
            "errors_come_back_as_tool_errors_with_code_and_hint",
            errors_come_back_as_tool_errors_with_code_and_hint,
        ),
        (
            "a_long_command_reports_progress_and_can_be_cancelled",
            a_long_command_reports_progress_and_can_be_cancelled,
        ),
        (
            "the_command_line_follows_jobs_too",
            the_command_line_follows_jobs_too,
        ),
        (
            "looks_are_one_png_of_the_size_asked",
            looks_are_one_png_of_the_size_asked,
        ),
        (
            "with_spawn_the_server_starts_its_session",
            with_spawn_the_server_starts_its_session,
        ),
        (
            "the_servers_output_ends_when_the_client_leaves_though_its_session_runs",
            the_servers_output_ends_when_the_client_leaves_though_its_session_runs,
        ),
    ];
    // Listed and run the way the standard harness does, so a runner can run each case alone.
    let trials = cases.map(|(name, case)| {
        Trial::test(name, move || {
            case();
            Ok(())
        })
    });
    libtest_mimic::run(&Arguments::from_args(), trials.into()).exit_code()
}

/// Exactly six tools, loaded up front; `gs_run` and `gs_query` answer with exactly what `gs call`
/// and `gs query` print, and leave the same scene; `gs_batch` is one step; `gs_help` finds and
/// describes commands.
fn six_tools_that_answer_like_gs() {
    let mut env = Env::new();
    env.spawn("agent");
    env.spawn("cli");
    let mut mcp = env.mcp("agent");
    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort_unstable();
    let six = [
        "gs_batch", "gs_help", "gs_job", "gs_look", "gs_query", "gs_run",
    ];
    assert_eq!(names, six, "exactly the six tools");
    for tool in &tools {
        assert_eq!(tool["_meta"]["anthropic/alwaysLoad"], json!(true), "{tool}");
        assert!(
            tool.get("outputSchema").is_none(),
            "no output schema: {tool}"
        );
    }
    let run = tools.iter().find(|t| t["name"] == json!("gs_run"));
    let schema = run.map(|t| t["inputSchema"].clone()).unwrap_or_default();
    assert!(schema["properties"]["command"].is_object(), "{schema}");
    assert_eq!(schema["required"], json!(["command"]), "{schema}");
    let about = mcp.request("server/discover", json!({}));
    let instructions = about.to_string();
    assert!(instructions.contains("follow it with gs_job"), "{about}");

    let sphere = json!({ "command": "primitive.sphere", "args": { "subdivisions": 3 } });
    let (text, failed) = mcp.text("gs_run", sphere);
    let printed = env.gs(&["-s", "cli", "call", "primitive.sphere", "subdivisions=3"]);
    assert!(!failed && printed.code == 0, "{text}\n{}", printed.stderr);
    assert_eq!(
        text,
        printed.stdout.trim_end(),
        "gs_run says what gs call says"
    );
    let (text, _) = mcp.text("gs_query", json!({ "command": "mesh.hash" }));
    let printed = env.gs(&["-s", "cli", "query", "mesh.hash"]);
    let hash = |text: &str| {
        text.lines()
            .find(|l| l.starts_with("result.hash: "))
            .map(str::to_string)
    };
    assert!(hash(&text).is_some(), "{text}");
    assert_eq!(hash(&text), hash(&printed.stdout), "the same scene");

    let steps = json!([
        { "command": "primitive.cube", "args": {} },
        { "command": "mesh.stats", "args": {} }
    ]);
    let (text, failed) = mcp.text("gs_batch", json!({ "commands": steps }));
    assert!(
        !failed && text.starts_with("ok ") && text.contains(" step=2 "),
        "{text}"
    );
    assert!(text.contains("result[1].parts"), "both results: {text}");

    let (text, _) = mcp.text("gs_help", json!({ "query": "sphere" }));
    assert!(text.starts_with("primitive.sphere  (edit)  "), "{text}");
    let (text, _) = mcp.text("gs_help", json!({ "command": "primitive.sphere" }));
    assert!(text.contains("arguments (JSON Schema): {"), "{text}");
    assert!(text.contains(r#"run: {"args":"#), "with examples: {text}");
    assert!(text.contains("\nfilled in when left out: name"), "{text}");
    let (text, failed) = mcp.text("gs_help", json!({ "query": "zzzz" }));
    assert!(!failed, "an empty search is no error: {text}");
    assert_eq!(
        text, "no command matches 'zzzz'; try fewer words",
        "says so"
    );
    let (all, _) = mcp.text("gs_help", json!({}));
    assert!(
        all.lines().count() > 20 && all.contains("\nmesh.stats  (query)  "),
        "{all}"
    );
}

/// Failures are `isError` results whose text carries the code, the bad argument's path, the
/// fix hint and an example; nothing is hidden in JSON-RPC errors.
fn errors_come_back_as_tool_errors_with_code_and_hint() {
    let mut env = Env::new();
    env.spawn("errors");
    let mut mcp = env.mcp("errors");
    let typo = json!({ "command": "primitive.sphere", "args": { "subdivisons": 3 } });
    let (text, failed) = mcp.text("gs_run", typo);
    assert!(failed, "{text}");
    for part in [
        "error E_ARGS_INVALID at /subdivisons",
        "hint: ",
        "did you mean: subdivisions",
        "example: ",
    ] {
        assert!(text.contains(part), "{part}: {text}");
    }
    let (text, failed) = mcp.text("gs_query", json!({ "command": "primitive.cube" }));
    assert!(failed && text.contains("not a query"), "{text}");
    let (text, failed) = mcp.text("gs_job", json!({ "wait_ms": 5 }));
    let missing = "error E_ARGS_INVALID: gs_job: missing field `id`";
    assert!(failed && text.starts_with(missing), "{text}");
    assert!(
        text.contains(r#"example: {"id":1"#),
        "the tool's own example: {text}"
    );
    let (text, failed) = mcp.text("gs_job", json!({ "id": "seven" }));
    assert!(
        failed && text.starts_with("error E_ARGS_INVALID at /id: gs_job: "),
        "{text}"
    );
    let (text, failed) = mcp.text("gs_job", json!({ "id": 42 }));
    assert!(
        failed && text.contains("E_NOT_FOUND") && text.contains("there is no job 42"),
        "{text}"
    );
    let (text, failed) = mcp.text("gs_help", json!({ "command": "primitive.spher" }));
    assert!(
        failed && text.contains("did you mean: primitive.sphere"),
        "{text}"
    );
    let (text, failed) = mcp.text("gs_teleport", json!({}));
    assert!(failed && text.contains("the tools are gs_help"), "{text}");
}

/// A long command answers with its job after `wait_ms`, sending progress while it waits;
/// `gs_job` reports how far it got and cancels it, and a cancelled job stays cancelled.
fn a_long_command_reports_progress_and_can_be_cancelled() {
    let mut env = Env::new();
    env.spawn("long");
    let mut mcp = env.mcp("long");
    let wait = json!({ "command": "test.wait", "args": { "steps": 40_000, "step_ms": 25 }, "wait_ms": 3000 });
    let (text, failed) = mcp.text("gs_run", wait);
    assert!(
        !failed && text.starts_with("running: job 1 (test.wait), "),
        "{text}"
    );
    assert!(
        text.contains("% done, step ") && text.contains(r#"gs_job {"id": 1, "cancel": true}"#),
        "{text}"
    );
    let notes = mcp.progress.len();
    assert!(notes >= 1, "progress while it waited: {:?}", mcp.progress);
    let note = &mcp.progress[0];
    assert_eq!(note["method"], json!("notifications/progress"), "{note}");
    assert_eq!(
        note["params"]["progressToken"],
        json!("call-1"),
        "the call's token"
    );
    let message = note["params"]["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("running: job 1"),
        "a readable line: {message}"
    );

    let (text, failed) = mcp.text("gs_job", json!({ "id": 1, "wait_ms": 2500 }));
    assert!(!failed && text.starts_with("running: job 1"), "{text}");
    let counts: Vec<f64> = (mcp.progress.iter())
        .filter(|n| n["params"]["progressToken"] == json!("call-2"))
        .filter_map(|n| n["params"]["progress"].as_f64())
        .collect();
    assert_eq!(
        counts,
        [1.0, 2.0],
        "gs_job sends progress too, always rising"
    );
    let (text, failed) = mcp.text("gs_job", json!({ "id": 1, "cancel": true }));
    assert!(failed && text.contains("E_CANCELLED"), "{text}");
    let (text, failed) = mcp.text("gs_job", json!({ "id": 1 }));
    assert!(
        failed && text.contains("E_CANCELLED"),
        "stays cancelled: {text}"
    );
    // A job that ends within the first wait just answers, with no progress note: the engine
    // waits for it before answering, for one command and for a batch alike.
    let quick = json!({ "command": "test.wait", "args": { "steps": 2, "step_ms": 20 } });
    let (text, failed) = mcp.text("gs_run", quick.clone());
    assert!(
        !failed && text.contains("result: 2"),
        "a quick one just answers: {text}"
    );
    let (text, failed) = mcp.text("gs_batch", json!({ "commands": [quick] }));
    assert!(!failed && text.contains("result: [2]"), "{text}");
    let quiet = |token: &str| {
        let token = json!(token);
        !(mcp.progress.iter()).any(|n| n["params"]["progressToken"] == token)
    };
    assert!(quiet("call-5") && quiet("call-6"), "{:?}", mcp.progress);
}

/// `gs call` waits for a long command until `--timeout-ms`, then names its job and how to
/// follow it; `gs job` reports and cancels it; `gs commands NAME` describes one command.
fn the_command_line_follows_jobs_too() {
    let mut env = Env::new();
    env.spawn("cli");
    let long = ["-s", "cli", "--timeout-ms", "1500", "call", "test.wait"];
    let out = env.gs(&[&long[..], &["steps=40000", "step_ms=25"]].concat());
    assert_eq!(out.code, 5, "still running is a timeout: {}", out.stderr);
    assert!(
        out.stderr.contains("running: job 1 (test.wait)"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("gs job 1 --wait-ms 60000"),
        "{}",
        out.stderr
    );
    let out = env.gs(&["-s", "cli", "job", "1", "--wait-ms", "100"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.starts_with("running: job 1 (test.wait), "),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("cancel it: gs job 1 --cancel"),
        "{}",
        out.stdout
    );
    let out = env.gs(&["-s", "cli", "--json", "job", "1"]);
    let report: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap_or_default();
    assert_eq!(report["id"], json!(1), "the whole report: {}", out.stdout);
    let out = env.gs(&["-s", "cli", "job", "1", "--cancel"]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(
        out.stderr.starts_with("error E_CANCELLED"),
        "{}",
        out.stderr
    );
    let out = env.gs(&["-s", "cli", "call", "test.wait", "steps=1", "step_ms=1"]);
    assert!(
        out.code == 0 && out.stdout.contains("result: 1"),
        "{}",
        out.stdout
    );
    // The call above received job 2's result, so the job now keeps a summary (decision 0010).
    let out = env.gs(&["-s", "cli", "job", "2"]);
    assert!(
        out.code == 0 && out.stdout.starts_with("ok seq=2") && out.stdout.contains("released"),
        "an ended job: {}",
        out.stdout
    );

    let out = env.gs(&["-s", "cli", "commands", "test.wait"]);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines[0].starts_with("test.wait  (query)  Waits"),
        "{}",
        out.stdout
    );
    let wanted = r#"  run: {"args":{"step_ms":1,"steps":1},"command":"test.wait"}"#;
    assert!(lines.contains(&wanted), "{}", out.stdout);
}

/// Width and height from a base64 PNG's header.
fn png_size(base64: &str) -> (u32, u32) {
    use base64::Engine as _;
    let png = (base64::engine::general_purpose::STANDARD.decode(base64)).unwrap_or_default();
    assert!(png.starts_with(b"\x89PNG"), "a PNG");
    let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap_or_default());
    (at(16), at(20))
}

/// `gs_look` answers with one PNG of the size asked (several views as one sheet), clamped at
/// 1568 px, plus a line saying what it shows; `gs look` writes the same to a file.
fn looks_are_one_png_of_the_size_asked() {
    let mut env = Env::new();
    env.spawn("eyes");
    let mut mcp = env.mcp("eyes");
    let (text, failed) = mcp.text("gs_look", json!({}));
    assert!(
        failed && text.contains("no parts"),
        "nothing to look at yet: {text}"
    );
    mcp.text("gs_run", json!({ "command": "primitive.sphere" }));
    let (content, failed) = mcp.tool("gs_look", json!({ "views": ["front", "iso"], "size": 512 }));
    assert!(
        !failed && content.len() == 2,
        "text and one image: {content:?}"
    );
    let said = json!("front, iso: 512x256 PNG, matcap shading");
    assert_eq!(content[0]["text"], said, "what the picture shows");
    assert_eq!(content[1]["type"], json!("image"), "{}", content[1]);
    assert_eq!(content[1]["mimeType"], json!("image/png"), "a PNG");
    let png = content[1]["data"].as_str().unwrap_or_default();
    assert_eq!(
        png_size(png),
        (512, 256),
        "the size asked, both views in one sheet"
    );
    let (content, _) = mcp.tool("gs_look", json!({ "size": 4000, "shading": "zebra" }));
    let png = content[1]["data"].as_str().unwrap_or_default();
    assert_eq!(png_size(png), (1568, 1568), "clamped at 1568 px");
    let alone = json!({ "command": "view.snapshot", "args": { "view": "front", "size": 256 } });
    let (text, _) = mcp.text("gs_run", alone);
    let snapshot = text
        .lines()
        .find_map(|l| l.strip_prefix("result.png_base64: "));
    let (content, _) = mcp.tool("gs_look", json!({ "views": ["front"], "size": 256 }));
    let one = content[1]["data"].as_str();
    assert_eq!(one, snapshot, "one view is a plain snapshot");
    let (content, _) = mcp.tool("gs_look", json!({}));
    let png = content[1]["data"].as_str().unwrap_or_default();
    assert_eq!(png_size(png), (1024, 1024), "1024 px by default");

    // A relative --out is the caller's folder, not the engine's.
    let look = [
        "-s", "eyes", "look", "--views", "front", "--orbit", "10,20", "--out", "a.png",
    ];
    let out = env.gs_in(&env.dir, &look);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let file = env.dir.join("a.png");
    assert!(
        out.stdout
            .starts_with("front, front az 10 el 20: 1024x512 PNG"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("wrote {}", file.display())),
        "{}",
        out.stdout
    );
    let written = std::fs::read(&file).unwrap_or_default();
    assert!(written.starts_with(b"\x89PNG"), "the file is a PNG");
    let plain = env.gs_in(
        &env.dir,
        &["-s", "eyes", "look", "--size", "64", "--out", "c.png"],
    );
    let said = plain.stdout.lines().next().unwrap_or_default();
    assert_eq!(
        said, "iso: 64x64 PNG, matcap shading",
        "iso by default: {}",
        plain.stderr
    );
    let bad = env.gs(&["-s", "eyes", "look", "--orbit", "1,2,3", "--out", "b.png"]);
    assert!(
        bad.code == 2 && bad.stderr.contains("AZIMUTH,ELEVATION"),
        "{}",
        bad.stderr
    );
}

/// `gs mcp --spawn` starts its session at the first call when it is not running, and uses it
/// as it is when it is; the session outlives the server.
fn with_spawn_the_server_starts_its_session() {
    let mut env = Env::new();
    let mut mcp = env.mcp_with("fresh", &["--spawn"]);
    let (text, failed) = mcp.text("gs_run", json!({ "command": "primitive.cube" }));
    assert!(
        !failed && text.starts_with("ok seq=1 "),
        "a new session: {text}"
    );
    env.keep_spawned_key("fresh");
    drop(mcp);
    let mut again = env.mcp_with("fresh", &["--spawn"]);
    let (text, failed) = again.text("gs_query", json!({ "command": "scene.describe" }));
    assert!(
        !failed && text.contains("cube"),
        "the same session, still running: {text}"
    );
}

/// A client that leaves (closes the server's input) sees the server's output end, while the
/// session `--spawn` started runs on. On Windows the engine inherited a copy of the server's
/// own input and output, so its output ended only when the session stopped (decisions 0064
/// and 0065). Linux and macOS hand a program only the handles it is given, so there this
/// passes with or without the fix.
fn the_servers_output_ends_when_the_client_leaves_though_its_session_runs() {
    let mut env = Env::new();
    let mut mcp = env.mcp_with("stays", &["--spawn"]);
    let (text, failed) = mcp.text("gs_run", json!({ "command": "primitive.cube" }));
    assert!(!failed, "a new session: {text}");
    env.keep_spawned_key("stays");
    assert!(
        mcp.leave(),
        "the output did not end within 30 s of the client leaving"
    );
    let out = env.gs(&["-s", "stays", "query", "mesh.hash"]);
    assert_eq!(out.code, 0, "the session still answers: {}", out.stderr);
}
