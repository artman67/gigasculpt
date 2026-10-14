//! A job runs on a shared copy of the session (decision 0010): it gives what the same call gives
//! in place, step by step and bit for bit, lands as the step the call would make there, keeps
//! what parts remember, and fails as stale only when something it depended on changed.

use gs_engine::EngineConfig;
use gs_protocol::Role;
use proptest::prelude::*;
use serde_json::{Value, json};

use crate::support::Session;

fn config(name: &str) -> EngineConfig {
    EngineConfig {
        session: name.into(),
        ..EngineConfig::default()
    }
}

/// A world-space stroke of `brush` over the sphere near `at`.
fn stroke(brush: &str, at: [f32; 3]) -> Value {
    let samples: Vec<Value> = (0..4)
        .map(|i| json!({ "position": [at[0] + 0.05 * i as f32, at[1], at[2]] }))
        .collect();
    json!({ "brush": brush, "part": "sphere", "samples": samples, "size": 0.3 })
}

/// The scene both sides start from: a sphere edited by a stroke (a copy rebuilt from its mesh
/// would cut and number it differently, review 0002 M6), a cube, and a checkpoint before the
/// cube moves.
fn setup() -> Vec<(&'static str, Value)> {
    let up = json!({ "part": "cube", "direction": [1, 0, 0], "distance": 0.5 });
    vec![
        ("primitive.sphere", json!({ "subdivisions": 24 })),
        ("brush.stroke", stroke("Draw", [0.0, 0.0, 1.0])),
        ("primitive.cube", json!({ "subdivisions": 4 })),
        ("history.checkpoint", json!({ "name": "base" })),
        ("mesh.translate", up),
    ]
}

/// A step a batch may hold besides its long one: edits, a normals-reading stroke, and queries
/// of the session's history, checkpoints, settings and scene.
fn extra(k: usize) -> Value {
    let (command, args) = match k {
        0 => (
            "mesh.translate",
            json!({ "part": "cube", "direction": [0, 1, 0], "distance": 0.25 }),
        ),
        1 => ("brush.stroke", stroke("Inflate", [0.3, 0.0, 0.95])),
        2 => ("history.list", json!({ "limit": 1000 })),
        3 => ("scene.describe", json!({})),
        4 => ("history.restore", json!({ "name": "base" })),
        5 => ("mesh.hash", json!({})),
        _ => ("brush.list", json!({})),
    };
    json!({ "command": command, "args": args })
}

fn long(part: usize) -> Value {
    let part = ["sphere", "cube"][part];
    let args = json!({ "part": part, "steps": 1, "step_ms": 0, "offset": 0.25 });
    json!({ "command": "test.slow", "args": args })
}

/// Runs `steps` as one batch over the socket (a job) and in process, after the same setup, and
/// checks every step's output, the effect, the step made, the history listing and the scene.
fn same_both_ways(name: &str, steps: &[Value]) {
    let s = Session::start(name);
    let mut a = s.client();
    let mut local = gs_testkit::Session::with(config(name), Role::Agent);
    for (command, args) in setup() {
        a.run(command, args.clone());
        local.ok(command, args);
    }
    let batch = json!({ "commands": steps });
    let job = a.result("job.start", json!({ "batch": batch, "wait_ms": 60_000 }));
    assert!(job["id"].is_u64(), "a job: {job}");
    let steps: Vec<(String, Value)> = (steps.iter())
        .map(|s| {
            (
                s["command"].as_str().unwrap_or_default().to_string(),
                s["args"].clone(),
            )
        })
        .collect();
    let refs: Vec<(&str, Value)> = steps.iter().map(|(c, a)| (c.as_str(), a.clone())).collect();
    let here = json!(local.batch(&refs));
    assert_eq!(
        job["response"]["ok"], here["ok"],
        "every step's output, the effect, the step"
    );
    assert_eq!(
        job["response"]["error"], here["error"],
        "or the same failure"
    );
    let all = json!({ "limit": 1000 });
    let listed = a.run("history.list", all.clone())["ok"]["result"].clone();
    assert_eq!(
        listed,
        local.query("history.list", all),
        "the history after it"
    );
    assert_eq!(a.hash(), json!(local.hash()), "the scene after it");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]
    /// Review 0002 M1, M4, M6 and T3: batches with one long step give over the socket exactly
    /// what they give in process.
    #[test]
    fn a_job_gives_exactly_what_the_call_gives_in_place(
        before in proptest::collection::vec(0..7usize, 0..3),
        part in 0..2usize,
        after in proptest::collection::vec(0..7usize, 0..3),
    ) {
        let mut steps: Vec<Value> = before.into_iter().map(extra).collect();
        steps.push(long(part));
        steps.extend(after.into_iter().map(extra));
        same_both_ways("same-batch", &steps);
    }
}

#[test]
fn a_job_that_strokes_after_an_edit_gives_the_same_bits() {
    // The batch review 0002 M6 named: a normals-reading stroke beside a long step, on a part
    // edited before.
    same_both_ways("same-bits", &[long(1), extra(1), extra(2)]);
}

fn code(report: &Value) -> &Value {
    &report["response"]["error"]["code"]
}

#[test]
fn a_job_lands_unless_a_part_it_depends_on_changed() {
    let s = Session::start("per-part");
    let (mut a, mut b) = (s.client(), s.client());
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    a.run("primitive.cube", json!({}));
    let slow = json!({ "part": "sphere", "steps": 20, "step_ms": 25, "offset": 0.5 });
    let moved = |part: &str| json!({ "part": part, "direction": [0, 1, 0], "distance": 1 });

    let id = a.start("test.slow", slow.clone(), 0)["id"].clone();
    b.run("mesh.translate", moved("cube"));
    let done = a.status(&id, 30_000, false);
    assert!(
        done["response"]["ok"].is_object(),
        "another part moved: {done}"
    );
    let mut local = gs_testkit::Session::with(config("per-part"), Role::Agent);
    local.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    local.ok("primitive.cube", json!({}));
    local.ok("mesh.translate", moved("cube"));
    local.ok("test.slow", slow.clone());
    assert_eq!(
        a.hash(),
        json!(local.hash()),
        "as if it ran where it landed"
    );

    let id = a.start("test.slow", slow, 0)["id"].clone();
    b.run("mesh.translate", moved("sphere"));
    let changed = b.hash();
    let done = a.status(&id, 30_000, false);
    assert_eq!(code(&done), &json!("E_STALE"), "its own part moved: {done}");
    let message = done["response"]["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("part 'sphere' changed"), "{message}");
    assert_eq!(a.hash(), changed, "and nothing applied");
}

#[test]
fn a_job_renders_through_the_session_renderer() {
    let renderer = gs_ui::Offscreen::host(None).unwrap_or_else(|e| panic!("{e}"));
    let s = Session::start_with("render", |engine| engine.attach_renderer(renderer));
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 4 }));
    let slow = json!({ "steps": 1, "step_ms": 0, "offset": 0.1 });
    let look = json!({ "size": 64, "antialias": false });
    let batch = json!({ "commands": [
        { "command": "test.slow", "args": slow },
        { "command": "view.snapshot", "args": look },
    ] });
    let job = a.result("job.start", json!({ "batch": batch, "wait_ms": 120_000 }));
    assert!(job["id"].is_u64(), "a job: {job}");
    let shot = &job["response"]["ok"]["result"][1];
    assert_eq!(shot["width"], json!(64), "{job}");
    assert!(
        shot["png_base64"].as_str().is_some_and(|p| p.len() > 100),
        "{shot}"
    );
}

#[test]
fn a_part_keeps_its_cell_size_through_jobs() {
    // A past bug: a remesh over the socket ran on a copy that forgot the kept cell size.
    let s = Session::start("kept");
    let mut a = s.client();
    let mut local = gs_testkit::Session::with(config("kept"), Role::Agent);
    let script = [
        ("primitive.cube", json!({ "subdivisions": 3 })),
        (
            "mesh.voxel_remesh",
            json!({ "part": "cube", "resolution": 16 }),
        ),
        ("mesh.voxel_remesh", json!({ "part": "cube" })),
    ];
    for (command, args) in script {
        let remote = a.run(command, args.clone());
        assert!(remote["ok"].is_object(), "{command}: {remote}");
        local.ok(command, args);
        assert_eq!(a.hash(), json!(local.hash()), "after {command}");
    }
}

#[test]
fn a_delivered_job_result_is_released() {
    // Review 0002 M3: finished jobs kept their full answers forever.
    let s = Session::start("release");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    a.run(
        "primitive.cube",
        json!({ "name": "big", "subdivisions": 12 }),
    );
    let batch = json!({ "commands": [
        { "command": "test.slow", "args": { "part": "sphere", "steps": 1, "step_ms": 0, "offset": 0.1 } },
        { "command": "part.get", "args": { "part": "big" } },
    ] });
    let first = a.result("job.start", json!({ "batch": batch, "wait_ms": 30_000 }));
    let mesh = &first["response"]["ok"]["result"][1];
    assert!(
        mesh["positions"].is_array(),
        "delivered whole once: {first}"
    );
    let later = a.status(&first["id"], 0, false);
    let ok = &later["response"]["ok"];
    assert!(ok["result"].is_null(), "then released: {later}");
    assert_eq!(
        ok["history_id"], first["response"]["ok"]["history_id"],
        "the step stays"
    );
    assert!(ok["effect"].is_object(), "and the effect");
    let warned = ok["warnings"].to_string();
    assert!(warned.contains("released"), "{later}");
    assert_eq!(later["working"], json!(false), "its thread is done");
}

#[test]
fn only_the_client_that_started_a_job_or_the_human_cancels_it() {
    // Review 0002 m4: an observer could cancel an agent's job.
    let s = Session::start("owner");
    let mut a = s.client();
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let slow = json!({ "steps": 400, "step_ms": 25, "offset": 0.5 });
    let id = a.start("test.slow", slow, 0)["id"].clone();
    for (name, role) in [("watcher", "observer"), ("other", "agent")] {
        let mut c = crate::support::Client::connect_as(&s.socket, name, role);
        let refused = c.call("job.status", json!({ "id": id, "cancel": true }));
        let data = &refused["error"]["data"];
        assert_eq!(data["code"], json!("E_PRECONDITION"), "{name}: {refused}");
        let running = c.call("job.status", json!({ "id": id }));
        assert!(
            running["result"]["response"].is_null(),
            "{name} could not stop it"
        );
    }
    let mut again = crate::support::Client::connect_as(&s.socket, "test", "agent");
    let cancelled = again.status(&id, 0, true);
    assert_eq!(
        code(&cancelled),
        &json!("E_CANCELLED"),
        "the same client name may"
    );
}

#[test]
fn a_job_that_changes_nothing_is_still_checked_before_it_is_logged() {
    // Review 0002 s1: its log line replays where it lands, so what it read must still be there.
    let s = Session::start("logged");
    let (mut a, mut b) = (s.client(), s.client());
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    let idle = json!({ "part": "sphere", "steps": 20, "step_ms": 25, "offset": 0 });
    let id = a.start("test.slow", idle, 0)["id"].clone();
    let moved = json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 1 });
    b.run("mesh.translate", moved);
    let done = a.status(&id, 30_000, false);
    assert_eq!(code(&done), &json!("E_STALE"), "{done}");
}

#[test]
fn a_job_that_adds_a_part_lands_only_if_no_other_call_added_one() {
    // Decision 0015: a job hands out part ids from its copy's counter, which are the ids the
    // call gets in place only if no other call added a part meanwhile.
    let s = Session::start("part-ids");
    let (mut a, mut b) = (s.client(), s.client());
    a.run("primitive.sphere", json!({ "subdivisions": 2 }));
    a.run("primitive.cube", json!({ "name": "box" }));
    let slow = json!({ "part": "sphere", "steps": 20, "step_ms": 25, "offset": 0.5 });
    let job = |name: &str| {
        let steps = json!([
            { "command": "test.slow", "args": slow },
            { "command": "primitive.cube", "args": { "name": name } },
        ]);
        json!({ "batch": { "commands": steps }, "wait_ms": 0 })
    };
    let moved = json!({ "part": "box", "direction": [0, 1, 0], "distance": 1 });
    let ids = |a: &mut crate::support::Client| {
        let list = a.run("part.list", json!({}))["ok"]["result"]["parts"].clone();
        let rows = list.as_array().cloned().unwrap_or_default();
        rows.iter()
            .map(|p| {
                (
                    p["id"].as_u64().unwrap_or(0),
                    p["name"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect::<Vec<_>>()
    };

    let id = a.result("job.start", job("first"))["id"].clone();
    b.run("mesh.translate", moved);
    let done = a.status(&id, 30_000, false);
    assert!(
        done["response"]["ok"].is_object(),
        "nothing was added: {done}"
    );
    let want = [(1, "sphere"), (2, "box"), (3, "first")].map(|(i, n)| (i, n.to_string()));
    assert_eq!(ids(&mut a), want, "the id it gets in place");

    // Another call adds a part and takes it back: the list is as the job saw it, but id 4
    // was handed out, so the job's part would get another id in place.
    let id = a.result("job.start", job("second"))["id"].clone();
    b.run("primitive.cube", json!({ "name": "other" }));
    b.run("history.undo", json!({}));
    let done = a.status(&id, 30_000, false);
    assert_eq!(code(&done), &json!("E_STALE"), "{done}");
    let message = done["response"]["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("added a part"), "{message}");
    let names: Vec<String> = ids(&mut a).into_iter().map(|r| r.1).collect();
    assert_eq!(names, ["sphere", "box", "first"], "and nothing applied");
}
