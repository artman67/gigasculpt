//! The Blender add-on's client over a session socket on this system (decision 0063): the
//! add-on's own `client` module, in a headless Blender, says hello and runs one command against
//! a small server on [`UnixListener`], which echoes the command's arguments. On Windows the
//! add-on reaches the socket through Winsock, as Python's socket module has no Unix sockets
//! there. Blender is `GS_BLENDER`, else `/usr/bin/blender`; `GS_BLENDER=skip` skips the test.
//! Tier 1 leaves it to tier 2; CI runs it on Linux, Windows and macOS.

mod tier2 {
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::thread;

    use gs_protocol::UnixListener;
    use serde_json::{Value, json};

    /// Answers every line on the first connection to `listener`: `engine.hello` with the
    /// protocol, `cmd.run` with its arguments echoed as the command's result.
    fn serve(listener: UnixListener) -> thread::JoinHandle<Vec<String>> {
        thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return vec![];
            };
            let mut methods = vec![];
            let second = stream.try_clone();
            let reader = BufReader::new(second.unwrap_or_else(|e| panic!("a second handle: {e}")));
            for line in reader.lines().map_while(Result::ok) {
                let request: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("a JSON request: {e}: {line}"));
                let method = request["method"].as_str().unwrap_or_default().to_string();
                let result = match method.as_str() {
                    "engine.hello" => json!({ "protocol": 1, "session": "bridge-test" }),
                    _ => json!({ "ok": { "result": { "echo": request["params"]["args"] } } }),
                };
                let reply = json!({ "jsonrpc": "2.0", "id": request["id"], "result": result });
                writeln!(&stream, "{reply}").unwrap_or_else(|e| panic!("the reply: {e}"));
                methods.push(method);
            }
            methods
        })
    }

    /// Runs the add-on's `script` with `args` in a headless Blender of its own, as gs-engine's
    /// exchange tests do, and returns its `RESULT` line; `None` when Blender is skipped.
    fn blender(dir: &Path, script: &str, args: &[&Path]) -> Option<Value> {
        let bin = std::env::var("GS_BLENDER").unwrap_or_else(|_| "/usr/bin/blender".into());
        if bin == "skip" {
            eprintln!("SKIP {script}: GS_BLENDER=skip");
            return None;
        }
        let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons/blender/tests");
        let out = Command::new(&bin)
            .args(["-b", "--factory-startup", "--offline-mode"])
            .args(["--python-exit-code", "1", "--python"])
            .arg(tests.join(script))
            .arg("--")
            .args(args)
            .env("BLENDER_USER_RESOURCES", dir.join("blender"))
            .output()
            .unwrap_or_else(|e| panic!("cannot run Blender at {bin} (set GS_BLENDER): {e}"));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let said = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{script} failed in Blender:\n{said}");
        let line = stdout.lines().find_map(|l| l.strip_prefix("RESULT "));
        let line = line.unwrap_or_else(|| panic!("{script} gave no RESULT line:\n{said}"));
        Some(serde_json::from_str(line).unwrap_or_else(|e| panic!("{script}: {e}: {line}")))
    }

    #[test]
    fn the_add_on_says_hello_and_runs_a_command_over_this_systems_socket() {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let socket = dir.join("s.sock");
        std::fs::remove_file(&socket).ok();
        let server = serve(UnixListener::bind(&socket).expect("the socket"));
        let Some(found) = blender(&dir, "transport_check.py", &[&socket]) else {
            return;
        };
        let platform = match std::env::consts::OS {
            "windows" => "win32",
            "macos" => "darwin",
            other => other,
        };
        let expected = json!({
            "hello": { "protocol": 1, "session": "bridge-test" },
            "echoed": { "echo": { "text": "h\u{e9}llo" } },
            "platform": platform,
        });
        assert_eq!(found, expected, "what the add-on's client got");
        let methods = server.join().expect("the server thread");
        assert_eq!(
            methods,
            ["engine.hello", "cmd.run"],
            "what the server was asked"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
