//! Times the two paths that settle the mask onto its grid when topology or a lower level makes
//! new values (decision 0015, amended 2026-10-13): a subdivision step onto a top of about 5M
//! points with a soft mask on every point (a cube sphere of 228 subdivisions by default, divided
//! once untimed, then the step timed and undone), and the carry of a Mask Pen stroke made one
//! level below that top. The whole call is timed. Medians of 3.
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example mask_grid_times -- [subdivisions]
//! ```

mod timing;

use std::io::Write;

use gs_engine::{Engine, EngineConfig};
use serde_json::json;
use timing::call;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let n: u32 = (std::env::args().nth(1)).map_or(228, |a| a.parse().unwrap_or(228));
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let e = &mut engine;
    call(e, "primitive.sphere", json!({ "subdivisions": n }));
    call(e, "level.subdivide", json!({}));
    call(e, "mask.set", json!({ "value": 0.3, "scope": "all" }));
    let mut divide = Vec::new();
    for _ in 0..3 {
        divide.push(call(e, "level.subdivide", json!({})));
        call(e, "history.undo", json!({}));
    }
    call(e, "level.subdivide", json!({}));
    let mut carry = Vec::new();
    for x in [-0.4, 0.0, 0.4] {
        call(e, "level.set", json!({ "level": 2 }));
        let samples = [[x, -0.2, 1.0], [x, 0.2, 1.0]].map(|p| json!({ "position": p }));
        let pen = json!({ "part": "sphere", "brush": "Mask Pen", "size": 0.8, "samples": samples });
        call(e, "brush.stroke", pen);
        carry.push(call(e, "level.set", json!({ "level": 3 })));
    }
    let line = format!(
        "sphere {n}: subdivision step to the top {:.1} ms, a mask stroke carried to the top {:.1} ms",
        median(divide),
        median(carry)
    );
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}
