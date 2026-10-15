//! A private session for the test, `gs` runs against it, and `gs mcp` as a child process spoken
//! to in MCP JSON-RPC over its stdin and stdout.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::common::{exit_code, gs_command, runtime_dir, stop_key_in};
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
        gs_command(&self.dir, args)
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
        let key = stop_key_in(&out.stdout)
            .unwrap_or_else(|| panic!("spawn {name}: {}{}", out.stdout, out.stderr));
        self.keys.push((name.into(), key.into()));
    }

    /// Keeps the stop key a `gs mcp --spawn` on `session` wrote to its log when it started the
    /// session, so the session is stopped.
    pub fn keep_spawned_key(&mut self, session: &str) {
        let log = fs::read_to_string(self.log_of(session)).unwrap_or_default();
        let key = stop_key_in(&log).unwrap_or_else(|| panic!("no stop key in the log: {log}"));
        self.keys.push((session.into(), key.into()));
    }

    /// Where the `gs mcp` servers on `session` write what they print to stderr.
    fn log_of(&self, session: &str) -> PathBuf {
        self.dir.join(format!("mcp-{session}.log"))
    }

    /// `gs mcp` on a session.
    pub fn mcp(&self, session: &str) -> Mcp {
        self.mcp_with(session, &[])
    }

    /// `gs mcp` on a session, with more flags.
    pub fn mcp_with(&self, session: &str, flags: &[&str]) -> Mcp {
        let args = [&["-s", session, "mcp"], flags].concat();
        let log = (File::options().create(true).append(true))
            .open(self.log_of(session))
            .unwrap_or_else(|e| panic!("mcp log: {e}"));
        let mut child = (self.command(&args))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
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
            input: Some(input),
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
    /// The server's input; gone once the client leaves.
    input: Option<ChildStdin>,
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
        let input = self
            .input
            .as_mut()
            .unwrap_or_else(|| panic!("the client left"));
        writeln!(input, "{request}").unwrap_or_else(|e| panic!("send: {e}"));
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

    /// Closes the server's input, as a client that leaves does. True if the server's output
    /// then ends within 30 s.
    pub fn leave(&mut self) -> bool {
        self.input = None;
        loop {
            match self.received.recv_timeout(Duration::from_secs(30)) {
                Ok(_) => {}
                Err(end) => return end == RecvTimeoutError::Disconnected,
            }
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
