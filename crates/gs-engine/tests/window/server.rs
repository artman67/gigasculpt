//! The session server with a window: `Server::start_with` attaches the window before anyone is
//! served, the window's in-process handle runs commands as the human, and closing the window
//! ends the session and removes the socket. (An agent driving a live window through the socket
//! is gs-ui's end-to-end test.)

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_engine::server::{ServeOptions, Server, Window};
use gs_protocol::{Outcome, Request, RunParams, UnixStream};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Shell, Studio};
use serde_json::{Value, json};

fn run(window: &Window, command: &str, args: Value) -> Value {
    let request = Request::Run(RunParams::new(command, args));
    let reply = window
        .execute(request)
        .unwrap_or_else(|| panic!("the engine is running"));
    match reply.outcome {
        Outcome::Ok(ok) => ok.result,
        Outcome::Error(e) => panic!("{command}: {}", e.message),
    }
}

#[test]
fn the_window_attaches_before_serving_runs_as_the_human_and_closes_the_session() {
    // In $XDG_RUNTIME_DIR (else /tmp): a socket path must stay under 108 bytes, and the target
    // folder can sit deep (mutation testing copies the workspace under target/).
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| "/tmp".into(), PathBuf::from);
    let socket = base.join(format!("gs-window-server-{}.sock", std::process::id()));
    let _leftovers = Leftovers(socket.clone());
    let config = EngineConfig {
        session: "live".into(),
        ..EngineConfig::default()
    };
    let options = ServeOptions {
        socket: socket.clone(),
        stop_key: None,
        max_line_bytes: 1 << 20,
    };
    let mut host = None;
    let server = Server::start_with(config, &options, |engine| {
        let shell = Shell::new(
            engine.catalog().clone(),
            Keymap::default(),
            LayoutFile::in_memory(Layout::default()),
        );
        let window = Host::new(Studio::new(shell, None, [900, 600], 1.0));
        engine.attach_window(Box::new(window.clone()));
        host = Some(window);
    })
    .unwrap_or_else(|e| panic!("serve: {e}"));
    let host = host.unwrap_or_else(|| panic!("the window was attached before serving"));
    assert_eq!(host.lock().shell.status.session, "live", "told at once");

    let window = server.window();
    let pen = json!({ "samples": [{ "x": 30.0, "y": 40.0, "contact": true, "pressure": 0.75 }] });
    let out = run(&window, "pen.input", pen);
    assert_eq!(out["window"], json!(true), "the window took the samples");
    let state = run(&window, "session.state", json!({}));
    assert_eq!(
        state["session.gesture"],
        json!(true),
        "the window is the human: its touch is a gesture"
    );
    run(&window, "session.control", json!({ "paused": true }));
    assert!(
        host.lock().shell.state.paused,
        "the status bar shows the pause"
    );

    let mut stream = UnixStream::connect(&socket).unwrap_or_else(|e| panic!("connect: {e}"));
    let hello = r#"{"jsonrpc":"2.0","id":1,"method":"engine.hello","params":{"client":"probe"}}"#;
    writeln!(stream, "{hello}").unwrap_or_else(|e| panic!("{e}"));
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        reply.contains("\"session\":\"live\""),
        "agents can attach: {reply}"
    );

    window.close();
    server.wait().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !socket.exists(),
        "the socket is gone when the window closes"
    );
}

/// The test's socket: removed with the lock file a server keeps next to it when dropped, also
/// when the test fails part way.
struct Leftovers(PathBuf);

impl Drop for Leftovers {
    fn drop(&mut self) {
        for leftover in [self.0.clone(), self.0.with_extension("lock")] {
            std::fs::remove_file(leftover).ok();
        }
    }
}
