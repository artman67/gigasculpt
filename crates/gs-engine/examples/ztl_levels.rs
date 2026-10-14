//! Builds every ZTL part's detail levels the way `file.import` does (decision 0014, section
//! 10) and reports, per part with levels: whether the stack builds (the stored top's
//! faces follow decision 0006's rule), whether the active level shows the saved level's stored
//! points exactly, how many of the saved level's points differ from the top's first points
//! (the edits left pending), how far carrying them up moves the top (relative to its size), and
//! the times. Each file runs in a process of its own.
//!
//! For checking against a private collection that never enters the repository:
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example ztl_levels -- <list> [<report>]
//! ```
//!
//! `<list>` holds one absolute path per line. The report names files by their place in the
//! list only.

use std::fs::File;
use std::io::{BufReader, Write};
use std::process::Command as Process;
use std::time::Instant;

use gs_core::{Levels, NodeConfig, PosRead, Settings};
use gs_io::ztl::MeshData;
use serde_json::{Value, json};

#[expect(clippy::disallowed_methods, reason = "a survey reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn print(line: &str) {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("cannot write: {e}");
    }
}

/// The active or top level's positions in canonical order.
fn points(l: &Levels, top: bool) -> Vec<[f32; 3]> {
    let (store, num) = if top {
        (l.top(), l.level_numbering(l.top_level()))
    } else {
        (l.store(), l.numbering())
    };
    let p = store.pos();
    num.ids().iter().map(|&v| p.get(v).to_array()).collect()
}

fn diagonal(p: &[[f32; 3]]) -> f64 {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for q in p {
        for k in 0..3 {
            lo[k] = lo[k].min(f64::from(q[k]));
            hi[k] = hi[k].max(f64::from(q[k]));
        }
    }
    (0..3)
        .map(|k| (hi[k] - lo[k]) * (hi[k] - lo[k]))
        .sum::<f64>()
        .sqrt()
}

/// One part's report.
fn part_report(mut m: MeshData) -> Value {
    let top = m.levels.last().map_or(0, |l| l.level);
    if top == 0 {
        return json!({ "levels": 1 });
    }
    let base = m.base_faces().to_vec();
    let top_mesh = m.levels.pop().map(|l| l.mesh).unwrap_or_default();
    let saved = m.levels.pop().filter(|l| l.level < top);
    let settings = Settings::default();
    let t = now();
    let built = Levels::from_top(
        &base,
        m.base_vertex_count,
        top as usize + 1,
        &top_mesh,
        &settings,
        NodeConfig::default(),
    );
    let build_s = t.elapsed().as_secs_f64();
    let mut l = match built {
        Ok(l) => l,
        Err(e) => return json!({ "levels": top + 1, "built": false, "error": e.to_string() }),
    };
    let mut row = json!({
        "levels": top + 1, "built": true, "top_points": top_mesh.positions.len(),
        "build_s": build_s,
    });
    let Some(saved) = saved else {
        return row;
    };
    let t = now();
    let ok = l.set(saved.level as usize);
    let gather_s = t.elapsed().as_secs_f64();
    if let Err(e) = ok.and_then(|_| l.load_active(&saved.mesh)) {
        row["error"] = json!(e.to_string());
        return row;
    }
    let shown = points(&l, false);
    let bits = |p: &[f32; 3]| p.map(f32::to_bits);
    let exact = shown
        .iter()
        .map(bits)
        .eq(saved.mesh.positions.iter().map(bits));
    let prefix = &top_mesh.positions[..shown.len()];
    let pending = (shown.iter().zip(prefix))
        .filter(|(a, b)| bits(a) != bits(b))
        .count();
    let t = now();
    let carried = l.set(l.top_level());
    let carry_s = t.elapsed().as_secs_f64();
    let after = points(&l, true);
    let moved = (after.iter().zip(&top_mesh.positions))
        .map(|(a, b)| {
            let d = |k: usize| f64::from(a[k] - b[k]);
            (0..3).map(|k| d(k) * d(k)).sum::<f64>().sqrt()
        })
        .fold(0.0, f64::max);
    let changed = (after.iter().zip(&top_mesh.positions))
        .filter(|(a, b)| bits(a) != bits(b))
        .count();
    row["saved_level"] = json!(saved.level);
    row["saved_points"] = json!(shown.len());
    row["saved_exact"] = json!(exact);
    row["pending_points"] = json!(pending);
    row["carried_ok"] = json!(carried.is_ok());
    row["top_points_changed"] = json!(changed);
    row["top_moved_of_size"] = json!(moved / diagonal(&top_mesh.positions).max(1e-30));
    row["gather_s"] = json!(gather_s);
    row["carry_s"] = json!(carry_s);
    row
}

/// Reports every part of one file, one JSON line each.
fn one(path: &str) {
    let file = File::open(path).unwrap_or_else(|e| panic!("{e}"));
    let mut z = gs_io::ztl::open(BufReader::new(file)).unwrap_or_else(|e| panic!("{e}"));
    let jobs: Vec<(usize, usize)> = (0..z.tools.len())
        .flat_map(|t| (0..z.tools[t].parts.len()).map(move |s| (t, s)))
        .collect();
    for (t, s) in jobs {
        let row = match z.geometry(t, s).map(|g| g.mesh) {
            Ok(Some(m)) => part_report(m),
            Ok(None) => json!({ "levels": 0 }),
            Err(e) => json!({ "error": e.to_string() }),
        };
        print(&row.to_string());
    }
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
    for (i, path) in list.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let child = Process::new(&me).args(["--one", path]).output();
        let text = child.map_or_else(
            |e| e.to_string(),
            |o| String::from_utf8_lossy(&o.stdout).into_owned(),
        );
        for line in text.lines().filter(|l| {
            l.contains("\"levels\":")
                && !l.contains("\"levels\":1}")
                && !l.contains("\"levels\":0}")
        }) {
            let line = format!("file {i}: {line}");
            print(&line);
            out.push(line);
        }
    }
    if let Some(report) = args.get(2) {
        std::fs::write(report, out.join("\n")).unwrap_or_else(|e| panic!("{e}"));
    }
}
