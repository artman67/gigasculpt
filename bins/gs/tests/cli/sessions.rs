//! Sessions: spawning, picking, stopping, names, and sessions that hang or vanish.

use std::fs;
use std::io::{BufRead, BufReader};

use gs_protocol::UnixListener;
use serde_json::json;

use crate::support::Env;

pub fn spawn_call_and_stop() {
    let env = Env::new();
    env.spawn("one");
    let again = env.gs(&["spawn", "one"]);
    assert_eq!(
        again.code, 2,
        "a running session cannot be spawned twice: {}",
        again.stderr
    );
    let made = env.ok(&["call", "primitive.sphere", "subdivisions=2"]);
    let header = made.lines()[0];
    assert!(
        header.starts_with("ok seq=1 rev=1 step=1"),
        "header: {header}"
    );
    let stats = env.ok(&["-s", "one", "query", "mesh.stats"]);
    assert!(
        stats.lines().contains(&"result.parts[0].vertices: 26"),
        "{}",
        stats.stdout
    );
    let refused = env.gs(&["stop"]);
    assert_eq!(refused.code, 3, "no key, no stop: {}", refused.stderr);
    let refused = env.gs(&["stop", "--key", "guess"]);
    assert_eq!(refused.code, 3, "a wrong key: {}", refused.stderr);
    let stopped = env.stop("one");
    assert!(stopped.stdout.contains("stopped"), "{}", stopped.stdout);
    assert!(env.sessions().is_empty(), "the socket is gone");
    assert_eq!(
        env.gs(&["call", "mesh.stats"]).code,
        4,
        "no session to talk to"
    );
}

pub fn picking_a_session() {
    let env = Env::new();
    env.spawn("a");
    env.spawn("b");
    let keys = env.keys.borrow().clone();
    assert!(
        keys["a"] != keys["b"] && keys.values().all(|k| k.len() == 32),
        "every session gets its own 128-bit stop key: {keys:?}"
    );
    let several = env.gs(&["query", "mesh.stats"]);
    assert_eq!(several.code, 2, "ambiguous: {}", several.stderr);
    assert!(
        several.stderr.contains("a, b"),
        "lists them: {}",
        several.stderr
    );
    env.ok(&["-s", "a", "call", "primitive.cube"]);
    let in_b = env.gs_with(&["query", "mesh.stats", "--json"], &[("GS_SESSION", "b")]);
    assert_eq!(
        in_b.json()["ok"]["result"]["parts"],
        json!([]),
        "GS_SESSION picks b"
    );
    assert_ne!(env.hash("a"), env.hash("b"), "a has the cube");
    assert_eq!(
        env.gs(&["-s", "nope", "query", "mesh.stats"]).code,
        4,
        "unknown session"
    );
    assert_eq!(env.stop("a").code, 0, "a stops");
    fs::write(env.dir.join("gigasculpt").join("ghost.sock"), "").unwrap_or_else(|e| panic!("{e}"));
    let only = env.ok(&["query", "mesh.stats", "--json"]);
    assert_eq!(
        only.json()["ok"]["result"]["parts"],
        json!([]),
        "the only one left is b"
    );
}

pub fn spawn_failures() {
    let env = Env::new();
    let bad = env.gs(&["spawn", "bad.name"]);
    assert_eq!(bad.code, 2, "gs refuses the name: {}", bad.stderr);
    assert!(
        bad.stderr.contains("bad session name"),
        "and says why: {}",
        bad.stderr
    );
    let missing = env.gs_with(&["spawn", "x"], &[("GS_ENGINE", "/nonexistent/engine")]);
    assert_eq!(missing.code, 4, "no engine program: {}", missing.stderr);
    #[cfg(unix)] // The engine that never listens is a shell script.
    {
        use std::os::unix::fs::PermissionsExt;
        let hang = env.file("hang.sh", "#!/bin/sh\nexec sleep 30\n");
        fs::set_permissions(&hang, fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|e| panic!("{e}"));
        let slow = env.gs_with(&["spawn", "x", "--wait-ms", "300"], &[("GS_ENGINE", &hang)]);
        assert_eq!(
            slow.code, 5,
            "an engine that never listens times out: {}",
            slow.stderr
        );
    }
    assert!(env.sessions().is_empty(), "nothing is left running");
}

/// m10: a session name can never reach outside the session folder.
pub fn names_stay_in_the_session_folder() {
    let env = Env::new();
    for args in [
        &["spawn", "../escape"][..],
        &["-s", "../escape", "query", "mesh.stats"],
    ] {
        let out = env.gs(args);
        assert_eq!(out.code, 2, "gs {args:?}: {}", out.stderr);
        assert!(
            out.stderr.contains("bad session name"),
            "says why: {}",
            out.stderr
        );
    }
    assert!(
        !env.dir.join("escape.stderr").exists(),
        "nothing is written outside the session folder"
    );
}

/// A session name too long for its socket is a mistake in the command, not a session that
/// cannot be reached: every command that names one exits 2 and names the longest that fits.
pub fn a_session_name_too_long_for_its_socket_is_a_usage_error() {
    let env = Env::new();
    // The socket is `<runtime>/gigasculpt/<name>.sock`, and sockets take at most 103 bytes on
    // macOS, 107 elsewhere.
    let limit = if cfg!(target_os = "macos") { 103 } else { 107 };
    let fits = limit - env.dir.join("gigasculpt").as_os_str().len() - "/.sock".len();
    let long = "n".repeat(fits + 1);
    for args in [
        &["spawn", &long][..],
        &["-s", &long, "query", "mesh.stats"],
        &["-s", &long, "stop", "--key", "k"],
    ] {
        let out = env.gs(args);
        assert_eq!(out.code, 2, "gs {args:?}: {}", out.stderr);
        assert!(
            out.stderr
                .contains(&format!("names of up to {fits} characters fit")),
            "names the longest that fits: {}",
            out.stderr
        );
    }
    let fits = "n".repeat(fits);
    let out = env.gs(&["-s", &fits, "query", "mesh.stats"]);
    assert_eq!(
        out.code, 4,
        "the longest that fits is only not running: {}",
        out.stderr
    );
}

/// M3: a session that never answers makes `gs` give up with exit code 5, not hang.
pub fn a_stuck_session_times_out() {
    let env = Env::new();
    let dir = env.dir.join("gigasculpt");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    // Accepts connections (the kernel queues them) but never reads or answers.
    let stuck = UnixListener::bind(dir.join("stuck.sock")).unwrap_or_else(|e| panic!("{e}"));
    let out = env.gs(&["-s", "stuck", "--timeout-ms", "300", "query", "mesh.stats"]);
    assert_eq!(out.code, 5, "a timeout: {}", out.stderr);
    assert!(
        out.stderr.contains("did not answer"),
        "says what happened: {}",
        out.stderr
    );
    drop(stuck);
    // A session that drops the connection with the request unread: lost, not slow.
    let gone = UnixListener::bind(dir.join("gone.sock")).unwrap_or_else(|e| panic!("{e}"));
    let dropper = std::thread::spawn(move || {
        if let Ok((stream, _)) = gone.accept() {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(stream);
        }
    });
    let out = env.gs(&["-s", "gone", "query", "mesh.stats"]);
    dropper.join().ok();
    assert_eq!(out.code, 4, "the session went away: {}", out.stderr);
    assert!(
        out.stderr.contains("lost the session"),
        "says so: {}",
        out.stderr
    );
    // A session that reads the request and closes without answering.
    let quiet = UnixListener::bind(dir.join("quiet.sock")).unwrap_or_else(|e| panic!("{e}"));
    let closer = std::thread::spawn(move || {
        if let Ok((stream, _)) = quiet.accept() {
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).ok();
        }
    });
    let out = env.gs(&["-s", "quiet", "query", "mesh.stats"]);
    closer.join().ok();
    assert_eq!(out.code, 4, "no answer: {}", out.stderr);
    assert!(
        out.stderr.contains("closed the connection"),
        "says so: {}",
        out.stderr
    );
}
