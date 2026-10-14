//! Times level switches through the engine on a sphere with three levels above it (by default a
//! 50M-triangle top; decision 0014): the whole call, so the part share, the swap, the effect
//! and the history are counted beside gs-core's work. Medians of 3.
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example level_switch -- [subdivisions]
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
    let n: u32 = (std::env::args().nth(1)).map_or(255, |a| a.parse().unwrap_or(255));
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    call(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": n }),
    );
    for _ in 0..3 {
        call(&mut engine, "level.subdivide", json!({}));
    }
    let (mut down, mut up) = (Vec::new(), Vec::new());
    for _ in 0..3 {
        down.push(call(&mut engine, "level.set", json!({ "level": 3 })));
        up.push(call(&mut engine, "level.set", json!({ "level": 4 })));
    }
    let line = format!(
        "sphere {n}: switch top to level 3 {:.1} ms, back to the top with nothing to carry {:.1} ms",
        median(down),
        median(up)
    );
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}
