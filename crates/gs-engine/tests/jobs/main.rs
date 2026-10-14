//! Jobs over the socket (ARCHITECTURE 7.4, `job` in gs-engine): a call with a long command runs
//! on its own thread and commits as one step exactly as it would in-process, reports progress,
//! outlives its client, fails with `E_STALE` when the document moved underneath it, can be
//! cancelled, waits for the human, and ends when the session stops. Differential and socket
//! round-trip tests, driven through the wire methods as clients use them.

mod boolean;
mod commands;
mod levels;
mod shared;
mod support;

use std::thread;
use std::time::Duration;

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::{Value, json};

use support::{Client, Session, slow};

fn code(report: &Value) -> &Value {
    &report["response"]["error"]["code"]
}

#[test]
fn a_long_call_commits_as_one_step_exactly_as_in_process() {
    let s = Session::start("same");
    let (mut a, mut b) = (s.client(), s.client());
    let mut local = gs_testkit::Session::with(EngineConfig::default(), Role::Agent);
    let moved = json!({ "part": "sphere", "steps": 1, "step_ms": 0, "offset": 0.5 });
    a.run("primitive.sphere", json!({ "subdivisions": 3 }));
    b.run("primitive.cube", json!({}));
    let made = b.hash();
    local.call("primitive.sphere", json!({ "subdivisions": 3 }));
    local.call("primitive.cube", json!({}));

    let report = a.start("test.slow", moved.clone(), 30_000);
    assert!(report["id"].is_u64(), "a long call is a job: {report}");
    let job = &report["response"];
    let here = json!(local.call("test.slow", moved));
    assert_eq!(job["ok"], here["ok"], "same result, effect and step");
    assert_eq!(
        job["concurrent_changes"],
        json!(1),
        "the cube someone else made"
    );
    let next = a.run("mesh.stats", json!({}));
    let seq = job["seq"].as_u64().unwrap_or_default();
    assert_eq!(
        next["seq"],
        json!(seq + 1),
        "the job took one sequence number"
    );
    assert_eq!(
        next["concurrent_changes"],
        json!(0),
        "its own commit is not news"
    );

    let remeshed = a.run(
        "mesh.voxel_remesh",
        json!({ "part": "cube", "resolution": 16 }),
    );
    assert!(
        remeshed.get("id").is_none(),
        "cmd.run answers with the response itself"
    );
    let here = json!(local.call(
        "mesh.voxel_remesh",
        json!({ "part": "cube", "resolution": 16 })
    ));
    assert_eq!(remeshed["ok"], here["ok"], "a real long command too");
    assert_eq!(a.hash(), json!(local.hash()), "the same scene");
    a.run("history.undo", json!({}));
    a.run("history.undo", json!({}));
    assert_eq!(a.hash(), made, "undo takes both jobs back exactly");
}

#[test]
fn a_job_reports_progress_and_outlives_its_client() {
    let s = Session::start("progress");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let before = a.hash();
    // Two seconds of work, so it is still running after the reconnect even on a busy machine.
    let id = a.start("test.slow", slow(80), 0)["id"].clone();
    drop(a);
    let mut b = s.client();
    // Until the first step is done: a wait on the job's state, not on a time (review 0002 T2).
    let begun = |r: &Value| r["progress"].as_f64().is_some_and(|p| p > 0.0);
    let report = (0..3000)
        .map(|_| b.status(&id, 10, false))
        .find(|r| begun(r) || !r["response"].is_null())
        .unwrap_or_default();
    assert!(report["response"].is_null(), "still running: {report}");
    assert_eq!(report["working"], json!(true), "its thread works: {report}");
    assert_eq!(report["command"], json!("test.slow"));
    let progress = report["progress"].as_f64().unwrap_or(-1.0);
    assert!(progress > 0.0 && progress < 1.0, "part way: {report}");
    let message = report["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("step ") && message.ends_with(" of 80"),
        "{report}"
    );
    let done = b.status(&id, 30_000, false);
    assert_eq!(done["response"]["ok"]["history_id"], json!(2), "{done}");
    assert_ne!(b.hash(), before, "it committed although its client left");
}

#[test]
fn a_job_fails_stale_when_the_document_moved_underneath_it() {
    let s = Session::start("stale");
    let (mut a, mut b) = (s.client(), s.client());
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let id = a.start("test.slow", slow(20), 0)["id"].clone();
    b.run("primitive.cube", json!({}));
    let changed = b.hash();
    let done = a.status(&id, 30_000, false);
    assert_eq!(code(&done), &json!("E_STALE"), "{done}");
    let error = &done["response"]["error"];
    assert_eq!(error["retryable"], json!(true), "{error}");
    let hint = error["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("run it again"), "{error}");
    assert_eq!(a.hash(), changed, "the job applied nothing");
}

/// Review 0003 M3: calls that change nothing (hiding no face, clearing a mask the part does not
/// have) leave the part as it was, so a job running on it still commits.
#[test]
fn a_call_that_changes_nothing_leaves_a_running_job_to_commit() {
    let s = Session::start("unchanged");
    let (mut a, mut b) = (s.client(), s.client());
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let id = a.start("test.slow", slow(20), 0)["id"].clone();
    let nothing = json!({ "part": "sphere", "select": { "by": "groups", "ids": [99] } });
    b.run("visibility.hide", nothing);
    b.run("mask.set", json!({ "part": "sphere", "value": 0 }));
    let done = a.status(&id, 30_000, false);
    assert!(done["response"]["ok"].is_object(), "{done}");
}

/// Review 0003 m2: a stroke in a job sets its caller's last edit point when the job commits,
/// as it does in a call that runs at once, so the next orbit turns about it.
#[test]
fn a_job_hands_its_strokes_last_edit_point_to_its_caller() {
    let s = Session::start("last-edit");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 8 }));
    // test.slow moves the sphere half a unit along x first.
    let draw = json!({ "brush": "Draw", "samples": [{ "position": [0.5, 0, 1] }] });
    let batch = json!({ "commands": [
        { "command": "test.slow", "args": slow(1) },
        { "command": "brush.stroke", "args": draw },
    ] });
    let job = a.result("job.start", json!({ "batch": batch, "wait_ms": 30_000 }));
    assert!(job["id"].is_u64(), "a job: {job}");
    assert!(job["response"]["ok"].is_object(), "that landed: {job}");
    let view = a.run("view.get", json!({}))["ok"]["result"].clone();
    assert_eq!(view["last_edit"], json!([0.5, 0.0, 1.0]), "{view}");
}

#[test]
fn a_cancelled_job_ends_at_once_and_never_commits() {
    let s = Session::start("cancel");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let before = a.hash();
    let id = a.start("test.slow", slow(400), 0)["id"].clone();
    let cancelled = a.status(&id, 0, true);
    assert_eq!(code(&cancelled), &json!("E_CANCELLED"), "{cancelled}");
    let hint = &cancelled["response"]["error"]["hint"];
    assert_eq!(hint, &json!("start it again to redo it"), "how to redo it");
    let stubborn = json!({ "steps": 4, "step_ms": 25, "offset": 0.5, "ignore_cancel": true });
    let id = a.start("test.slow", stubborn, 0)["id"].clone();
    a.status(&id, 0, true);
    // Its thread finishes the work meanwhile, and the work is thrown away. Wait for the thread
    // itself, not for a time (review 0002 T2).
    let finished = (0..3000).any(|_| {
        let working = a.status(&id, 0, false)["working"] == json!(true);
        if working {
            thread::sleep(Duration::from_millis(10));
        }
        !working
    });
    assert!(finished, "the stubborn job's thread finished");
    assert_eq!(a.hash(), before, "neither job changed anything");
    assert_eq!(code(&a.status(&id, 0, false)), &json!("E_CANCELLED"));
}

#[test]
fn at_most_four_jobs_work_at_once() {
    let s = Session::start("limit");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 1 }));
    let ids: Vec<Value> = (0..4)
        .map(|_| a.start("test.slow", slow(400), 0)["id"].clone())
        .collect();
    let refused = a.start("test.slow", slow(1), 0);
    assert!(refused["id"].is_null(), "{refused}");
    assert_eq!(code(&refused), &json!("E_LIMIT"));
    let error = &refused["response"]["error"];
    assert_eq!(error["retryable"], json!(true), "a place frees up: {error}");
    let hint = error["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("job.status"), "{error}");
    a.status(&ids[0], 0, true);
    // The cancelled job's thread stops within one step and frees its place (on a busy machine
    // that can take a while, so this waits up to a minute).
    let freed = (0..6000).any(|_| {
        let started = a.start("test.slow", slow(1), 0)["id"].is_u64();
        thread::sleep(Duration::from_millis(10 * u64::from(!started)));
        started
    });
    assert!(freed, "a place frees once a cancelled job stops");
    for id in &ids[1..] {
        a.status(id, 0, true);
    }
}

#[test]
fn a_held_commit_waits_for_the_human_to_let_agents_go_on() {
    let s = Session::start("held");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let before = a.hash();
    let id = a.start("test.slow", slow(8), 0)["id"].clone();
    s.as_human("session.control", json!({ "paused": true }));
    let held = a.status(&id, 1_000, false);
    assert!(
        held["response"].is_null(),
        "held while agents are paused: {held}"
    );
    assert_eq!(a.hash(), before);
    s.as_human("session.control", json!({ "paused": false }));
    let done = a.status(&id, 30_000, false);
    assert!(done["response"]["ok"].is_object(), "{done}");
    assert_ne!(a.hash(), before, "committed once agents may go on");
}

#[test]
fn stopping_the_session_ends_running_jobs() {
    let mut s = Session::start("stop");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 1 }));
    let socket = s.socket.clone();
    let waiting = thread::spawn(move || Client::connect(&socket).run("test.slow", slow(400)));
    let started = (0..500).any(|_| {
        let found = a
            .call("job.status", json!({ "id": 1 }))
            .get("error")
            .is_none();
        thread::sleep(Duration::from_millis(10 * u64::from(!found)));
        found
    });
    assert!(started, "the job started");
    s.stop();
    let response = waiting.join().unwrap_or_default();
    assert_eq!(
        response["error"]["code"],
        json!("E_CANCELLED"),
        "{response}"
    );
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("the session stopped"), "{message}");
}

#[test]
fn only_long_edits_and_queries_become_jobs() {
    let s = Session::start("what");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 1 }));
    let short = a.start("mesh.stats", json!({}), 0);
    assert!(
        short["id"].is_null() && short["response"]["ok"].is_object(),
        "{short}"
    );
    let run = json!({ "command": "test.slow", "args": slow(400), "dry_run": true });
    let dry = a.result("job.start", json!({ "run": run, "wait_ms": 0 }));
    assert!(dry["id"].is_null(), "a dry run runs nothing: {dry}");
    assert_eq!(dry["response"]["ok"]["result"]["dry_run"], json!(true));
    let session = a.start("test.long_session", json!({}), 0);
    assert!(
        session["id"].is_null() && session["response"]["ok"].is_object(),
        "{session}"
    );

    let step = json!({ "command": "test.slow", "args": slow(1) });
    let batch = json!({ "commands": [step, step] });
    let both = a.result("job.start", json!({ "batch": batch, "wait_ms": 30_000 }));
    assert!(both["id"].is_u64(), "{both}");
    assert_eq!(both["command"], json!("cmd.batch"));
    assert_eq!(
        both["response"]["ok"]["history_id"],
        json!(2),
        "one step: {both}"
    );
    let failed = a.start(
        "test.slow",
        json!({ "part": "nope", "steps": 1, "step_ms": 0, "offset": 1 }),
        30_000,
    );
    assert_eq!(code(&failed), &json!("E_NOT_FOUND"), "{failed}");

    let unknown = a.call("job.status", json!({ "id": 99 }));
    let data = &unknown["error"]["data"];
    assert_eq!(data["code"], json!("E_NOT_FOUND"), "{unknown}");
    assert_eq!(data["suggestions"], json!(["1", "2"]), "the jobs there are");
    let hint = data["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("job.start"), "{unknown}");
    let neither = a.call("job.start", json!({ "wait_ms": 5 }));
    assert_eq!(neither["error"]["code"], json!(-32602), "{neither}");
}
