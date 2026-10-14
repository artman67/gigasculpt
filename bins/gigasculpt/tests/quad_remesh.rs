//! Decision 0038, section 1: an engine whose `GS_QUADWILD` names a folder without the Quad
//! Remesh program refuses `mesh.quad_remesh` with the reason and the fix, which names the cache
//! folder, and changes nothing. A process of its own, since the engine reads the variables from
//! its environment.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use gs_protocol::UnixStream;
use serde_json::{Value, json};

/// Kills the engine and removes its folder when the test ends, however it ends.
struct Engine(Child, PathBuf);

impl Drop for Engine {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
        std::fs::remove_dir_all(&self.1).ok();
    }
}

/// `command` with `args` run over the socket: the reply's `result`.
fn run(socket: &Path, command: &str, args: &Value) -> std::io::Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let params = json!({ "command": command, "args": args });
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "cmd.run", "params": params });
    stream.write_all(format!("{request}\n").as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply: Value = serde_json::from_str(&reply)?;
    Ok(reply["result"].clone())
}

#[test]
fn a_missing_program_refuses_with_the_reason_and_the_fix() {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("gigasculpt-quad-{}", std::process::id()));
    let empty = dir.join("no-program");
    std::fs::create_dir_all(&empty).unwrap_or_else(|e| panic!("{e}"));
    let (socket, cache) = (dir.join("quad.sock"), dir.join("cache"));
    let child = Command::new(env!("CARGO_BIN_EXE_gigasculpt"))
        .args(["serve", "--headless", "--session", "quad", "--socket"])
        .arg(&socket)
        .env("GS_QUADWILD", &empty)
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CACHE_HOME", &cache)
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start the engine: {e}"));
    let mut engine = Engine(child, dir);
    // Starting an engine can take a while on a busy machine: wait while it lives, up to 3
    // minutes, and fail at once if it ends first.
    let up = (0..18_000).find_map(|_| {
        if let Some(status) = engine.0.try_wait().unwrap_or_else(|e| panic!("{e}")) {
            panic!("the engine ended ({status}) before listening");
        }
        let made = run(&socket, "primitive.sphere", &json!({ "subdivisions": 8 })).ok();
        if made.is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        made
    });
    assert!(up.is_some_and(|r| r.get("ok").is_some()), "the sphere");
    let hash = || {
        let reply = run(&socket, "mesh.hash", &json!({})).unwrap_or_else(|e| panic!("{e}"));
        (reply["ok"]["result"]["hash"].clone(), reply["rev"].clone())
    };
    let before = hash();
    let refused = run(&socket, "mesh.quad_remesh", &json!({ "faces": 200 }))
        .unwrap_or_else(|e| panic!("{e}"));
    let error = &refused["error"];
    assert_eq!(error["code"], json!("E_PRECONDITION"), "{refused}");
    let (message, hint) = (error["message"].to_string(), error["hint"].as_str());
    assert!(
        message.contains("Quad Remesh program is missing") && message.contains("no stamp"),
        "the reason: {message}"
    );
    let hint = hint.unwrap_or_default();
    let folder = cache
        .join("gigasculpt")
        .join("quadwild")
        .display()
        .to_string();
    let how = if cfg!(target_os = "linux") {
        "build.sh"
    } else {
        "not shipped"
    };
    assert!(
        hint.contains(&folder) && hint.contains(how) && hint.contains("GS_QUADWILD"),
        "the fix names {folder} and {how:?}: {hint}"
    );
    assert_eq!(hash(), before, "nothing changed");
}
