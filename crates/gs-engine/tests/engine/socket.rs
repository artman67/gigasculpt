//! Socket round trips: the real server, started through the same command-line code the
//! `gigasculpt` binary runs (or in-process with the window's handle, for the tests that need
//! the human), answering JSON-RPC lines from several clients over a Unix socket.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gs_engine::EngineConfig;
use gs_engine::server::ServeOptions;
use gs_protocol::UnixStream;
use serde_json::{Value, json};

pub fn gigasculpt(args: &[&str]) -> ExitCode {
    let args = std::iter::once("gigasculpt")
        .chain(args.iter().copied())
        .map(OsString::from);
    gs_engine::cli::main(args)
}

/// How long to wait for anything before calling it a hang: every wait ends as soon as what it
/// waits for happens, and a busy machine can take far longer than an idle one.
pub const PATIENCE: Duration = Duration::from_secs(60);
/// The stop key the test servers are started with.
pub const KEY: &str = "test-key";

/// The exit code of a command-line thread, or `None` if it is still running after a while.
pub fn finished(thread: JoinHandle<ExitCode>) -> Option<ExitCode> {
    for _ in 0..PATIENCE.as_millis() / 10 {
        if thread.is_finished() {
            return thread.join().ok();
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Runs the command line on its own thread; `None` means it is still running (serving).
pub fn briefly(args: &[&str]) -> Option<ExitCode> {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    finished(thread::spawn(move || {
        gigasculpt(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }))
}

pub fn scratch(name: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sk{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    dir
}

/// A socket path for `name`, in `$XDG_RUNTIME_DIR` (else /tmp) rather than the target folder:
/// a socket path must stay under 108 bytes, and mutation testing copies the workspace deep
/// under target/. A server removes its socket when it stops.
pub fn socket_path(name: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| "/tmp".into(), PathBuf::from);
    base.join(format!("gs-test-{}-{name}.sock", std::process::id()))
}

/// Waits until something listens at `socket`.
pub fn listening(socket: &Path, alive: impl Fn() -> bool) {
    for _ in 0..PATIENCE.as_millis() / 10 {
        if UnixStream::connect(socket).is_ok() {
            return;
        }
        assert!(alive(), "the server stopped before listening");
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the server never listened at {}", socket.display());
}

/// A test server's socket: removed with its lock file when dropped, also when a test fails part
/// way. (Servers keep their lock file on purpose; each test socket has its own name.)
pub struct Leftovers(pub PathBuf);

impl Drop for Leftovers {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
        std::fs::remove_file(self.0.with_extension("lock")).ok();
    }
}

pub struct Server {
    pub socket: PathBuf,
    pub log: PathBuf,
    pub thread: JoinHandle<ExitCode>,
    _leftovers: Leftovers,
}

impl Server {
    pub fn start(name: &str, extra: &[&str]) -> Self {
        Self::start_at(&socket_path(name), name, extra)
    }

    /// Starts a server through the command line on `socket`, logging into the scratch folder
    /// for `name`, with the stop key `KEY`, and waits until it listens.
    pub fn start_at(socket: &Path, name: &str, extra: &[&str]) -> Self {
        let (socket, log) = (socket.to_path_buf(), scratch(name).join("s.jsonl"));
        let mut args: Vec<String> = ["serve", "--headless", "--session", name, "--stop-key", KEY]
            .map(String::from)
            .to_vec();
        args.extend([
            "--socket".into(),
            socket.display().to_string(),
            "--log".into(),
            log.display().to_string(),
        ]);
        args.extend(extra.iter().map(ToString::to_string));
        let thread =
            thread::spawn(move || gigasculpt(&args.iter().map(String::as_str).collect::<Vec<_>>()));
        let leftovers = Leftovers(socket.clone());
        listening(&socket, || !thread.is_finished());
        Self {
            socket,
            log,
            thread,
            _leftovers: leftovers,
        }
    }

    pub fn client(&self, name: &str, role: &str) -> Client {
        Client::hello(&self.socket, name, role)
    }

    /// Shuts the server down with its stop key and returns its exit code.
    pub fn stop(self) -> ExitCode {
        let reply = Client::connect(&self.socket).rpc("engine.shutdown", json!({ "key": KEY }));
        assert_eq!(
            reply["result"],
            json!({ "stopping": true }),
            "shutdown reply"
        );
        let code = finished(self.thread).unwrap_or_else(|| panic!("the server did not stop"));
        assert!(!self.socket.exists(), "the socket is removed on shutdown");
        code
    }
}

/// Waits for an in-process server to stop, failing (not hanging) if it does not.
pub fn stopped(server: gs_engine::server::Server) {
    let waiting = thread::spawn(move || server.wait().is_ok());
    for _ in 0..PATIENCE.as_millis() / 10 {
        if waiting.is_finished() {
            assert!(
                waiting.join().unwrap_or(false),
                "the server stopped cleanly"
            );
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the server did not stop");
}

/// A server started in-process, so the test can act as the human at the window, with its socket
/// (keep the `Leftovers` alive until the end of the test).
pub fn with_window(name: &str) -> (gs_engine::server::Server, PathBuf, Leftovers) {
    let socket = socket_path(name);
    let options = ServeOptions {
        socket: socket.clone(),
        stop_key: None,
        max_line_bytes: 1 << 20,
    };
    let server = gs_engine::server::Server::start(EngineConfig::default(), &options)
        .unwrap_or_else(|e| panic!("the server did not start: {e}"));
    (server, socket.clone(), Leftovers(socket))
}

pub struct Client {
    pub reader: BufReader<UnixStream>,
    pub writer: UnixStream,
    pub next_id: u64,
}

impl Client {
    pub fn connect(socket: &Path) -> Self {
        let writer = UnixStream::connect(socket).unwrap_or_else(|e| panic!("connect: {e}"));
        writer
            .set_read_timeout(Some(PATIENCE))
            .unwrap_or_else(|e| panic!("timeout: {e}"));
        let reader = BufReader::new(writer.try_clone().unwrap_or_else(|e| panic!("clone: {e}")));
        Self {
            reader,
            writer,
            next_id: 0,
        }
    }

    pub fn hello(socket: &Path, name: &str, role: &str) -> Self {
        let mut client = Self::connect(socket);
        let hello = client.rpc(
            "engine.hello",
            json!({ "client": name, "role": role, "protocol": 1 }),
        );
        assert!(hello.get("result").is_some(), "hello failed: {hello}");
        client
    }

    /// Sends raw bytes and a line break.
    pub fn send(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.write_all(b"\n"))
            .unwrap_or_else(|e| panic!("write: {e}"));
    }

    /// Reads one response line.
    pub fn read(&mut self) -> Value {
        let mut reply = String::new();
        self.reader
            .read_line(&mut reply)
            .unwrap_or_else(|e| panic!("read: {e}"));
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("bad reply {reply:?}: {e}"))
    }

    /// Sends one raw line and reads one response.
    pub fn raw(&mut self, line: &str) -> Value {
        self.send(line.as_bytes());
        self.read()
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let request = gs_protocol::RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(self.next_id)),
            method: method.into(),
            params,
        };
        let reply = self.raw(&json!(request).to_string());
        assert_eq!(
            reply["id"],
            json!(self.next_id),
            "the reply answers this request"
        );
        assert_eq!(reply["jsonrpc"], json!("2.0"), "JSON-RPC 2.0");
        reply
    }

    /// Runs a command and returns the response envelope (`seq`, `rev`, `ok` or `error`...).
    pub fn run(&mut self, command: &str, args: Value) -> Value {
        let mut params = json!({ "command": command });
        params["args"] = args;
        self.rpc("cmd.run", params)["result"].clone()
    }

    pub fn error_code(&mut self, command: &str, args: Value) -> Value {
        self.run(command, args)["error"]["code"].clone()
    }
}

#[test]
fn a_session_answers_over_its_socket_and_logs_what_ran() {
    let server = Server::start(
        "round-trip_1",
        &["--seed", "5", "--history-mib", "3", "--verify"],
    );
    let mut a = server.client("agent-a", "agent");
    let hello = a.rpc("engine.hello", json!({ "client": "agent-a" }))["result"].clone();
    assert_eq!(hello["session"], json!("round-trip_1"), "session name");
    assert_eq!(
        (hello["protocol"].clone(), hello["role"].clone()),
        (json!(1), json!("agent")),
        "defaults"
    );
    let catalog = a.rpc("catalog.get", json!({}))["result"].clone();
    assert_eq!(
        catalog["hash"], hello["catalog_hash"],
        "hello names the catalog it serves"
    );
    assert!(
        catalog["commands"]
            .as_array()
            .is_some_and(|c| c.len() >= 20),
        "the full catalog"
    );
    let made = a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    assert_eq!(
        (made["seq"].clone(), made["rev"].clone()),
        (json!(1), json!(1)),
        "first edit"
    );
    assert_eq!(made["ok"]["result"]["part"], json!("sphere"), "result");
    let info = a.run("session.info", json!({}))["ok"]["result"].clone();
    assert_eq!(info["seed"], json!(5), "--seed reaches the engine");
    assert_eq!(
        info["history_budget_bytes"],
        json!(3 << 20),
        "--history-mib reaches the engine"
    );
    assert_eq!(
        info["log_path"],
        json!(server.log.display().to_string()),
        "--log reaches the engine"
    );
    let hash = a.run("mesh.hash", json!({}))["ok"]["result"]["hash"].clone();
    let log = server.log.clone();
    assert_eq!(server.stop(), ExitCode::SUCCESS, "a clean shutdown");
    let lines = std::fs::read_to_string(&log).unwrap_or_default();
    let read = |n: usize| -> Value {
        serde_json::from_str(lines.lines().nth(n).unwrap_or_default()).unwrap_or_default()
    };
    assert_eq!(
        (read(0)["session"].clone(), read(0)["verify"].clone()),
        (json!("round-trip_1"), json!(true)),
        "a header, with --verify"
    );
    let record = read(1);
    assert_eq!(
        (
            record["seq"].clone(),
            record["command"].clone(),
            record["hash"].clone()
        ),
        (json!(1), json!("primitive.sphere"), hash),
        "logged, with the scene hash after it"
    );
    assert!(
        record["client"]
            .as_str()
            .is_some_and(|c| c.starts_with("agent-a#")),
        "who: {record}"
    );
    assert_eq!(lines.lines().count(), 2, "queries are not logged");
}

#[test]
fn clients_learn_about_each_others_changes() {
    let server = Server::start("concurrent", &[]);
    let (mut a, mut b) = (server.client("a", "agent"), server.client("b", "agent"));
    assert_eq!(
        a.run("primitive.cube", json!({}))["concurrent_changes"],
        json!(0),
        "nothing before"
    );
    assert_eq!(
        b.run("mesh.stats", json!({}))["concurrent_changes"],
        json!(1),
        "b sees a's edit"
    );
    assert_eq!(
        b.run("mesh.stats", json!({}))["concurrent_changes"],
        json!(0),
        "only once"
    );
    b.run("primitive.sphere", json!({}));
    b.run("history.undo", json!({}));
    let seen = a.run("mesh.stats", json!({}));
    assert_eq!(
        (seen["concurrent_changes"].clone(), seen["rev"].clone()),
        (json!(2), json!(3)),
        "a sees b's two"
    );
    let mut late = server.client("late", "agent");
    assert_eq!(
        late.run("mesh.stats", json!({}))["concurrent_changes"],
        json!(0),
        "counted from hello"
    );
    let mut anonymous = Client::connect(&server.socket);
    let made = anonymous.run("primitive.sphere", json!({ "name": "x" }));
    assert!(made.get("ok").is_some(), "no hello means an agent: {made}");
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}

/// Only the client holding the stop key (the one that spawned the session) can stop it.
#[test]
fn only_the_stop_key_stops_a_session() {
    let server = Server::start("stop-key", &[]);
    let mut observer = server.client("eye", "observer");
    let mut agent = server.client("bot", "agent");
    for (observes, params) in [
        (true, json!({ "key": KEY })),
        (false, json!({})),
        (false, json!({ "key": "guess" })),
    ] {
        let client = if observes { &mut observer } else { &mut agent };
        let refused = client.rpc("engine.shutdown", params.clone());
        assert_eq!(
            refused["error"]["code"],
            json!(-32001),
            "refused with {params}"
        );
        let message = refused["error"]["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains("role"),
            "no hint at another role: {message}"
        );
    }
    assert!(
        agent.run("mesh.stats", json!({})).get("ok").is_some(),
        "still running"
    );
    assert_eq!(server.stop(), ExitCode::SUCCESS, "the key stops it");
}

#[test]
fn one_server_per_socket_and_stale_sockets_are_replaced() {
    let server = Server::start("taken", &[]);
    let socket = server.socket.display().to_string();
    let second = briefly(&[
        "serve",
        "--headless",
        "--session",
        "taken",
        "--socket",
        &socket,
    ]);
    assert_eq!(
        second,
        Some(ExitCode::FAILURE),
        "a running session's socket is not taken over"
    );
    assert_eq!(
        server.stop(),
        ExitCode::SUCCESS,
        "the first one still stops cleanly"
    );
    let stale = socket_path("stale");
    std::fs::write(&stale, "left over from a crash").unwrap_or_else(|e| panic!("{e}"));
    let restarted = Server::start_at(&stale, "stale", &[]);
    assert_eq!(
        restarted.stop(),
        ExitCode::SUCCESS,
        "a stale socket file is replaced"
    );
}

#[test]
fn the_command_line_checks_its_arguments() {
    assert_eq!(briefly(&[]), Some(ExitCode::from(2)), "no window yet");
    assert_eq!(
        briefly(&["serve", "--session", "x"]),
        Some(ExitCode::from(2)),
        "serve needs --headless"
    );
    assert_eq!(briefly(&["--help"]), Some(ExitCode::SUCCESS), "help");
    assert_eq!(
        briefly(&["serve", "--headless", "--bogus"]),
        Some(ExitCode::from(2)),
        "unknown flag"
    );
    let socket = socket_path("names").display().to_string();
    for bad in ["a/b", "", "a b", "ü", "a.b", "../escape"] {
        let code = briefly(&["serve", "--headless", "--session", bad, "--socket", &socket]);
        assert_eq!(code, Some(ExitCode::FAILURE), "bad name {bad:?}");
    }
}

/// The engine forgets a client when its connection closes, so long sessions do not grow with
/// every `gs` call.
#[test]
fn disconnected_clients_are_forgotten() {
    let server = Server::start("goodbye", &[]);
    let mut a = server.client("a", "agent");
    let mut count = || a.run("session.info", json!({}))["ok"]["result"]["clients"].clone();
    {
        let _b = server.client("b", "agent");
        let _c = server.client("c", "agent");
        assert_eq!(count(), json!(3), "a, b and c");
    }
    let mut seen = Value::Null;
    for _ in 0..PATIENCE.as_millis() / 10 {
        seen = count();
        if seen == json!(1) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen, json!(1), "b and c are forgotten once they leave");
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}
