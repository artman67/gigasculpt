//! The in-process session reports exactly what the engine did, and never hides a failure.

use gs_engine::{EngineConfig, Request};
use gs_protocol::{ErrorCode, Role, RunParams};
use gs_testkit::{Session, code, failure, success, text};
use serde_json::json;

use crate::data::tetra;

/// The example from research 04, section 12, with the edit and the reads around it.
#[test]
#[expect(
    clippy::result_large_err,
    reason = "the example propagates the engine's own error type with ?"
)]
fn a_session_runs_commands_and_reads_the_results_back() -> Result<(), gs_protocol::CommandError> {
    let mut s = Session::headless_seeded(1);
    let empty = s.hash();
    assert!(
        empty.len() == 64 && empty.chars().all(|c| c.is_ascii_hexdigit()),
        "a full hash: {empty}"
    );
    s.run("primitive.sphere", json!({ "subdivisions": 5 }))?;
    let before = s.hash();
    assert_ne!(before, empty, "the sphere changed the scene");
    s.run("mesh.jitter", json!({ "amount": "1%" }))?;
    assert_ne!(s.hash(), before, "the jitter moved something");
    s.run("history.undo", json!({}))?;
    assert_eq!(s.hash(), before, "undo put it back");
    assert_eq!(s.history()["undoable"], json!(1), "one step left");
    assert_eq!(s.query("session.info", json!({}))["seed"], json!(1), "seed");
    let sphere = s.mesh("sphere");
    assert_eq!(sphere.positions.len(), 6 * 25 + 2, "the sphere's vertices");
    s.add_part("t", &tetra());
    let scene = s.scene();
    let names: Vec<&String> = scene.keys().collect();
    assert_eq!(names, ["sphere", "t"], "every part, by name");
    assert_eq!(scene["t"].faces, tetra().faces, "read back as made");
    assert_eq!(scene["sphere"].positions, sphere.positions, "same data");
    Ok(())
}

#[test]
fn failures_come_back_as_errors_with_their_codes() {
    let mut s = Session::new();
    let missing = json!({ "part": "zz", "direction": [1, 0, 0], "distance": 1 });
    let err = s.run("mesh.translate", missing.clone()).err();
    assert_eq!(
        err.map(|e| code(e.code)),
        Some("E_NOT_FOUND".to_string()),
        "run returns the error"
    );
    let err = s.err("mesh.translate", missing.clone());
    assert_eq!(err.code, ErrorCode::NotFound, "err returns it too");
    let err = failure(s.call("mesh.translate", missing), "translate");
    assert_eq!(code(err.code), "E_NOT_FOUND", "and so does failure");
    let done = success(s.call("primitive.cube", json!({})), "cube");
    assert_eq!(
        done.result["vertices"],
        json!(8),
        "success returns the result"
    );
    assert_eq!(
        s.ok("mesh.stats", json!({})).result["parts"][0]["faces"],
        json!(6),
        "ok too"
    );
}

#[test]
#[should_panic(expected = "mesh.translate failed: E_NOT_FOUND")]
fn ok_refuses_a_failure() {
    Session::new().ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1 }),
    );
}

#[test]
#[should_panic(expected = "primitive.cube succeeded but should fail")]
fn err_refuses_a_success() {
    Session::new().err("primitive.cube", json!({}));
}

#[test]
#[should_panic(expected = "expected text, got 3")]
fn text_refuses_a_number() {
    text(&json!(3));
}

#[test]
fn codes_and_text_read_the_wire_form() {
    assert_eq!(code(ErrorCode::Precondition), "E_PRECONDITION", "code");
    assert_eq!(text(&json!("abc")), "abc", "text");
}

/// The configuration and role given to a session are the ones the engine gets.
#[test]
fn seeds_and_roles_reach_the_engine() {
    let config = EngineConfig {
        seed: 77,
        ..EngineConfig::default()
    };
    let mut observer = Session::with(config, Role::Observer);
    assert_eq!(
        observer.query("session.info", json!({}))["seed"],
        json!(77),
        "seed"
    );
    let err = failure(observer.call("primitive.cube", json!({})), "observer edit");
    assert_eq!(code(err.code), "E_PRECONDITION", "an observer cannot edit");
    for mut s in [Session::new(), Session::default()] {
        assert_eq!(
            s.query("session.info", json!({}))["seed"],
            json!(0),
            "seed 0"
        );
        s.ok("primitive.cube", json!({}));
    }
}

#[test]
fn batches_and_raw_requests_go_through() {
    let mut s = Session::new();
    let done = success(
        s.batch(&[
            ("primitive.cube", json!({ "name": "a" })),
            ("primitive.cube", json!({ "name": "b" })),
        ]),
        "batch",
    );
    assert_eq!(done.result.as_array().map(Vec::len), Some(2), "two outputs");
    assert_eq!(s.history()["undoable"], json!(1), "one step");
    let dry = Request::Run(RunParams::new("primitive.sphere", json!({})).dry());
    let done = success(s.request(dry), "dry run");
    assert_eq!(
        done.result,
        json!({ "dry_run": true }),
        "the request arrived as sent"
    );
    assert_eq!(s.scene().len(), 2, "and changed nothing");
}
