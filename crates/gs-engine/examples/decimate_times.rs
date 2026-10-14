//! Times `mesh.decimate` to 10% through the engine (decision 0034) on a sphere subdivided to one
//! level: the whole call, wall and CPU (every thread, from `/proc/self/stat`, Linux's 100 ticks a
//! second). Medians of 3, with an undo between.
//!
//! ```text
//! gate/slot.sh cargo run --release -p gs-engine --example decimate_times -- [subdivisions] [levels]
//! ```
//!
//! `204 1` gives 998,784 quads (the default); `228 2` gives 4,990,464.

use std::io::Write;
use std::time::Instant;

use gs_testkit::Session;
use serde_json::json;

#[expect(clippy::disallowed_methods, reason = "a timing tool reads the clock")]
fn now() -> Instant {
    Instant::now()
}

/// CPU seconds the process has used so far, every thread.
fn cpu() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // The fields after the command name, which sits in parentheses and may hold spaces: the
    // first is field 3, so utime (14) and stime (15) are at 11 and 12.
    let rest = stat.rsplit_once(')').map_or("", |r| r.1);
    let fields: Vec<f64> = (rest.split_whitespace())
        .map(|f| f.parse().unwrap_or(0.0))
        .collect();
    fields.get(11..13).map_or(0.0, |t| (t[0] + t[1]) / 100.0)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<u32>().ok());
    let n = args.next().flatten().unwrap_or(204);
    let levels = args.next().flatten().unwrap_or(1);
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": n }));
    for _ in 0..levels {
        s.ok("level.subdivide", json!({}));
    }
    if levels > 0 {
        s.ok("level.delete_lower", json!({}));
    }
    let (mut wall, mut cpus, mut reply) = (Vec::new(), Vec::new(), json!(null));
    for _ in 0..3 {
        let (t, c) = (now(), cpu());
        reply = s.ok("mesh.decimate", json!({ "percent": 10 })).result;
        wall.push(t.elapsed().as_secs_f64());
        cpus.push(cpu() - c);
        s.ok("history.undo", json!({}));
    }
    let line = format!(
        "sphere {n}, {levels} levels: {} faces to {}, wall {:.3} s, cpu {:.3} s",
        reply["faces_before"],
        reply["faces"],
        median(wall),
        median(cpus)
    );
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}
