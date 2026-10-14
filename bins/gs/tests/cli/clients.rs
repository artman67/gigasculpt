//! Who a client is and what it may send: `GS_CLIENT` names a client, so two agents each undo
//! only their own steps, and a request over the session's line limit comes back as `E_LIMIT`.

use std::process::Command;
use std::time::Duration;

use crate::support::Env;

/// Two agents with their own `GS_CLIENT` names share a session: a plain undo only moves the
/// caller's own step. Without the variable every `gs` call is the same client, `gs`.
pub fn named_clients_undo_only_their_own_steps() {
    let env = Env::new();
    env.spawn("shared");
    let (a, b) = ([("GS_CLIENT", "agent-a")], [("GS_CLIENT", "agent-b")]);
    let made = env.gs_with(&["call", "primitive.cube"], &a);
    assert_eq!(made.code, 0, "{}", made.stderr);
    let history = env.ok(&["query", "history.list", "--json"]).json();
    let client = history["ok"]["result"]["steps"][0]["client"].clone();
    assert!(
        client.as_str().is_some_and(|c| c.starts_with("agent-a#")),
        "the step is agent-a's: {client}"
    );
    let refused = env.gs_with(&["call", "history.undo"], &b);
    assert_eq!(
        refused.code, 3,
        "agent-b cannot undo it: {}",
        refused.stderr
    );
    assert!(
        refused.stderr.contains("another client's step"),
        "{}",
        refused.stderr
    );
    let undone = env.gs_with(&["call", "history.undo"], &a);
    assert_eq!(undone.code, 0, "agent-a can: {}", undone.stderr);
    env.ok(&["call", "primitive.cube"]);
    let blank = [("GS_CLIENT", " ")];
    let same = env.gs_with(&["call", "history.undo"], &blank);
    assert_eq!(same.code, 0, "a blank name is plain gs: {}", same.stderr);
}

/// A request line over the session's limit gets `E_LIMIT` (exit code 3), not a protocol error,
/// and the session keeps serving.
pub fn an_oversized_request_is_refused_with_e_limit() {
    let env = Env::new();
    let me = std::env::current_exe().unwrap_or_else(|e| panic!("no test program path: {e}"));
    let mut engine = Command::new(me)
        .args([
            "serve",
            "--headless",
            "--session",
            "small",
            "--max-line-mib",
        ])
        .args(["1", "--stop-key", "small-key"])
        .env("XDG_RUNTIME_DIR", &env.dir)
        .env("XDG_STATE_HOME", env.dir.join("state"))
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start the engine: {e}"));
    env.keys
        .borrow_mut()
        .insert("small".into(), "small-key".into());
    let socket = env.dir.join("gigasculpt").join("small.sock");
    // Starting an engine can take well over 30 s on a busy machine: wait while it lives, up to
    // 3 minutes, and fail at once if it ends first.
    let listening = (0..3600).any(|_| {
        if let Some(status) = engine.try_wait().unwrap_or_else(|e| panic!("{e}")) {
            panic!("the engine ended ({status}) before listening");
        }
        let up = socket.exists();
        std::thread::sleep(Duration::from_millis(50 * u64::from(!up)));
        up
    });
    assert!(listening, "the engine never made {}", socket.display());
    let big = format!(r#"{{"part": "{}"}}"#, "x".repeat(2 << 20));
    let file = env.file("big.json", &big);
    let out = env.gs(&["-s", "small", "query", "mesh.hash", &format!("@{file}")]);
    assert_eq!(out.code, 3, "a limit: {}", out.stderr);
    assert!(
        out.stderr.contains("E_LIMIT"),
        "the code itself: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("smaller parts"),
        "with the fix: {}",
        out.stderr
    );
    env.ok(&["-s", "small", "query", "mesh.stats"]);
    env.stop("small");
    engine.wait().ok();
}
