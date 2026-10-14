//! The session policy: who may run what, while the human works. Checked as a table over every
//! catalog command, both ways in (`cmd.run` and `cmd.batch`), every role and every switch, so
//! no path can skip a rule another path applies.

use gs_engine::{EngineConfig, Request};
use gs_protocol::{BatchParams, CmdResponse, CommandInfo, Kind, Outcome, Role, RunParams, Step};
use serde_json::{Value, json};

use gs_testkit::{Session, SplitMix64, code, failure, success};

use crate::support::{EngineSession, agent};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Switch {
    Off,
    Paused,
    Gesture,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Path {
    Run,
    Batch,
}

/// What the policy says a call does: refused with this code, or admitted.
fn expected(info: &CommandInfo, role: Role, switch: Switch, path: Path) -> Option<&'static str> {
    let batchable = matches!(info.kind, Kind::Edit | Kind::Query);
    if path == Path::Batch && !batchable {
        return Some("E_ARGS_INVALID");
    }
    if info.human_only && role != Role::Human {
        return Some("E_PRECONDITION");
    }
    if role == Role::Human || info.kind == Kind::Query {
        return None;
    }
    if role == Role::Observer {
        return Some("E_PRECONDITION");
    }
    match switch {
        Switch::Off => None,
        Switch::Paused => Some("E_PAUSED"),
        Switch::Gesture => Some("E_BUSY"),
    }
}

/// Runs the first example of `info` (or no arguments) by `role` along `path`, with the human
/// having set `switch`, and returns the outcome: `ok`, or the error code and message.
fn attempt(info: &CommandInfo, role: Role, switch: Switch, path: Path) -> (String, String) {
    let mut s = Session::with(EngineConfig::default(), role);
    let example = info.examples.first();
    // The client builds the starting scene itself, so its own steps are on top; an observer
    // cannot, so the human does.
    for step in example.map(|e| e.setup.clone()).unwrap_or_default() {
        let response = if role == Role::Observer {
            s.as_human(&step.command, step.args)
        } else {
            s.call(&step.command, step.args)
        };
        success(response, "setup");
    }
    let switches = match switch {
        Switch::Off => json!({}),
        Switch::Paused => json!({ "paused": true }),
        Switch::Gesture => json!({ "gesture": true }),
    };
    success(s.as_human("session.control", switches), "switch");
    let args = example.map_or(json!({}), |e| e.args.clone());
    let response = match path {
        Path::Run => s.call(&info.name, args),
        Path::Batch => s.batch(&[(info.name.as_str(), args)]),
    };
    outcome(response)
}

fn outcome(response: CmdResponse) -> (String, String) {
    match response.outcome {
        Outcome::Ok(_) => ("ok".into(), String::new()),
        Outcome::Error(err) => (code(err.code), err.message),
    }
}

/// The policy table along one way in: role x switch x every command. A refused call gets the
/// policy's code; an admitted call gets exactly what the same client gets with no switch set.
fn the_policy_holds_along(path: Path) {
    let catalog = agent().engine.catalog().clone();
    let mut refusals = 0;
    for info in &catalog.commands {
        for role in [Role::Human, Role::Agent, Role::Observer] {
            let unswitched = attempt(info, role, Switch::Off, path);
            for switch in [Switch::Off, Switch::Paused, Switch::Gesture] {
                let what = format!("{} by {role:?}, {switch:?}, {path:?}", info.name);
                let got = attempt(info, role, switch, path);
                match expected(info, role, switch, path) {
                    Some(wanted) => {
                        assert_eq!(got.0, wanted, "{what}: {}", got.1);
                        refusals += 1;
                    }
                    None => assert_eq!(got, unswitched, "{what} is admitted"),
                }
            }
        }
    }
    assert!(refusals > 100, "non-trivial: {refusals} refusals checked");
}

/// The policy table, one test for each way in (`cmd.run` and `cmd.batch`), so nextest runs
/// the two side by side (decision 0008, amended 2026-10-14).
mod the_policy_holds_for_every_command_role_switch_and_path {
    use super::{Path, the_policy_holds_along};

    #[test]
    fn run() {
        the_policy_holds_along(Path::Run);
    }

    #[test]
    fn batch() {
        the_policy_holds_along(Path::Batch);
    }
}

/// Refusals tell an agent what it can do, never how to get round the human: no hint mentions
/// becoming the human or changing role.
#[test]
fn refusals_never_teach_a_way_around_the_human() {
    let catalog = agent().engine.catalog().clone();
    let mut texts = Vec::new();
    for info in &catalog.commands {
        for role in [Role::Agent, Role::Observer] {
            for switch in [Switch::Paused, Switch::Gesture] {
                let mut s = Session::with(EngineConfig::default(), role);
                let flag = if switch == Switch::Paused {
                    "paused"
                } else {
                    "gesture"
                };
                success(
                    s.as_human("session.control", json!({ flag: true })),
                    "switch",
                );
                if let Outcome::Error(err) = s.call(&info.name, json!({})).outcome {
                    texts.push(format!("{} {}", err.message, err.hint.unwrap_or_default()));
                }
            }
        }
    }
    let mut s = agent();
    success(s.as_human("primitive.cube", json!({})), "the human's cube");
    let err = failure(s.call("history.undo", json!({})), "agent undo");
    texts.push(format!("{} {}", err.message, err.hint.unwrap_or_default()));
    assert!(texts.len() > 40, "non-trivial: {} refusals", texts.len());
    for text in texts {
        let lower = text.to_lowercase();
        for bypass in [
            "role human",
            "--role",
            "as human",
            "reconnect",
            "human role",
            "expect=",
        ] {
            assert!(
                !lower.contains(bypass),
                "a refusal teaches '{bypass}': {text}"
            );
        }
    }
}

/// M6: an agent's plain undo only undoes the agent's own step. It never removes a human's step,
/// and names the step it refused to touch.
#[test]
fn a_plain_undo_from_an_agent_only_undoes_its_own_step() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let moved = success(
        s.as_human(
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 1 }),
        ),
        "the human moves it",
    );
    let err = s.err("history.undo", json!({}));
    assert_eq!(code(err.code), "E_STALE", "the newest step is the human's");
    let id = moved.history_id.unwrap_or_default();
    assert!(
        err.message
            .contains(&format!("is {id} (mesh.translate, by window#0)"))
            && err.message.contains("human"),
        "names the step: {}",
        err.message
    );
    assert!(
        err.hint
            .as_deref()
            .is_some_and(|h| h.contains("history.list")),
        "points at the history: {:?}",
        err.hint
    );
    assert_eq!(
        code(s.err("history.redo", json!({})).code),
        "E_PRECONDITION",
        "nothing to redo"
    );
    success(
        s.as_human("history.undo", json!({})),
        "the human undoes their own move",
    );
    let err = s.err("history.redo", json!({}));
    assert_eq!(code(err.code), "E_STALE", "nor redo the human's step");
    success(s.as_human("history.redo", json!({})), "the human redoes it");
    success(s.as_human("history.undo", json!({})), "and undoes it again");
    let own = s.ok("history.undo", json!({}));
    assert_eq!(
        own.result["command"],
        json!("primitive.sphere"),
        "its own step goes"
    );
}

/// An agent's plain undo does not take another agent's step either; naming the step does.
#[test]
fn a_plain_undo_from_an_agent_leaves_another_agents_step() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let move_by_other = s.engine.execute(
        &gs_engine::Origin {
            client_id: 7,
            client: "other-agent".into(),
            role: Role::Agent,
        },
        Request::Run(RunParams::new(
            "mesh.translate",
            json!({ "direction": [0, 1, 0], "distance": 1 }),
        )),
    );
    let step = success(move_by_other, "another agent moves the cube").history_id;
    let err = s.err("history.undo", json!({}));
    assert!(
        code(err.code) == "E_STALE" && err.message.contains("other-agent#7"),
        "another client's step is not undone either: {}",
        err.message
    );
    assert!(
        err.message.contains("another client's step")
            && err
                .hint
                .as_deref()
                .is_some_and(|h| h.contains(&format!("expect={}", step.unwrap_or_default()))),
        "says whose it is, and how to mean it: {} / {:?}",
        err.message,
        err.hint
    );
    let undone = s.ok("history.undo", json!({ "expect": step }));
    assert_eq!(
        undone.result["step"],
        json!(step),
        "unless named on purpose"
    );
}

/// The human's plain undo undoes the newest step, whoever made it.
#[test]
fn the_human_undoes_any_step() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let undone = success(s.as_human("history.undo", json!({})), "human undo");
    assert_eq!(
        undone.result["command"],
        json!("primitive.cube"),
        "the agent's cube"
    );
}

/// K3: every step of a batch draws its own random numbers: step `i` gets value `seq + i` of the
/// session stream, as single calls do.
#[test]
fn each_step_of_a_batch_gets_its_own_seed() {
    let mut s = Session::with(
        EngineConfig {
            seed: 42,
            ..EngineConfig::default()
        },
        Role::Agent,
    );
    s.ok("primitive.cube", json!({ "name": "a" }));
    s.ok("primitive.cube", json!({ "name": "b" }));
    let response = s.batch(&[
        ("mesh.jitter", json!({ "part": "a", "amount": 0.1 })),
        ("mesh.jitter", json!({ "part": "b", "amount": 0.1 })),
    ]);
    let seq = response.seq.unwrap_or_default() as usize;
    let done = success(response, "two jitters");
    let stream: Vec<u64> = SplitMix64::new(42).take(seq + 2).collect();
    let seeds: Vec<Value> = done
        .result
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| r["seed"].clone())
        .collect();
    assert_eq!(
        seeds,
        [json!(stream[seq]), json!(stream[seq + 1])],
        "one seed per step"
    );
}

/// m5: an empty batch is an argument error and uses no sequence number.
#[test]
fn an_empty_batch_is_refused() {
    let mut s = agent();
    let response = s.request(Request::Batch(BatchParams {
        commands: Vec::<Step>::new(),
        if_rev: None,
    }));
    assert!(response.seq.is_none(), "no sequence number");
    let err = failure(response, "empty batch");
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        ("E_ARGS_INVALID".into(), Some("/commands")),
        "points at the list"
    );
    assert!(err.hint.is_some(), "and says what a batch needs");
    assert_eq!(
        s.call("mesh.stats", json!({})).seq,
        Some(1),
        "the next call is seq 1"
    );
}

/// m11: refusals that clear up on their own say when to retry.
#[test]
fn busy_and_paused_say_when_to_retry() {
    for (flag, wanted) in [("paused", "E_PAUSED"), ("gesture", "E_BUSY")] {
        let mut s = agent();
        success(
            s.as_human("session.control", json!({ flag: true })),
            "switch",
        );
        let err = s.err("primitive.cube", json!({}));
        assert_eq!(code(err.code), wanted, "{flag}");
        assert!(
            err.retryable && err.retry_after_ms.is_some_and(|ms| ms > 0),
            "{flag}: retry after a while"
        );
    }
}
