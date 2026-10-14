//! What the engine's timing tools share: one whole `Engine::execute` call, timed.

use std::time::Instant;

use gs_engine::{Engine, Origin, Request};
use gs_protocol::{Outcome, Role, RunParams};
use serde_json::Value;

#[expect(clippy::disallowed_methods, reason = "a timing tool reads the clock")]
fn now() -> Instant {
    Instant::now()
}

/// Runs one command and gives the milliseconds the whole call took.
pub fn call(engine: &mut Engine, command: &str, args: Value) -> f64 {
    let origin = Origin {
        client_id: 1,
        client: "timing".into(),
        role: Role::Human,
    };
    let t = now();
    let response = engine.execute(&origin, Request::Run(RunParams::new(command, args)));
    let ms = t.elapsed().as_secs_f64() * 1e3;
    if let Outcome::Error(e) = response.outcome {
        panic!("{command}: {}", e.message);
    }
    ms
}
