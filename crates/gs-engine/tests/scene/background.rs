//! Saves and opens in the background for every caller (decision 0016, section 6): the window
//! gets the job's id at once and the save lands later; a socket client's `cmd.run` waits for
//! the end; while a scene opens, edits wait with `E_BUSY`, the human's too.

use gs_protocol::{Outcome, Request, RunParams};
use serde_json::{Value, json};

use crate::{at, folder};

#[expect(
    dead_code,
    reason = "these tests use part of the jobs tests' session helpers"
)]
#[path = "../jobs/support.rs"]
mod support;

/// Runs a command as the human at the window and gives the answer's result, or the error code.
fn window(s: &support::Session, command: &str, args: Value) -> Result<Value, String> {
    let request = Request::Run(RunParams::new(command, args));
    let response = s.window().execute(request).expect("the engine runs");
    match response.outcome {
        Outcome::Ok(done) => Ok(done.result),
        Outcome::Error(e) => Err(json!(e.code).as_str().unwrap_or_default().to_string()),
    }
}

#[test]
fn the_windows_save_answers_at_once_and_lands_as_a_job() {
    let dir = folder("window-save");
    let path = at(&dir, "scene.gss");
    let s = support::Session::start("scene-window-save");
    s.as_human("primitive.sphere", json!({ "subdivisions": 8 }));
    let started = window(&s, "file.save", json!({ "path": path })).expect("started");
    assert_eq!(started["working"], json!(true), "{started}");
    let id = started["job"].clone();
    let mut agent = s.client();
    let report = agent.status(&id, 30_000, false);
    assert!(
        report["response"]["ok"].is_object(),
        "the save succeeded: {report}"
    );
    assert_eq!(report["response"]["ok"]["result"]["path"], json!(path));
    let waited = agent.run("file.save", json!({}));
    assert_eq!(
        waited["ok"]["result"]["path"],
        json!(path),
        "a socket client's cmd.run waits for the end, and the landing made it the scene's file"
    );
}

#[test]
fn edits_wait_while_a_scene_opens() {
    let dir = folder("window-busy");
    let path = at(&dir, "big.gss");
    let s = support::Session::start("scene-window-busy");
    s.as_human("primitive.sphere", json!({ "subdivisions": 160 }));
    let mut agent = s.client();
    agent.run("file.save", json!({ "path": path }));
    let started = window(&s, "file.open", json!({ "path": path })).expect("started");
    let busy = window(&s, "primitive.cube", json!({}));
    assert_eq!(
        busy,
        Err("E_BUSY".into()),
        "the human's edit waits while the scene opens"
    );
    let looked = window(&s, "mesh.stats", json!({}));
    assert!(looked.is_ok(), "queries still answer: {looked:?}");
    let report = agent.status(&started["job"], 60_000, false);
    assert!(
        report["response"]["ok"].is_object(),
        "the open landed: {report}"
    );
    assert!(
        window(&s, "primitive.cube", json!({})).is_ok(),
        "then edits run again"
    );
}
