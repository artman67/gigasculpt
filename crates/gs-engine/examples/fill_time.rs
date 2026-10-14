//! Times `paint.fill` through the engine on a sphere of about 10M triangles (decision 0039,
//! section 11): a cube sphere of 228 subdivisions by default, subdivided twice, so the top level
//! the fill writes has 192 x 228 x 228 = 9,980,928 triangles. The whole call is timed, so the
//! history and the effect count beside gs-core's mix. Two fills: one at full strength on a part
//! with no colors yet (every point takes the color's bytes) and one at half over it (every point
//! mixed and dithered). Then the process's peak memory.
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-engine --example fill_time -- [subdivisions]
//! ```

use std::io::Write;
use std::time::Instant;

use gs_engine::{Engine, EngineConfig, Origin, Request};
use gs_protocol::{Outcome, Role, RunParams};
use serde_json::{Value, json};

#[expect(clippy::disallowed_methods, reason = "a timing tool reads the clock")]
fn now() -> Instant {
    Instant::now()
}

/// The process's peak resident memory, as Linux reports it (`VmHWM`).
fn peak() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    (status.lines())
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .map_or_else(|| "unknown".into(), |v| v.trim().to_string())
}

fn main() {
    let n: u32 = (std::env::args().nth(1)).map_or(228, |a| a.parse().unwrap_or(228));
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let origin = Origin {
        client_id: 1,
        client: "fill timing".into(),
        role: Role::Human,
    };
    // Runs one command and gives the milliseconds the whole call took.
    let mut run = |command: &str, args: Value| {
        let start = now();
        let done = engine.execute(&origin, Request::Run(RunParams::new(command, args)));
        let ms = start.elapsed().as_secs_f64() * 1e3;
        if let Outcome::Error(e) = done.outcome {
            panic!("{command}: {}", e.message);
        }
        ms
    };
    run("primitive.sphere", json!({ "subdivisions": n }));
    run("level.subdivide", json!({}));
    run("level.subdivide", json!({}));
    let made = peak();
    let first = run("paint.fill", json!({ "color": "#ff0000", "seed": 1 }));
    let half = json!({ "color": "#0000ff", "strength": 0.5, "seed": 2 });
    let second = run("paint.fill", half);
    let line = format!(
        "sphere {n} subdivided twice ({} triangles): fill at full strength from no colors \
         {first:.0} ms, at half over it {second:.0} ms; peak memory {made} before the fills, {} \
         at the end",
        192 * u64::from(n) * u64::from(n),
        peak()
    );
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}
