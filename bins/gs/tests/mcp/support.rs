//! A private session for the test, `gs` runs against it, and `gs mcp` as a child process spoken
//! to in MCP JSON-RPC over its stdin and stdout.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::common::{exit_code, runtime_dir};
use serde_json::{Value, json};

/// A private runtime folder with sessions in it. Stops everything started in it, and removes
/// it, when dropped, also when the test fails part way.
pub struct Env {
    pub dir: PathBuf,
    keys: Vec<(String, String)>,
}

/// What a `gs` run printed.
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Env {
    pub fn new() -> Self {
        // Its own folder: dropping an Env kills every process started in its folder.
        Self {
            dir: runtime_dir("gs-mcp"),
            keys: Vec::new(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let me = std::env::current_exe().unwrap_or_else(|e| panic!("no test program path: {e}"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_gs"));
        command
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.dir)
            // Engines log in the state folder: keep their logs in this case's folder.
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("GS_ENGINE", me)
            .env_remove("GS_SESSION");
        command
    }

    pub fn gs(&self, args: &[&str]) -> Out {
        self.gs_in(&std::env::current_dir().unwrap_or_default(), args)
    }

    /// `gs` run from folder `dir`.
    pub fn gs_in(&self, dir: &std::path::Path, args: &[&str]) -> Out {
        let mut command = self.command(args);
        let output = (command.current_dir(dir).output()).unwrap_or_else(|e| panic!("gs: {e}"));
        Out {
            code: exit_code(output.status),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Spawns a session and keeps its stop key. A busy machine can take well over the default
    /// 10 s to start an engine; `gs spawn` fails at once if the engine dies.
    pub fn spawn(&mut self, name: &str) {
        let out = self.gs(&["spawn", name, "--wait-ms", "180000"]);
        let key = (out.stdout.lines())
            .find_map(|l| l.strip_prefix("stop key: "))
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or_else(|| panic!("spawn {name}: {}{}", out.stdout, out.stderr));
        self.keys.push((name.into(), key.into()));
    }

    /// `gs mcp` on a session.
    pub fn mcp(&self, session: &str) -> Mcp {
        self.mcp_with(session, &[])
    }

    /// `gs mcp` on a session, with more flags.
    pub fn mcp_with(&self, session: &str, flags: &[&str]) -> Mcp {
        let args = [&["-s", session, "mcp"], flags].concat();
        let mut child = (self.command(&args))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("gs mcp: {e}"));
        let input = child.stdin.take().unwrap_or_else(|| panic!("no stdin"));
        let output = child.stdout.take().unwrap_or_else(|| panic!("no stdout"));
        let (lines, received) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    return;
                }
            }
        });
        Mcp {
            child,
            input,
            received,
            next: 0,
            progress: Vec::new(),
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (name, key) in &self.keys {
            self.gs(&["-s", name, "--timeout-ms", "3000", "stop", "--key", key]);
        }
        // An engine `gs stop` could not reach would otherwise run forever. Engines inherit
        // this test's private runtime folder, so find them by it.
        let marker = format!("XDG_RUNTIME_DIR={}", self.dir.display());
        for process in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let environ = fs::read(process.path().join("environ")).unwrap_or_default();
            if environ.split(|&b| b == 0).any(|v| v == marker.as_bytes()) {
                Command::new("kill")
                    .arg("-9")
                    .arg(process.file_name())
                    .status()
                    .ok();
            }
        }
        fs::remove_dir_all(&self.dir).ok();
    }
}

/// `gs mcp`, spoken to as Claude Code does: no `initialize`, client details in every
/// request's `_meta`, and a progress token with every call.
pub struct Mcp {
    child: Child,
    input: ChildStdin,
    received: Receiver<String>,
    next: u64,
    /// Progress notifications received so far.
    pub progress: Vec<Value>,
}

impl Mcp {
    /// The result of one request; notifications that arrive first are kept in `progress`.
    pub fn request(&mut self, method: &str, mut params: Value) -> Value {
        self.next += 1;
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "gs-test", "version": "1" },
            "io.modelcontextprotocol/clientCapabilities": {},
            "progressToken": format!("call-{}", self.next),
        });
        let request =
            json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params });
        writeln!(self.input, "{request}").unwrap_or_else(|e| panic!("send: {e}"));
        loop {
            let line = (self.received.recv_timeout(Duration::from_secs(60)))
                .unwrap_or_else(|e| panic!("no answer to {method}: {e}"));
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
            if message["id"] == json!(self.next) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
            self.progress.push(message);
        }
    }

    /// Calls a tool: its result's content and whether it is an error.
    pub fn tool(&mut self, name: &str, arguments: Value) -> (Vec<Value>, bool) {
        let mut params = json!({ "name": name });
        params["arguments"] = arguments;
        let result = self.request("tools/call", params);
        assert!(
            result.get("structuredContent").is_none(),
            "text only: {result}"
        );
        let content = result["content"].as_array().cloned().unwrap_or_default();
        (content, result["isError"] == json!(true))
    }

    /// Calls a tool that answers with text only.
    pub fn text(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let (content, error) = self.tool(name, arguments);
        assert_eq!(content.len(), 1, "one text block: {content:?}");
        let text = content[0]["text"].as_str().unwrap_or_default().to_string();
        (text, error)
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
