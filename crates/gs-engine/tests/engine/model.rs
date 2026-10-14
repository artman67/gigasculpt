//! The model test of the undo history (ARCHITECTURE 10.1): random command sequences run against
//! the real engine and against a plain list of full states, compared after every step. Every
//! sequence is also logged and replayed into a fresh session, which must rebuild the same scene
//! after every line.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::EngineConfig;
use gs_protocol::{AttrDomain, AttrValues, Attribute, Outcome, PlainMesh, Role, Success};
use proptest::prelude::*;
use serde_json::json;

use gs_testkit::{Session, code};

use crate::support::EngineSession;

const NAMES: [&str; 3] = ["a", "b", "c"];
const SAVES: [&str; 2] = ["one", "two"];
const MASKS: [f32; 2] = [0.25, 0.75];

#[derive(Clone, Debug)]
enum Op {
    Sphere(usize, u32),
    Cube(usize),
    /// A triangle with a vertex mask column: parts that differ only in attribute values.
    Tagged(usize, usize),
    Translate(usize, i8),
    Jitter(usize),
    Undo,
    /// Undo with `expect` set to the newest step's id, read from `history.list`.
    UndoExpecting,
    Redo,
    Checkpoint(usize),
    Restore(usize),
    Panic(usize),
    /// A query: uses a sequence number, changes nothing.
    Query,
    /// A cube and a move in one batch.
    Batch(usize, usize),
    /// Two unseeded jitters in one batch.
    JitterBatch(usize, usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..3usize, 1..4u32).prop_map(|(n, s)| Op::Sphere(n, s)),
        (0..3usize).prop_map(Op::Cube),
        (0..3usize, 0..2usize).prop_map(|(n, k)| Op::Tagged(n, k)),
        (0..3usize, -2..3i8).prop_map(|(n, d)| Op::Translate(n, d)),
        (0..3usize).prop_map(Op::Jitter),
        Just(Op::Undo),
        Just(Op::UndoExpecting),
        Just(Op::Redo),
        (0..2usize).prop_map(Op::Checkpoint),
        (0..2usize).prop_map(Op::Restore),
        (0..3usize).prop_map(Op::Panic),
        Just(Op::Query),
        (0..3usize, 0..3usize).prop_map(|(n, m)| Op::Batch(n, m)),
        (0..3usize, 0..3usize).prop_map(|(n, m)| Op::JitterBatch(n, m)),
    ]
}

/// The model: every document state as (scene hash, part names), with a cursor.
type State = (String, BTreeSet<&'static str>);

struct Model {
    states: Vec<State>,
    at: usize,
    saves: BTreeMap<usize, State>,
}

impl Model {
    fn now(&self) -> &State {
        &self.states[self.at]
    }

    fn has(&self, part: usize) -> bool {
        self.now().1.contains(NAMES[part])
    }

    fn push(&mut self, state: State) {
        self.states.truncate(self.at + 1);
        self.states.push(state);
        self.at += 1;
    }

    fn with(&self, part: usize) -> BTreeSet<&'static str> {
        let mut names = self.now().1.clone();
        names.insert(NAMES[part]);
        names
    }
}

fn tagged(mask: f32) -> PlainMesh {
    let mut mesh = PlainMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        faces: vec![[0, 1, 2, u32::MAX]],
        attributes: BTreeMap::new(),
    };
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(vec![mask; 3]),
    };
    mesh.attributes.insert("mask".into(), column);
    mesh
}

/// Sends `op` to the engine.
fn send(s: &mut Session, op: &Op) -> gs_protocol::CmdResponse {
    let translate =
        |n: usize, d: f64| json!({ "part": NAMES[n], "direction": [0, 1, 0], "distance": d });
    match op {
        Op::Sphere(n, sub) => s.call(
            "primitive.sphere",
            json!({ "name": NAMES[*n], "subdivisions": sub }),
        ),
        Op::Cube(n) => s.call("primitive.cube", json!({ "name": NAMES[*n] })),
        Op::Tagged(n, k) => s.call(
            "part.create",
            json!({ "name": NAMES[*n], "mesh": tagged(MASKS[*k]) }),
        ),
        Op::Translate(n, d) => s.call("mesh.translate", translate(*n, f64::from(*d))),
        Op::Jitter(n) => s.call("mesh.jitter", json!({ "part": NAMES[*n], "amount": 0.01 })),
        Op::Undo => s.call("history.undo", json!({})),
        Op::UndoExpecting => {
            let steps = s.history()["steps"].clone();
            let newest = steps
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["done"] == json!(true));
            let expect = newest.map(|step| step["id"].clone()).unwrap_or(json!(1));
            s.call("history.undo", json!({ "expect": expect }))
        }
        Op::Redo => s.call("history.redo", json!({})),
        Op::Checkpoint(k) => s.call(
            "history.checkpoint",
            json!({ "name": SAVES[*k], "replace": true }),
        ),
        Op::Restore(k) => s.call("history.restore", json!({ "name": SAVES[*k] })),
        Op::Panic(n) => s.call("debug.panic", json!({ "part": NAMES[*n] })),
        Op::Query => s.call("mesh.stats", json!({})),
        Op::Batch(n, m) => s.batch(&[
            ("primitive.cube", json!({ "name": NAMES[*n] })),
            ("mesh.translate", translate(*m, 1.0)),
        ]),
        Op::JitterBatch(n, m) => s.batch(&[
            ("mesh.jitter", json!({ "part": NAMES[*n], "amount": 0.01 })),
            ("mesh.jitter", json!({ "part": NAMES[*m], "amount": 0.01 })),
        ]),
    }
}

/// Runs `op` on both and checks the engine against the model.
fn step(s: &mut Session, model: &mut Model, op: &Op) {
    let before = model.now().0.clone();
    let response = send(s, op);
    match (response.outcome, expect(model, op)) {
        (Outcome::Error(err), Err(wanted)) => {
            assert_eq!(code(err.code), wanted, "{op:?}: {}", err.message);
        }
        (Outcome::Ok(done), Ok(next)) => {
            let hash = s.hash();
            check_change(&done, &before, &hash, op);
            match next {
                Next::Same => assert!(done.history_id.is_none(), "{op:?} made a step"),
                Next::Undo => model.at -= 1,
                Next::Redo => model.at += 1,
                Next::Save(k) => {
                    model.saves.insert(k, model.now().clone());
                }
                Next::Push(names) => {
                    assert!(done.history_id.is_some(), "{op:?} made no step");
                    model.push((hash.clone(), names));
                }
                Next::Restore(state) => {
                    assert_eq!(hash, state.0, "{op:?} did not restore the saved scene");
                    model.push(state);
                }
            }
        }
        (outcome, expected) => {
            panic!("{op:?}: engine gave {outcome:?}, model expected {expected:?}")
        }
    }
    check_history(s, model, op);
}

/// An edit makes a step exactly when the scene changed, and says why when it did not.
fn check_change(done: &Success, before: &str, after: &str, op: &Op) {
    let Some(effect) = &done.effect else {
        return;
    };
    let changed = before != after;
    assert_eq!(
        done.history_id.is_some(),
        changed,
        "{op:?}: a step exactly when the scene changed"
    );
    assert_eq!(
        effect.parts.is_empty(),
        !changed,
        "{op:?}: parts named exactly when changed"
    );
    assert_eq!(
        effect.noop_reason.is_some(),
        !changed,
        "{op:?}: a reason exactly when unchanged"
    );
    assert!(
        effect.verts_touched > 0 || effect.topology_changed || effect.noop_reason.is_some(),
        "{op:?}: no vertex touched and no reason given: {effect:?}"
    );
}

/// The engine's scene and history listing agree with the model, and the steps' bytes add up to
/// the history's: nothing the history holds belongs to no step (review 0002 M4).
fn check_history(s: &mut Session, model: &Model, op: &Op) {
    assert_eq!(s.hash(), model.now().0, "after {op:?}");
    let all = s.query("history.list", json!({ "limit": 1000 }));
    let steps = all["steps"].as_array().cloned().unwrap_or_default();
    let sum: u64 = steps.iter().filter_map(|s| s["bytes"].as_u64()).sum();
    assert_eq!(all["bytes"], json!(sum), "the steps add up after {op:?}");
    let history = s.history();
    assert_eq!(
        history["undoable"],
        json!(model.at),
        "undoable after {op:?}"
    );
    let redoable = model.states.len() - 1 - model.at;
    assert_eq!(
        history["redoable"],
        json!(redoable),
        "redoable after {op:?}"
    );
    let done: Vec<bool> = history["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| s["done"] == json!(true))
        .collect();
    let expected: Vec<bool> = (0..redoable)
        .map(|_| false)
        .chain((0..model.at).map(|_| true))
        .collect();
    assert_eq!(
        done, expected,
        "newest first: undone steps, then done ones, after {op:?}"
    );
}

#[derive(Debug)]
enum Next {
    Same,
    Undo,
    Redo,
    Save(usize),
    Push(BTreeSet<&'static str>),
    Restore(State),
}

/// What the model says `op` does: the next state, or the error code.
fn expect(model: &Model, op: &Op) -> Result<Next, &'static str> {
    let need = |part: usize| {
        if model.has(part) {
            Ok(())
        } else {
            Err("E_NOT_FOUND")
        }
    };
    match op {
        Op::Sphere(n, _) | Op::Cube(n) | Op::Tagged(n, _) if model.has(*n) => Err("E_PRECONDITION"),
        Op::Sphere(n, _) | Op::Cube(n) | Op::Tagged(n, _) => Ok(Next::Push(model.with(*n))),
        Op::Translate(n, 0) => need(*n).map(|()| Next::Same),
        Op::Translate(n, _) | Op::Jitter(n) => need(*n).map(|()| Next::Push(model.now().1.clone())),
        Op::Undo | Op::UndoExpecting if model.at == 0 => Err("E_PRECONDITION"),
        Op::Undo | Op::UndoExpecting => Ok(Next::Undo),
        Op::Redo if model.at + 1 == model.states.len() => Err("E_PRECONDITION"),
        Op::Redo => Ok(Next::Redo),
        Op::Checkpoint(k) => Ok(Next::Save(*k)),
        Op::Restore(k) => match model.saves.get(k) {
            None => Err("E_NOT_FOUND"),
            Some(saved) if saved.0 == model.now().0 => Ok(Next::Same),
            Some(saved) => Ok(Next::Restore(saved.clone())),
        },
        Op::Panic(n) => need(*n).and(Err("E_INTERNAL")),
        Op::Query => Ok(Next::Same),
        Op::Batch(n, _) if model.has(*n) => Err("E_PRECONDITION"),
        Op::Batch(n, m) if *m == *n || model.has(*m) => Ok(Next::Push(model.with(*n))),
        Op::Batch(..) => Err("E_NOT_FOUND"),
        Op::JitterBatch(n, m) => need(*n)
            .and(need(*m))
            .map(|()| Next::Push(model.now().1.clone())),
    }
}

/// A fresh log file for one run.
fn log_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("model-{}-{n}.jsonl", std::process::id()))
}

/// Runs `ops` against the engine and the model, then replays the session's log into a fresh
/// session with another seed, which must match after every line and at the end.
fn run(ops: &[Op]) -> Model {
    let log = log_path();
    let config = EngineConfig {
        seed: 3,
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    let mut model = Model {
        states: vec![(s.hash(), BTreeSet::new())],
        at: 0,
        saves: BTreeMap::new(),
    };
    for op in ops {
        step(&mut s, &mut model, op);
    }
    let mut replay = Session::with(
        EngineConfig {
            seed: 99,
            ..EngineConfig::default()
        },
        Role::Agent,
    );
    replay.replay_log(&log);
    std::fs::remove_file(&log).ok();
    assert_eq!(
        replay.hash(),
        s.hash(),
        "the replay of {ops:?} ends with the same scene"
    );
    model
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]
    /// Model test: random command sequences against a plain list of full states. Catches
    /// sequencing bugs (a new edit that forgets to clear redo) no invariant can see, and any
    /// call the log cannot replay.
    #[test]
    fn history_matches_a_list_of_full_states(ops in proptest::collection::vec(op(), 1..40)) {
        run(&ops);
    }
}

#[test]
fn the_model_test_covers_every_branch() {
    let ops = [
        Op::Undo,
        Op::UndoExpecting,
        Op::Redo,
        Op::Restore(0),
        Op::Panic(0),
        Op::Translate(0, 1),
        Op::Query,
        Op::Sphere(0, 2),
        Op::Sphere(0, 2),
        Op::Checkpoint(0),
        Op::Translate(0, 0),
        Op::Translate(0, 2),
        Op::Jitter(0),
        Op::Panic(0),
        Op::Query,
        Op::UndoExpecting,
        Op::Undo,
        Op::Redo,
        Op::Cube(1),
        Op::Redo,
        Op::Restore(0),
        Op::Restore(0),
        Op::Batch(1, 1),
        Op::Batch(2, 0),
        Op::Batch(2, 1),
        Op::Undo,
        Op::JitterBatch(0, 1),
        Op::JitterBatch(0, 2),
        // A part that differs from the checkpoint only in its mask values.
        Op::Tagged(2, 0),
        Op::Checkpoint(1),
        Op::Undo,
        Op::Tagged(2, 1),
        Op::Restore(1),
        Op::Query,
        Op::UndoExpecting,
        Op::Redo,
    ];
    let model = run(&ops);
    assert!(
        model.states.len() >= 6,
        "non-trivial: {} states",
        model.states.len()
    );
}
