//! What it costs to start a job in a session holding a big part (decision 0010): a session server
//! gets a part of `--quads` quads (default 5,000,000, which is 10M triangles), then a socket
//! client runs `file.import` of a one-triangle file several times. Each of those is a job, so its
//! time is the job's start (copying the session), the tiny import and the commit. Prints each
//! time and the median.
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example job_start -- <scratch folder> [--quads N]
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    let options = WriteOptions::default();
    let out = std::io::BufWriter::new(file);
    gs_io::ply::write(out, &[object], &options).unwrap_or_else(|e| panic!("{e}"));
    say(&format!(
        "big part: {} vertices, {} quads",
        mesh.positions.len(),
        mesh.faces.len()
    ));
    path
}

fn line(stream: &mut UnixStream, reader: &mut BufReader<UnixStream>, request: &Value) -> Value {
    writeln!(stream, "{request}").unwrap_or_else(|e| panic!("send: {e}"));
    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .unwrap_or_else(|e| panic!("reply: {e}"));
    serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(
        args.get(1)
            .unwrap_or_else(|| panic!("give a scratch folder")),
    );
    let quads = (args.iter().position(|a| a == "--quads"))
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(5_000_000u64);
    std::fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
    let big = big_file(&folder, quads);
    let tiny = folder.join("tiny.obj");
    std::fs::write(&tiny, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_or_else(|e| panic!("{e}"));

    let socket = folder.join(format!("job-start-{}.sock", std::process::id()));
    let options = ServeOptions {
        socket: socket.clone(),
        stop_key: None,
        max_line_bytes: 1 << 20,
    };
    let server = Server::start(EngineConfig::default(), &options).unwrap_or_else(|e| panic!("{e}"));
    let mut stream = UnixStream::connect(&socket).unwrap_or_else(|e| panic!("{e}"));
    let mut reader = BufReader::new(stream.try_clone().unwrap_or_else(|e| panic!("{e}")));
    let hello = json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.hello",
        "params": { "client": "job-start", "role": "agent" } });
    line(&mut stream, &mut reader, &hello);
    // Through the socket, whose call answers once the import lands; the window's would answer
    // at once with its job (decision 0010 as amended 2026-10-13).
    let made = json!({ "jsonrpc": "2.0", "id": 2, "method": "cmd.run", "params": {
        "command": "file.import", "args": { "path": big } } });
    let made = line(&mut stream, &mut reader, &made);
    assert!(made["result"]["ok"].is_object(), "import: {made}");
    let mut times = Vec::new();
    for i in 0..7 {
        let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "cmd.run", "params": {
            "command": "file.import", "args": { "path": tiny, "name": format!("tiny{i}") } } });
        let t = now();
        let reply = line(&mut stream, &mut reader, &call);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(reply["result"]["ok"].is_object(), "{reply}");
        say(&format!("job {i}: {ms:.1} ms"));
        times.push(ms);
    }
    times.sort_by(f64::total_cmp);
    say(&format!("median {:.1} ms", times[times.len() / 2]));
    drop(stream);
    drop(server);
    std::fs::remove_file(&big).ok();
    std::fs::remove_file(&socket).ok();
    std::fs::remove_file(socket.with_extension("lock")).ok();
}
