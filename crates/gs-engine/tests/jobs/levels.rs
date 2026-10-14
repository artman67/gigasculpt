//! A part with detail levels in a job (decision 0014 with decision 0010): a job works on a share
//! of the whole level stack, so level changes, strokes on a lower level and carrying them up
//! give over the socket exactly the bits they give in process, and land as the same steps.

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::{Value, json};

use crate::support::{Session, stroke};

/// A world-space Draw stroke over the sphere near `at`.
fn step(command: &str, args: &Value) -> Value {
    json!({ "command": command, "args": args })
}

#[test]
fn a_job_on_a_part_with_levels_gives_the_same_bits_as_in_place() {
    let s = Session::start("levels");
    let mut a = s.client();
    let config = EngineConfig {
        session: "levels".into(),
        ..EngineConfig::default()
    };
    let mut local = gs_testkit::Session::with(config, Role::Agent);
    let setup = [
        ("primitive.sphere", json!({ "subdivisions": 5 })),
        ("level.subdivide", json!({})),
        ("level.set", json!({ "level": 1 })),
        ("brush.stroke", stroke([0.0, 0.0, 1.0])),
    ];
    for (command, args) in setup {
        a.run(command, args.clone());
        local.ok(command, args);
    }
    assert_eq!(a.hash(), json!(local.hash()), "the same start");
    // A subdivide (a long step, so a job) carries the pending level-1 stroke up first; then a
    // stroke on the new top, a switch down, a stroke there and a step back up.
    let steps = [
        step("level.subdivide", &json!({})),
        step("brush.stroke", &stroke([0.3, 0.0, 0.95])),
        step("level.set", &json!({ "level": 2 })),
        step("brush.stroke", &stroke([0.0, 0.3, 0.95])),
        step("level.step", &json!({ "by": 1 })),
        step("level.describe", &json!({})),
    ];
    let batch = json!({ "commands": steps });
    let job = a.result("job.start", json!({ "batch": batch, "wait_ms": 120_000 }));
    assert!(job["id"].is_u64(), "a job: {job}");
    let pairs: Vec<(&str, Value)> = (steps.iter())
        .map(|s| (s["command"].as_str().unwrap_or_default(), s["args"].clone()))
        .collect();
    let here = json!(local.batch(&pairs));
    assert_eq!(
        job["response"]["ok"], here["ok"],
        "every output, the effect and the step"
    );
    assert_eq!(a.hash(), json!(local.hash()), "the scene after it");
    a.run("history.undo", json!({}));
    local.ok("history.undo", json!({}));
    assert_eq!(a.hash(), json!(local.hash()), "and after undoing it");
}

#[test]
fn an_in_place_job_lands_on_a_lower_level_and_a_level_change_makes_it_stale() {
    let s = Session::start("levels-slow");
    let (mut a, mut b) = (s.client(), s.client());
    let config = EngineConfig {
        session: "levels-slow".into(),
        ..EngineConfig::default()
    };
    let mut local = gs_testkit::Session::with(config, Role::Agent);
    let slow = json!({ "part": "sphere", "steps": 1, "step_ms": 0, "offset": 0.25 });
    for (command, args) in [
        ("primitive.sphere", json!({ "subdivisions": 4 })),
        ("level.subdivide", json!({})),
        ("level.set", json!({ "level": 1 })),
        ("test.slow", slow.clone()),
    ] {
        a.run(command, args.clone());
        local.ok(command, args);
    }
    assert_eq!(
        a.hash(),
        json!(local.hash()),
        "an in-place job on level 1 lands as in place"
    );
    let waiting = json!({ "part": "sphere", "steps": 40, "step_ms": 25, "offset": 0.25 });
    let id = a.start("test.slow", waiting, 0)["id"].clone();
    b.run("level.set", json!({ "level": 2 }));
    let changed = b.hash();
    let done = a.status(&id, 30_000, false);
    assert_eq!(
        done["response"]["error"]["code"],
        json!("E_STALE"),
        "{done}"
    );
    assert_eq!(a.hash(), changed, "and nothing applied");
}
