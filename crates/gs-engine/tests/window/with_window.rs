//! With the real window host attached (gs-ui's studio, drawing on lavapipe): the engine tells
//! the window about every call, window commands reach it, and screenshots are written where
//! asked.

use gs_engine::EngineConfig;
use gs_protocol::{BatchParams, Request, Role, Step};
use gs_render::{AdapterKind, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

use gs_testkit::session::{Session, code};

use crate::{As, origin};

/// A session named `studio` whose window draws on lavapipe, or draws nothing.
fn windowed(draws: bool) -> (Session, Host) {
    let config = EngineConfig {
        session: "studio".into(),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    let shell = Shell::new(
        s.engine.catalog().clone(),
        Keymap::default(),
        LayoutFile::in_memory(Layout::default()),
    );
    let render = draws.then(|| {
        let renderer = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
        Render::new(renderer, 1).unwrap_or_else(|e| panic!("{e}"))
    });
    let host = Host::new(Studio::new(shell, render, [640, 400], 1.0));
    s.engine.attach_window(Box::new(host.clone()));
    (s, host)
}

/// The labels in the status bar, read as the window (an agent's look would itself show up as
/// agent activity).
fn status(s: &mut Session) -> Vec<String> {
    let d = s.by(Role::Human).query("ui.describe", json!({}));
    let status = d["panels"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["id"] == "status").cloned())
        .unwrap_or_default();
    status["widgets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["label"].as_str().map(String::from))
        .collect()
}

/// Widens the window to show its whole status line: a narrower one leaves out the line's end,
/// the session's name and revision among it (decision 0016, section 17, item 10).
fn widen(host: &Host) {
    host.lock().resize([1600, 400], 1.0);
}

#[test]
fn attaching_tells_the_window_about_the_session() {
    let (mut s, host) = windowed(false);
    widen(&host);
    let labels = status(&mut s);
    assert!(labels.contains(&"Session studio".to_string()), "{labels:?}");
    assert!(labels.contains(&"rev 0".to_string()), "{labels:?}");
    assert!(
        labels.contains(&"Agent: no agent activity".to_string()),
        "{labels:?}"
    );
}

#[test]
fn every_call_reaches_the_window() {
    let (mut s, host) = windowed(false);
    widen(&host);
    s.by(Role::Agent).query("primitive.sphere", json!({}));
    let labels = status(&mut s);
    assert!(labels.contains(&"rev 1".to_string()), "{labels:?}");
    assert!(
        labels.contains(&"Agent: claude#9 ran primitive.sphere (done)".to_string()),
        "{labels:?}"
    );
    let parts = s.by(Role::Agent).query("ui.describe", json!({}))["viewport"]["parts"].clone();
    assert_eq!(parts, json!(["sphere"]), "the window has the parts");
    let batch = Request::Batch(BatchParams {
        commands: vec![
            Step {
                command: "primitive.cube".into(),
                args: json!({}),
            },
            Step {
                command: "part.get".into(),
                args: json!({ "part": "nope" }),
            },
        ],
        if_rev: None,
    });
    s.engine.execute(&origin(Role::Agent), batch);
    let agent_line = |s: &mut Session| {
        status(s)
            .into_iter()
            .find(|l| l.starts_with("Agent:"))
            .unwrap_or_default()
    };
    assert_eq!(
        agent_line(&mut s),
        "Agent: claude#9 ran primitive.cube, part.get (refused)",
        "a batch names every step; this one failed as a whole"
    );
    s.by(Role::Human)
        .query("primitive.cube", json!({ "name": "box" }));
    assert!(
        agent_line(&mut s).contains("(refused)"),
        "the human's own calls are not agent activity"
    );
    s.by(Role::Human)
        .query("session.control", json!({ "paused": true }));
    assert!(
        host.lock().shell.state.paused,
        "the pause reached the window"
    );
}

#[test]
fn the_palette_opens_in_the_window_and_is_not_logged() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("window-palette-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let (mut s, host) = windowed(false);
    let info = s
        .by(Role::Agent)
        .query("ui.palette", json!({ "query": "undo" }));
    assert_eq!(info["open"], json!(true));
    // The rows lead with the controls, by their labels (review 0009, C12): History's Undo.
    let runs = |i: usize| info["rows"][i]["command"]["command"].clone();
    let first = [runs(0), runs(1)];
    assert!(
        first.contains(&json!("history.undo")),
        "history.undo is among the best matches (view.undo matches as well): {first:?}"
    );
    assert!(host.lock().shell.palette.open);
    let info = s
        .by(Role::Agent)
        .query("ui.palette", json!({ "open": false }));
    assert_eq!(
        (info["open"].clone(), info["query"].clone()),
        (json!(false), json!("undo"))
    );
    let logged = EngineConfig {
        log_path: Some(dir.join("palette.jsonl")),
        ..EngineConfig::default()
    };
    let mut quiet = Session::with(logged, Role::Human);
    let shell = Shell::new(
        quiet.engine.catalog().clone(),
        Keymap::default(),
        LayoutFile::in_memory(Layout::default()),
    );
    quiet.engine.attach_window(Box::new(Host::new(Studio::new(
        shell,
        None,
        [640, 400],
        1.0,
    ))));
    quiet.by(Role::Agent).query("ui.palette", json!({}));
    drop(quiet);
    let text = std::fs::read_to_string(dir.join("palette.jsonl")).unwrap_or_default();
    assert!(!text.contains("ui.palette"), "not in the log: {text}");
}

fn shot(s: &mut Session, args: Value) -> Value {
    s.by(Role::Agent).query("ui.screenshot", args)
}

#[test]
fn screenshots_come_back_as_base64_or_go_to_a_file() {
    let (mut blind, _h) = windowed(false);
    let e = blind.by(Role::Agent).err("ui.screenshot", json!({}));
    assert_eq!(
        code(e.code),
        "E_INTERNAL",
        "a window that draws nothing cannot be shot"
    );
    let (mut s, _host) = windowed(true);
    s.by(Role::Agent).query("primitive.sphere", json!({}));
    let inline = shot(&mut s, json!({}));
    assert_eq!(
        (inline["width"].clone(), inline["height"].clone()),
        (json!(640), json!(400))
    );
    let text = inline["png_base64"].as_str().unwrap_or_default();
    assert!(text.starts_with("iVBORw0KGgo"), "a PNG, in base64");
    assert_eq!(inline.get("path"), None);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("window-shot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("window.png").display().to_string();
    let saved = shot(&mut s, json!({ "path": path }));
    assert_eq!(saved["path"], json!(path));
    assert_eq!(saved.get("png_base64"), None);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let nowhere = dir.join("missing").join("window.png").display().to_string();
    let e = s
        .by(Role::Agent)
        .err("ui.screenshot", json!({ "path": nowhere }));
    assert_eq!(code(e.code), "E_PRECONDITION");
    assert_eq!(e.path.as_deref(), Some("/path"), "points at the bad path");
    assert!(e.message.contains("cannot write"), "{}", e.message);
}
