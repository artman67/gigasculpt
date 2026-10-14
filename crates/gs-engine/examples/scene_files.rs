//! Saving and opening a big scene (decision 0016, against ARCHITECTURE 11: open and save of 100
//! million triangles in at most 5 s each, the engine thread never held more than 100 ms by a
//! save). A session server gets a part of `--quads` quads (default 50,000,000, which is 100
//! million triangles), jittered so its positions are not a clean grid. Then a socket client
//! saves it as a job and opens it again, while a second client asks `session.info` every 2 ms
//! and records the longest answer: how long the engine thread was busy.
//!
//! With `--levels L` the part is a stack of L detail levels (decision 0016, section 15): a base
//! of `N / 4^(L-1)` quads divided L-1 times, jittered at the top, then the level below the top
//! made active and jittered again, so every point of it is an edit not carried up yet (the
//! most a save of levels writes).
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example scene_files -- <scratch folder> [--quads N] [--levels L]
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use gs_engine::EngineConfig;
use gs_engine::server::{ServeOptions, Server};
use gs_io::mesh::{Object, WriteOptions};
use gs_protocol::UnixStream;
use serde_json::{Value, json};

#[expect(clippy::disallowed_methods, reason = "a measurement reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn say(line: &str) {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("cannot write: {e}");
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Writes a cube grid of about `quads` quads as a binary PLY.
fn big_file(folder: &Path, quads: u64) -> PathBuf {
    let n = ((quads as f64 / 6.0).sqrt().round() as u32).max(1);
    let mesh = gs_testkit::fixtures::cube(n);
    let path = folder.join(format!("cube-{n}.ply"));
    let file = std::fs::File::create(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let object = Object {
        name: "big",
        mesh: &mesh,
        positions: &mesh.positions,
    };
    let out = std::io::BufWriter::new(file);
    gs_io::ply::write(out, &[object], &WriteOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    say(&format!(
        "part: {} vertices, {} quads ({} triangles)",
        mesh.positions.len(),
        mesh.faces.len(),
        2 * mesh.faces.len()
    ));
    path
}

/// One socket client, introduced as an agent.
struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn new(socket: &Path, name: &str) -> Self {
        let stream = UnixStream::connect(socket).unwrap_or_else(|e| panic!("{e}"));
        let reader = BufReader::new(stream.try_clone().unwrap_or_else(|e| panic!("{e}")));
        let mut c = Self { stream, reader };
        c.call("engine.hello", json!({ "client": name, "role": "agent" }));
        c
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let mut request = json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        request["params"] = params;
        writeln!(self.stream, "{request}").unwrap_or_else(|e| panic!("send: {e}"));
        let mut reply = String::new();
        (self.reader.read_line(&mut reply)).unwrap_or_else(|e| panic!("reply: {e}"));
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }

    fn run(&mut self, command: &str, args: Value) -> Value {
        let mut run = json!({ "command": command });
        run["args"] = args;
        let reply = self.call("cmd.run", run);
        assert!(reply["result"]["ok"].is_object(), "{command}: {reply}");
        reply["result"]["ok"]["result"].clone()
    }
}

/// Asks `session.info` every 2 ms while `busy` is set, and returns the longest answer in ms.
fn watch(socket: PathBuf, busy: Arc<AtomicBool>) -> thread::JoinHandle<f64> {
    thread::spawn(move || {
        let mut c = Client::new(&socket, "watch");
        let mut longest: f64 = 0.0;
        while !busy.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        while busy.load(Ordering::SeqCst) {
            let t = now();
            c.call("cmd.run", json!({ "command": "session.info", "args": {} }));
            longest = longest.max(ms(t));
            thread::sleep(Duration::from_millis(2));
        }
        longest
    })
}

/// Runs `command` as a job, timing its admission (the engine thread taking the copy), its end,
/// and the longest engine-thread answer meanwhile.
fn timed(c: &mut Client, socket: &Path, command: &str, args: Value) -> Value {
    let busy = Arc::new(AtomicBool::new(true));
    let watcher = watch(socket.to_path_buf(), Arc::clone(&busy));
    thread::sleep(Duration::from_millis(50));
    let t = now();
    let mut run = json!({ "command": command });
    run["args"] = args;
    let mut start = json!({ "wait_ms": 0 });
    start["run"] = run;
    let started = c.call("job.start", start);
    let admitted = ms(t);
    let id = started["result"]["id"].clone();
    let done = c.call("job.status", json!({ "id": id, "wait_ms": 600_000 }));
    let total = ms(t);
    busy.store(false, Ordering::SeqCst);
    let longest = watcher.join().unwrap_or_default();
    let response = &done["result"]["response"];
    assert!(response["ok"].is_object(), "{command}: {done}");
    say(&format!(
        "{command}: {total:.0} ms in all; admitted in {admitted:.1} ms; the engine thread's \
         longest answer meanwhile {longest:.1} ms"
    ));
    response["ok"]["result"].clone()
}

/// Divides part `big` into `levels` levels, jitters its top, then makes the level below the
/// top active and jitters it too. Returns the top's quads.
fn stack(c: &mut Client, levels: u64) -> Option<u64> {
    for _ in 1..levels {
        let t = now();
        c.run("level.subdivide", json!({ "part": "big" }));
        say(&format!("level.subdivide: {:.0} ms", ms(t)));
    }
    c.run("mesh.jitter", json!({ "part": "big", "amount": "0.01%" }));
    if levels > 1 {
        c.run("level.set", json!({ "part": "big", "level": levels - 1 }));
        c.run("mesh.jitter", json!({ "part": "big", "amount": "0.01%" }));
    }
    let described = c.run("level.describe", json!({ "part": "big" }));
    let top = &described["levels"][levels as usize - 1];
    say(&format!(
        "levels: {} (active {}, edits pending: {}), top {} vertices, {} quads",
        described["count"],
        described["active"],
        described["pending"],
        top["vertices"],
        top["faces"]
    ));
    top["faces"].as_u64()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(
        args.get(1)
            .unwrap_or_else(|| panic!("give a scratch folder")),
    );
    let option = |name: &str| {
        (args.iter().position(|a| a == name)).and_then(|i| args.get(i + 1)?.parse::<u64>().ok())
    };
    let quads = option("--quads").unwrap_or(50_000_000);
    let levels = option("--levels").unwrap_or(1).clamp(1, 8);
    std::fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
    let big = big_file(&folder, quads >> (2 * (levels - 1)));
    let scene = folder.join("big.gss");
    std::fs::remove_file(&scene).ok();
    let socket = folder.join(format!("scene-{}.sock", std::process::id()));
    let options = ServeOptions {
        socket: socket.clone(),
        stop_key: None,
        max_line_bytes: 1 << 20,
    };
    let server = Server::start(EngineConfig::default(), &options).unwrap_or_else(|e| panic!("{e}"));
    let mut c = Client::new(&socket, "scene-files");
    let t = now();
    // Through the socket, whose call answers once the import lands; the window's would answer
    // at once with its job (decision 0010 as amended 2026-10-13).
    c.run("file.import", json!({ "path": big, "name": "big" }));
    std::fs::remove_file(&big).ok();
    say(&format!("import: {:.0} ms", ms(t)));
    let quads = stack(&mut c, levels).unwrap_or(quads);
    let before = c.run("mesh.hash", json!({}))["hash"].clone();
    let saved = timed(&mut c, &socket, "file.save", json!({ "path": scene }));
    let bytes = saved["bytes"].as_u64().unwrap_or_default();
    let tris = 2.0 * quads as f64 / 1e6;
    say(&format!(
        "file: {:.0} MB ({:.2} MB per million triangles), {:.0} MB of columns before compression",
        bytes as f64 / 1e6,
        bytes as f64 / 1e6 / tris,
        saved["raw_bytes"].as_u64().unwrap_or_default() as f64 / 1e6
    ));
    let open = json!({ "path": scene, "discard_changes": true });
    timed(&mut c, &socket, "file.open", open);
    let after = c.run("mesh.hash", json!({}))["hash"].clone();
    say(&format!("same scene after the open: {}", before == after));
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let peak = status
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .unwrap_or("VmHWM: unknown");
    say(&format!(
        "peak memory (the whole run): {}",
        peak.trim_start_matches("VmHWM:").trim()
    ));
    drop(c);
    drop(server);
    std::fs::remove_file(&scene).ok();
    std::fs::remove_file(gs_io::scene::backup_path(&scene)).ok();
    std::fs::remove_file(&socket).ok();
    std::fs::remove_file(socket.with_extension("lock")).ok();
}
