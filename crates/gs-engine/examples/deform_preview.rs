//! What an open deformer costs on the CPU at 10M points with the preview on the graphics card,
//! against the CPU path (decision 0041, amended 2026-10-12 for `DF9a`, section 11), on DF4's part
//! and calls (section 8): a quad sphere of 10,077,698 points (`primitive.sphere` cube form at
//! 162, three `level.subdivide`, then `level.delete_lower`), each whole `Engine::execute` timed,
//! with no window. The CPU path is the same deformer mirrored across x, a plane that splits the
//! part, which the card refuses (section 1): today's update, one pass over every point, with
//! each point's offset flipped in its region. Prints the open, the first update, the median and
//! range of the next 8, the Accept, and the peak memory (`VmHWM`, reset before each deformer).
//! The deformers: (`DF9a`)'s bend, and (`DF9b`)'s two slowest on the CPU, a soft 4 by 4 by 4 cage
//! and a curve through 32 points under a gradient (amended 2026-10-13).
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example deform_preview -- [subdivisions]
//! ```

#[path = "../../gs-render/examples/peak/mod.rs"]
mod peak;
mod timing;

use std::io::Write;

use gs_engine::{Engine, EngineConfig};
use serde_json::{Value, json};
use timing::call;

/// The process's peak resident memory in GB.
fn peak_gb() -> f64 {
    peak::peak_kib() / 1e6
}

/// Sets the peak back to the memory held now.
fn reset_peak() {
    if let Err(e) = std::fs::write("/proc/self/clear_refs", "5") {
        panic!("clear_refs: {e}");
    }
}

/// Transform `t` as the window sends it (`DF7a`: the part, the frame and the box), mirrored
/// across x when `mirrored`.
fn placed(mut t: Value, mirrored: bool) -> Value {
    t["part"] = json!("sphere");
    t["frame"] = json!({ "origin": [0.05, 0.03, 0.02], "axes": [0.0, 0.0, 0.0, 1.0] });
    t["box"] = json!({ "lo": [-1.05, -1.03, -1.02], "hi": [0.95, 0.97, 0.98] });
    if mirrored {
        t["symmetry"] = json!({ "mirror": { "x": true } });
    }
    t
}

/// Step `k` of a drag of a bend: `6 k` degrees toward x along y, none at the open.
fn bend(k: f64) -> Value {
    json!({ "op": { "bend": { "axis": "y", "toward": "x", "degrees": 6.0 * k } } })
}

/// Step `k` of a drag of a soft 4 by 4 by 4 cage: one inner point lifted `0.02 k`, two more
/// moved and held.
fn cage(k: f64) -> Value {
    let offsets = json!([
        { "at": [1, 2, 1], "by": [0.0, 0.02 * k, 0.0] },
        { "at": [2, 1, 2], "by": [0.1, 0.0, -0.05] },
        { "at": [2, 3, 1], "by": [0.0, 0.0, 0.1] },
    ]);
    json!({ "op": { "lattice": { "kind": "soft", "size": [4, 4, 4], "offsets": offsets } } })
}

/// Step `k` of a drag of a curve through 32 points along y, a wave across the part, twisted
/// `10 k` degrees from end to end, faded along y by a gradient.
fn curve(k: f64) -> Value {
    let points: Vec<[f64; 3]> = (0..32)
        .map(|i| {
            let y = -1.03 + 2.0 * f64::from(i) / 31.0;
            [
                0.2 * libm::sin(3.0 * y),
                y,
                0.15 * libm::cos(2.0 * y) - 0.15,
            ]
        })
        .collect();
    let twist: Vec<f64> = (0..32).map(|i| 10.0 * k * f64::from(i) / 31.0).collect();
    let scale: Vec<f64> = (0..32)
        .map(|i| 1.0 + 0.1 * libm::sin(f64::from(i) / 4.0))
        .collect();
    let curve = json!({ "axis": "y", "points": points, "twist": twist, "scale": scale });
    let gradient = json!({ "axis": "y", "from": 0.0, "to": 1.0 });
    json!({ "op": { "curve": curve }, "gradient": gradient })
}

fn deformer(engine: &mut Engine, phase: &str, t: Option<Value>) -> f64 {
    let mut args = json!({ "phase": phase });
    if let Some(t) = t {
        args["transforms"] = json!([t]);
    }
    call(engine, "gizmo.deformer", args)
}

/// A deformer's transform at drag step `k`, 0 at the open.
type Make = fn(f64) -> Value;

/// One pull of the deformer `make` gives at each step: the open at step 0, nine updates and the
/// Accept, then the Accept undone.
fn pull(engine: &mut Engine, label: &str, make: Make, mirrored: bool) -> String {
    reset_peak();
    let step = |k: u32| Some(placed(make(f64::from(k)), mirrored));
    let open = deformer(engine, "open", step(0));
    let first = deformer(engine, "update", step(1));
    let mut next: Vec<f64> = (2..10)
        .map(|k| deformer(engine, "update", step(k)))
        .collect();
    let accept = deformer(engine, "accept", None);
    let held = peak_gb();
    call(engine, "history.undo", json!({}));
    next.sort_by(f64::total_cmp);
    format!(
        "{label}: open {open:.1} ms, first update {first:.1} ms, next 8 median {:.1} ms \
         ({:.1} to {:.1}), accept {accept:.1} ms, peak {held:.2} GB",
        next[next.len() / 2],
        next[0],
        next[next.len() - 1]
    )
}

fn main() {
    let n: u32 = (std::env::args().nth(1)).map_or(162, |a| a.parse().unwrap_or(162));
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    call(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": n }),
    );
    for _ in 0..3 {
        call(&mut engine, "level.subdivide", json!({}));
    }
    call(&mut engine, "level.delete_lower", json!({}));
    say(&format!("sphere {n}: built, peak {:.2} GB", peak_gb()));
    let deformers: [(&str, Make); 3] = [
        ("bend", bend),
        ("soft cage", cage),
        ("curve of 32 under a gradient", curve),
    ];
    for (name, make) in deformers {
        for (path, mirrored) in [("on the card", false), ("the CPU path", true)] {
            for _ in 0..2 {
                say(&pull(
                    &mut engine,
                    &format!("{name}, {path}"),
                    make,
                    mirrored,
                ));
            }
        }
    }
}

fn say(line: &str) {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}
