//! The shipped binaries together: the real `gs` starts the real `gigasculpt serve --headless`
//! (found next to `gs`, as in an install), adds a sphere, undoes and redoes it, and stops. The
//! real engine process keeps no file descriptor per finished connection, and refuses a session
//! name too long for its socket, or an explicit `--socket` too long, before binding.
//!
//! `gs` is the program next to `gigasculpt` in the build folder. `cargo test --workspace` builds
//! both; when this package is tested alone (cargo-mutants does that), the test builds `gs` itself
//! in the same profile.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The `gs` next to `gigasculpt`, built in the same profile if it is not there yet.
fn gs_binary() -> PathBuf {
    let engine = PathBuf::from(env!("CARGO_BIN_EXE_gigasculpt"));
    let gs = engine.with_file_name(format!("gs{}", std::env::consts::EXE_SUFFIX));
    if !gs.exists() {
        let folder = engine
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("debug");
        let profile = if folder == "debug" { "dev" } else { folder };
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--package", "gs", "--profile", profile])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .unwrap_or_else(|e| panic!("cannot run cargo to build gs: {e}"));
        assert!(status.success(), "building gs for this test failed");
    }
    assert!(gs.exists(), "gs was not built at {}", gs.display());
    gs
}

/// A short base folder for sessions: `$XDG_RUNTIME_DIR`, else the temporary folder. Session
/// sockets must stay under 108 bytes wherever the tests run from.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

fn gs(runtime: &Path, args: &[&str]) -> (i32, String) {
    let output = Command::new(gs_binary())
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime)
        // The engine's log goes in the state folder: keep it in the test's folder.
        .env("XDG_STATE_HOME", runtime.join("state"))
        .env_remove("GS_ENGINE")
        .env_remove("GS_SESSION")
        .output()
        .unwrap_or_else(|e| panic!("cannot run gs: {e}"));
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    (output.status.code().unwrap_or(-1), text)
}

fn hash(runtime: &Path) -> String {
    let (code, text) = gs(runtime, &["query", "mesh.hash", "--json"]);
    assert_eq!(code, 0, "{text}");
    let reply: Value = serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("{e}: {text}"));
    reply["ok"]["result"]["hash"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Stops the session and removes its folder even when the test fails part way.
struct Cleanup(PathBuf, String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.0.join("gigasculpt").join("real.sock").exists() {
            gs(
                &self.0,
                &["--timeout-ms", "60000", "stop", "--key", &self.1],
            );
        }
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn gs_drives_a_headless_gigasculpt_session() {
    let runtime = runtime_dir().join(format!("gigasculpt-bin-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).unwrap_or_else(|e| panic!("{e}"));
    // A busy machine can take well over the default 10 s to start an engine; `gs spawn` fails
    // at once if the engine dies, so the long limit only stretches a real hang.
    let (code, text) = gs(&runtime, &["spawn", "real", "--wait-ms", "180000"]);
    let key = text
        .lines()
        .find_map(|l| l.strip_prefix("stop key: "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_default()
        .to_string();
    let _cleanup = Cleanup(runtime.clone(), key.clone());
    assert!(code == 0 && !key.is_empty(), "spawn: {text}");
    let empty = hash(&runtime);
    let (code, text) = gs(&runtime, &["call", "primitive.sphere", "subdivisions=3"]);
    assert!(code == 0 && text.contains("vertices: 56"), "sphere: {text}");
    let sphere = hash(&runtime);
    assert_ne!(sphere, empty, "the sphere is there");
    assert_eq!(gs(&runtime, &["call", "history.undo"]).0, 0, "undo");
    assert_eq!(hash(&runtime), empty, "undo removes it exactly");
    assert_eq!(gs(&runtime, &["call", "history.redo"]).0, 0, "redo");
    assert_eq!(hash(&runtime), sphere, "redo brings it back exactly");
    let (code, text) = gs(&runtime, &["--timeout-ms", "60000", "stop", "--key", &key]);
    assert_eq!(code, 0, "stop: {text}");
    assert!(
        !runtime.join("gigasculpt").join("real.sock").exists(),
        "the socket is gone"
    );
}

/// A session name too long for its socket fails at once with words the user can act on, before
/// the engine makes its session folder, its lock or its socket.
#[test]
fn a_session_name_too_long_for_its_socket_is_refused_before_binding() {
    let runtime = runtime_dir().join(format!("gigasculpt-long-{}", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_gigasculpt"))
        .args(["serve", "--headless", "--session", &"n".repeat(120)])
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_STATE_HOME", runtime.join("state"))
        .output()
        .unwrap_or_else(|e| panic!("cannot run the engine: {e}"));
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    let made = runtime.join("gigasculpt").exists();
    std::fs::remove_dir_all(&runtime).ok();
    assert!(
        !output.status.success() && said.contains("is too long") && said.contains("characters"),
        "refused with the limit and the longest name: {said}"
    );
    assert!(!made, "nothing was made in the session folder");
}

/// An explicit `--socket` too long for the system's socket address is refused the same way,
/// before the engine makes the socket's folder, its lock or the socket.
#[test]
fn an_explicit_socket_too_long_is_refused_before_binding() {
    let runtime = runtime_dir().join(format!("gigasculpt-sock-{}", std::process::id()));
    let folder = runtime.join("sockets");
    let output = Command::new(env!("CARGO_BIN_EXE_gigasculpt"))
        .args(["serve", "--headless", "--socket"])
        .arg(folder.join(format!("{}.sock", "s".repeat(120))))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_STATE_HOME", runtime.join("state"))
        .output()
        .unwrap_or_else(|e| panic!("cannot run the engine: {e}"));
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    let made = folder.exists();
    std::fs::remove_dir_all(&runtime).ok();
    assert!(
        !output.status.success() && said.contains("is too long") && said.contains("bytes"),
        "refused with the limit: {said}"
    );
    assert!(!made, "nothing was made in the socket's folder");
}

/// The engine's descriptors are counted in Linux's `/proc`.
#[cfg(target_os = "linux")]
mod descriptors {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, Command};
    use std::thread;
    use std::time::Duration;

    use gs_protocol::UnixStream;

    use super::runtime_dir;

    /// Kills the engine process when the test ends, however it ends.
    struct Engine(Child);

    impl Drop for Engine {
        fn drop(&mut self) {
            self.0.kill().ok();
            self.0.wait().ok();
        }
    }

    /// The engine's open file descriptors, a work counter for connections it still holds.
    fn descriptors(pid: u32) -> usize {
        std::fs::read_dir(format!("/proc/{pid}/fd")).map_or_else(
            |e| panic!("cannot list the engine's descriptors: {e}"),
            Iterator::count,
        )
    }

    /// M3: every connection's descriptors are closed when it ends, so a long session never runs
    /// out of them. 250 clients connect, ask something and leave; the count stays where it was.
    #[test]
    fn finished_connections_leave_no_descriptors_behind() {
        let dir = runtime_dir().join(format!("gigasculpt-fd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let socket = dir.join("fd.sock");
        let child = Command::new(env!("CARGO_BIN_EXE_gigasculpt"))
            .args(["serve", "--headless", "--session", "fd", "--socket"])
            .arg(&socket)
            .env("XDG_STATE_HOME", dir.join("state"))
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start the engine: {e}"));
        let mut engine = Engine(child);
        let ask = || {
            let mut stream = UnixStream::connect(&socket)?;
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            stream.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"cmd.run\",\"params\":{\"command\":\"mesh.stats\"}}\n")?;
            let mut reply = String::new();
            BufReader::new(stream).read_line(&mut reply)?;
            Ok::<String, std::io::Error>(reply)
        };
        // Starting an engine can take well over a few seconds on a busy machine: wait while it
        // lives, up to 3 minutes, and fail at once if it ends first.
        let listening = (0..18_000).any(|_| {
            if let Some(status) = engine.0.try_wait().unwrap_or_else(|e| panic!("{e}")) {
                panic!("the engine ended ({status}) before listening");
            }
            let up = ask().is_ok();
            thread::sleep(Duration::from_millis(10 * u64::from(!up)));
            up
        });
        assert!(
            listening,
            "the engine never listened at {}",
            socket.display()
        );
        let pid = engine.0.id();
        // A connection's thread closes it just after the client leaves; give it a moment.
        thread::sleep(Duration::from_millis(200));
        let before = descriptors(pid);
        for i in 0..250 {
            let reply = ask().unwrap_or_else(|e| panic!("call {i}: {e}"));
            assert!(reply.contains("\"ok\""), "call {i}: {reply}");
        }
        // The last threads close later on a busy machine: wait up to a minute for the count to
        // come back (leaked descriptors never do).
        let settled = (0..6000).any(|_| {
            let back = descriptors(pid) <= before + 2;
            thread::sleep(Duration::from_millis(10 * u64::from(!back)));
            back
        });
        let after = descriptors(pid);
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            settled,
            "the engine held {before} descriptors before 250 connections and {after} after"
        );
    }
}
