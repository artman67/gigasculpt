//! Times decision 0057's release re-run (section 10) through the engine, as Live Boolean's live
//! mode runs it (step LB6): research 26's 1M body (`sphere(289)`, 1,002,252 triangles) minus its
//! 20k cutter (`sphere(41)`, 20,172 triangles) placed by `shapes::cutter()`, the live mode on.
//! The first run imports both operands; each later one follows a move of the cutter, so both
//! come from the cache, on `gs_par::background`'s 4 threads. Each time is the move's call and
//! the wait for its run to land (`Engine::tick`). Then `boolean.apply` with the settled result,
//! and the same Apply with the live mode off for comparison.
//!
//! ```text
//! gate/heavy.sh cargo run --release --locked -p gs-engine --example live_probe -- [moves]
//! ```

#[expect(dead_code, reason = "shared with gs-geo's tests, which use the rest")]
#[path = "../../gs-geo/tests/boolean/shapes.rs"]
mod shapes;

use std::io::Write;
use std::time::Instant;

use gs_testkit::Session;
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

/// Runs `command`, lands every live run, and gives the seconds both took and the live state.
fn landed(s: &mut Session, command: &str, args: Value) -> (f64, Value) {
    let start = now();
    s.ok(command, args);
    let _ = s.engine.tick();
    let took = start.elapsed().as_secs_f64();
    (
        took,
        s.query("session.state", json!({}))["boolean.live"].clone(),
    )
}

/// Seconds `boolean.apply` took, and whether it took the live result.
fn apply(s: &mut Session) -> (f64, Value) {
    let start = now();
    let out = s.ok("boolean.apply", json!({}));
    (
        start.elapsed().as_secs_f64(),
        out.result["groups"][0]["live"].clone(),
    )
}

fn main() {
    let moves: usize = (std::env::args().nth(1)).map_or(5, |a| a.parse().unwrap_or(5));
    let mut s = Session::new();
    s.add_part("body", &shapes::sphere(289));
    s.add_part("cutter", &shapes::sphere(41));
    let c = shapes::cutter();
    let frame = json!({ "origin": [0.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] });
    let scale =
        json!({ "part": "cutter", "frame": frame, "op": { "scale": [c.scale, c.scale, c.scale] } });
    s.ok("transform.apply", scale);
    let to = json!({ "part": "cutter", "op": { "move": c.translation.to_array() } });
    s.ok("transform.apply", to);
    let difference = json!({ "part": "cutter", "operation": "difference" });
    s.ok("part.set_operation", difference);
    let on = json!({ "live_boolean": true });
    let (first, live) = landed(&mut s, "view.display", on);
    say(&format!(
        "first run, both operands imported: {first:.3} s; {live}"
    ));
    let mut times = Vec::new();
    for i in 0..moves {
        let by = if i % 2 == 0 { 0.01 } else { -0.01 };
        let step = json!({ "part": "cutter", "op": { "move": [0.0, by, 0.0] } });
        let (took, live) = landed(&mut s, "transform.apply", step);
        let group = &live["groups"][0];
        say(&format!(
            "move {}: {took:.3} s, {}, {} faces, {} points, solver {}, imports {}",
            i + 1,
            group["state"],
            group["faces"],
            group["points"],
            group["solver"],
            live["imports"]
        ));
        times.push(took);
    }
    times.sort_by(f64::total_cmp);
    let median = times.get(times.len() / 2).copied().unwrap_or(f64::NAN);
    say(&format!("median of {moves} re-runs: {median:.3} s"));
    let (taken, live) = apply(&mut s);
    say(&format!(
        "boolean.apply with the settled result: {taken:.3} s (live: {live})"
    ));
    s.ok("history.undo", json!({}));
    s.ok("view.display", json!({ "live_boolean": false }));
    let (fresh, live) = apply(&mut s);
    say(&format!(
        "boolean.apply with the live mode off: {fresh:.3} s (live: {live})"
    ));
}
