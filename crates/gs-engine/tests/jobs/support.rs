//! A real session server per test, and socket clients that speak to it line by line.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use gs_engine::EngineConfig;
use gs_engine::server::{ServeOptions, Server, Window};
use gs_protocol::{Request, RunParams, UnixStream};
use serde_json::{Value, json};

const KEY: &str = "jobs-key";

/// A session server on its own socket. Stops, and removes its socket and lock, when dropped.
pub struct Session {
    server: Option<Server>,
    pub socket: PathBuf,
}

impl Session {
    pub fn start(name: &str) -> Self {
        Self::start_with(name, |_| {})
    }

    /// A session the caller sets up first (a renderer, say), as the binaries do.
    pub fn start_with(name: &str, prepare: impl FnOnce(&mut gs_engine::Engine)) -> Self {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| "/tmp".into(), PathBuf::from);
        let socket = base.join(format!("gs-jobs-{}-{name}.sock", std::process::id()));
        let config = EngineConfig {
            session: name.into(),
            ..EngineConfig::default()
        };
        let options = ServeOptions {
            socket: socket.clone(),
            stop_key: Some(KEY.into()),
            max_line_bytes: 1 << 20,
        };
        let server =
            Server::start_with(config, &options, prepare).unwrap_or_else(|e| panic!("start: {e}"));
        Self {
            server: Some(server),
            socket,
        }
    }

    pub fn client(&self) -> Client {
        Client::connect(&self.socket)
    }

    /// The human at the window.
    pub fn window(&self) -> Window {
        let server = self.server.as_ref();
        server.map_or_else(|| panic!("the session stopped"), Server::window)
    }

    /// Runs a command as the human.
    pub fn as_human(&self, command: &str, args: Value) {
        let request = Request::Run(RunParams::new(command, args));
        let response = (self.window().execute(request))
            .unwrap_or_else(|| panic!("the engine stopped before {command}"));
        let response = json!(response);
        assert!(response.get("ok").is_some(), "{command}: {response}");
    }

    /// Stops the session and waits until it has, as `engine.shutdown` does. A session that
    /// does not stop within a minute fails the test instead of hanging it.
    pub fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            if let Ok(mut client) = Client::try_connect(&self.socket) {
                client.call("engine.shutdown", json!({ "key": KEY }));
            }
            let stopping = std::thread::spawn(move || server.wait().ok());
            let stopped = (0..6000).any(|_| {
                std::thread::sleep(Duration::from_millis(10));
                stopping.is_finished()
            });
            assert!(stopped, "the session did not stop");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        std::fs::remove_file(&self.socket).ok();
        std::fs::remove_file(self.socket.with_extension("lock")).ok();
    }
}

/// One socket client, introduced as an agent.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn try_connect(socket: &PathBuf) -> std::io::Result<Self> {
        Self::try_connect_as(socket, "test", "agent")
    }

    fn try_connect_as(socket: &PathBuf, name: &str, role: &str) -> std::io::Result<Self> {
        let writer = UnixStream::connect(socket)?;
        writer.set_read_timeout(Some(Duration::from_secs(30)))?;
        let reader = BufReader::new(writer.try_clone()?);
        let mut client = Self { reader, writer };
        client.call("engine.hello", json!({ "client": name, "role": role }));
        Ok(client)
    }

    pub fn connect(socket: &PathBuf) -> Self {
        Self::try_connect(socket).unwrap_or_else(|e| panic!("connect: {e}"))
    }

    /// A client introduced as `name` with `role` (`agent` or `observer`).
    pub fn connect_as(socket: &PathBuf, name: &str, role: &str) -> Self {
        Self::try_connect_as(socket, name, role).unwrap_or_else(|e| panic!("connect: {e}"))
    }

    /// The whole JSON-RPC response to one request.
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let request = with(
            json!({ "jsonrpc": "2.0", "id": 1, "method": method }),
            "params",
            params,
        );
        writeln!(self.writer, "{request}").unwrap_or_else(|e| panic!("send: {e}"));
        let mut line = String::new();
        (self.reader.read_line(&mut line)).unwrap_or_else(|e| panic!("reply: {e}"));
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }

    /// The result of a request that must succeed.
    pub fn result(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// `cmd.run`: the command's response.
    pub fn run(&mut self, command: &str, args: Value) -> Value {
        self.result("cmd.run", with(json!({ "command": command }), "args", args))
    }

    /// `job.start` of one command: the job report.
    pub fn start(&mut self, command: &str, args: Value, wait_ms: u64) -> Value {
        let run = with(json!({ "command": command }), "args", args);
        self.result("job.start", with(json!({ "wait_ms": wait_ms }), "run", run))
    }

    /// `job.status`: the job report.
    pub fn status(&mut self, id: &Value, wait_ms: u64, cancel: bool) -> Value {
        let params = json!({ "id": id, "wait_ms": wait_ms, "cancel": cancel });
        self.result("job.status", params)
    }

    pub fn hash(&mut self) -> Value {
        self.run("mesh.hash", json!({}))["ok"]["result"]["hash"].clone()
    }
}

/// `object` with `key` set to `value`.
fn with(mut object: Value, key: &str, value: Value) -> Value {
    object[key] = value;
    object
}

/// A world-space Draw stroke of size 0.4 over the part "sphere" near `at`: four samples 0.05
/// apart along x.
pub fn stroke(at: [f32; 3]) -> Value {
    let samples: Vec<Value> = (0..4)
        .map(|i| json!({ "position": [at[0] + 0.05 * i as f32, at[1], at[2]] }))
        .collect();
    json!({ "brush": "Draw", "part": "sphere", "samples": samples, "size": 0.4 })
}

/// A slow `test.slow`: `steps` waits of 25 ms.
pub fn slow(steps: u32) -> Value {
    json!({ "steps": steps, "step_ms": 25, "offset": 0.5 })
}
