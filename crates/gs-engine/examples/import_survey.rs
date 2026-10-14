//! Imports a list of files with `file.import`, each in a fresh engine in a process of its own,
//! and reports per file whether it imported, how many parts, vertices and faces it made, the
//! subdivision levels not kept, the time and the process's peak memory, then the totals.
//!
//! For checking imports against a private collection that never enters the repository:
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example import_survey -- <list> [<report>]
//! ```
//!
//! `<list>` holds one absolute path per line. Lines of the report name files by their place in
//! the list only.

#[path = "../../gs-render/examples/peak/mod.rs"]
mod peak;

use std::io::Write;
use std::process::Command as Process;
use std::time::Instant;

use gs_engine::{Engine, EngineConfig, Origin, Request};
use gs_protocol::{Outcome, Role, RunParams};
use serde_json::{Value, json};

macro_rules! say {
    ($out:expr, $($t:tt)*) => {{
        let line = format!($($t)*);
        if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
            panic!("cannot write: {e}");
        }
        $out.push(line);
    }};
}

#[expect(clippy::disallowed_methods, reason = "a survey reads the clock")]
fn now() -> Instant {
    Instant::now()
}

/// Peak resident memory of this process in MiB, from `/proc/self/status`.
fn peak_mib() -> f64 {
    peak::peak_kib() / 1024.0
}

/// Imports one file and prints one JSON line about it.
fn one(path: &str) {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let origin = Origin {
        client_id: 1,
        client: "survey".into(),
        role: Role::Human,
    };
    let t = now();
    let response = engine.execute(
        &origin,
        Request::Run(RunParams::new("file.import", json!({ "path": path }))),
    );
    let seconds = t.elapsed().as_secs_f64();
    let line = match response.outcome {
        Outcome::Ok(done) => {
            let parts = done.result["parts"].as_array().cloned().unwrap_or_default();
            let sum = |key: &str| parts.iter().filter_map(|p| p[key].as_u64()).sum::<u64>();
            let not_kept: u64 = (parts.iter())
                .filter_map(|p| p["levels"]["not_kept"].as_array().map(|l| l.len() as u64))
                .sum();
            let with = |column: &str| {
                let has = |p: &&Value| p["columns"].to_string().contains(column);
                parts.iter().filter(has).count()
            };
            json!({
                "ok": true, "parts": parts.len(), "vertices": sum("vertices"), "faces": sum("faces"),
                "levels_not_kept": not_kept, "with_uvs": with("uv0.u"), "with_groups": with("\"group\""),
                "with_mask": with("\"mask\""), "with_color": with("\"color\""),
                "notes": done.result["notes"].as_array().map_or(0, Vec::len),
                "seconds": seconds, "peak_mib": peak_mib(),
            })
        }
        Outcome::Error(e) => json!({
            "ok": false, "code": e.code, "message": e.message, "seconds": seconds, "peak_mib": peak_mib(),
        }),
    };
    say!(Vec::new(), "{line}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--one") {
        one(&args[2]);
        return;
    }
    let list = std::fs::read_to_string(&args[1]).unwrap_or_else(|e| panic!("{e}"));
    let me = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let mut out = Vec::new();
    let mut totals = [0f64; 8];
    for (i, path) in list.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let child = Process::new(&me).args(["--one", path]).output();
        let text = child.map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let row: Value = text
            .ok()
            .and_then(|t| serde_json::from_str(t.trim()).ok())
            .unwrap_or(json!({ "ok": false, "message": "the import process died" }));
        let ok = row["ok"].as_bool() == Some(true);
        let get = |k: &str| row[k].as_f64().unwrap_or(0.0);
        totals[0] += 1.0;
        totals[1] += f64::from(u8::from(ok));
        for (t, key) in
            totals[2..]
                .iter_mut()
                .zip(["parts", "vertices", "faces", "levels_not_kept", "seconds"])
        {
            *t += get(key);
        }
        totals[7] = totals[7].max(get("peak_mib"));
        say!(out, "file {i}: {row}");
    }
    say!(
        out,
        "\ntotals: {} of {} files imported, {} parts, {} vertices, {} faces, {} levels not kept, {:.1} s, worst peak {:.0} MiB",
        totals[1],
        totals[0],
        totals[2],
        totals[3],
        totals[4],
        totals[5],
        totals[6],
        totals[7]
    );
    if let Some(report) = args.get(2) {
        std::fs::write(report, out.join("\n")).unwrap_or_else(|e| panic!("{e}"));
    }
}
