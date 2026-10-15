//! Random command sequences, generated from the catalog: no per-command test code.
//!
//! A plan loads one or two fixtures, then runs steps. Each step is an edit command, called with
//! one of its own catalog examples whose values are perturbed, or `history.undo`,
//! `history.redo` or `history.checkpoint`. Each step comes from its own seed, so removing a
//! step (when proptest shrinks a failure) leaves the others as they were.
//!
//! Perturbation keeps most calls meaningful and pushes some to the edges (research 06, 2.2 and
//! 2.4): numbers are kept, scaled, set to lattice values (0, -0, +-0.5, +-1), made subnormal,
//! or made too large for `f32` (which must be refused). Values between about 1e4 and 1e38 are
//! left out on purpose: a part moved that far loses its shape to `f32` rounding, and until the
//! engine decides how to treat that, such calls would only report that one known gap.
//! Arguments are sometimes dropped, an unknown one sometimes added, and the `part` argument
//! names a loaded fixture, a part that does not exist, or nothing; so does any other argument
//! of one part (a `PartRef`, such as `mesh.project_detail`'s `source`) that the example gives.
//!
//! A harness may name steps every plan starts with ([`Plan::starting_with`]), called as given
//! on each loaded part before the random steps. Unlike a random step, each must succeed and
//! change something, or the run fails as a harness error (review 0008, T1).
//!
//! A plan can also call one command's examples as given ([`as_given`]), with nothing random in
//! it, for the claims random calls reach only now and then.

use gs_protocol::{CommandInfo, Step};
use serde_json::{Map, Number, Value, json};

use crate::SplitMix64;
use crate::fixtures::{self, Fixture};
use crate::symmetry::CubeSymmetry;

/// One random sequence.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The session seed.
    pub seed: u64,
    /// The parts loaded before the first step.
    pub fixtures: Vec<Fixture>,
    /// The calls.
    pub steps: Vec<Step>,
    /// How many of the first calls set the plan up ([`Plan::starting_with`]): each must
    /// succeed and must not report that it changed nothing.
    pub setup: usize,
    /// For each call, the cube symmetry a `mirror_equivariant` claim is checked under.
    pub mirrors: Vec<CubeSymmetry>,
}

impl Plan {
    /// This plan with each call of `first` made before its steps, once on each part it loads,
    /// with the `part` argument set to that part. Its mirror is the identity.
    #[must_use]
    pub fn starting_with(mut self, first: &[Step]) -> Self {
        let names: Vec<&str> = self.fixtures.iter().map(|f| f.name).collect();
        let steps: Vec<Step> = (first.iter())
            .flat_map(|call| {
                names.iter().map(|&name| {
                    let mut step = call.clone();
                    step.args["part"] = json!(name);
                    step
                })
            })
            .collect();
        let identity = CubeSymmetry::all()[0];
        self.setup += steps.len();
        self.mirrors.splice(0..0, vec![identity; steps.len()]);
        self.steps.splice(0..0, steps);
        self
    }
}

/// Checkpoint names. `base` is the name examples use, so restores can find a checkpoint.
const SAVES: [&str; 2] = ["base", "spare"];

/// A plan from seeds: the first picks the session seed and fixtures, each other one a step.
/// `pool` holds the edit commands steps may call. Panics if `seeds` is empty.
pub fn plan(pool: &[&CommandInfo], seeds: &[u64]) -> Plan {
    let mut rng = SplitMix64::new(seeds[0]);
    let mut all = fixtures::all();
    rng.shuffle(&mut all);
    all.truncate(1 + rng.below(2));
    let names: Vec<&str> = all.iter().map(|f| f.name).collect();
    let symmetries = CubeSymmetry::all();
    let (steps, mirrors) = seeds[1..]
        .iter()
        .map(|&seed| {
            let mut rng = SplitMix64::new(seed);
            let mirror = *rng.pick(&symmetries);
            (step(pool, &names, &mut rng), mirror)
        })
        .unzip();
    Plan {
        seed: rng.next_u64(),
        fixtures: all,
        steps,
        setup: 0,
        mirrors,
    }
}

fn step(pool: &[&CommandInfo], parts: &[&str], rng: &mut SplitMix64) -> Step {
    let call = |command: &str, args: Value| Step {
        command: command.into(),
        args,
    };
    let choice = rng.below(pool.len() + 3);
    let Some(info) = pool.get(choice) else {
        return match choice - pool.len() {
            0 => call("history.undo", json!({})),
            1 => call("history.redo", json!({})),
            _ => call(
                "history.checkpoint",
                json!({ "name": rng.pick(&SAVES), "replace": true }),
            ),
        };
    };
    let args = rng.pick(&info.examples).args.clone();
    let mut args = perturb(&args, rng);
    if let Value::Object(map) = &mut args {
        reshape(map, info, parts, rng);
    }
    call(&info.name, args)
}

/// A plan that calls each of `info`'s examples once as given, on the `k`-th standard fixture
/// with the next one loaded beside it: no value perturbed and no argument dropped, the `part`
/// argument naming the `k`-th and any other one-part argument an example gives the other. Then
/// the first example once more with an argument the command does not have, which every
/// command refuses, so a failed call is checked too. Each call is checked under one of the cube
/// symmetries other than the identity, in turn, and the session seed is fixed: it is the same
/// plan on every run.
pub fn as_given(info: &CommandInfo, k: usize) -> Plan {
    let mut both = fixtures::all();
    let n = both.len();
    both.rotate_left(k % n);
    both.truncate(2);
    let names = [both[0].name, both[1].name];
    let mut calls: Vec<Value> = info.examples.iter().map(|e| e.args.clone()).collect();
    if let Some(Value::Object(first)) = calls.first() {
        let mut refused = first.clone();
        refused.insert("zz_unknown".into(), json!(1));
        calls.push(Value::Object(refused));
    }
    let steps: Vec<Step> = (calls.into_iter())
        .map(|mut args| {
            if let Value::Object(map) = &mut args {
                for key in part_keys(map, info) {
                    let name = if key == "part" { names[0] } else { names[1] };
                    map.insert(key, json!(name));
                }
            }
            Step {
                command: info.name.clone(),
                args,
            }
        })
        .collect();
    let symmetries = CubeSymmetry::all();
    let others = &symmetries[1..];
    Plan {
        seed: 7,
        fixtures: both,
        mirrors: (0..steps.len()).map(|i| others[i % others.len()]).collect(),
        steps,
        setup: 0,
    }
}

/// The arguments of `map` that name a part: `part`, if the command has it, and any other
/// one-part argument `map` gives.
fn part_keys(map: &Map<String, Value>, info: &CommandInfo) -> Vec<String> {
    let properties = info.args_schema["properties"]
        .as_object()
        .into_iter()
        .flatten();
    (properties)
        .filter(|(key, schema)| *key == "part" || (map.contains_key(*key) && one_part(schema)))
        .map(|(key, _)| key.clone())
        .collect()
}

/// Sets the part arguments, drops an argument now and then, and sometimes adds an unknown one.
fn reshape(map: &mut Map<String, Value>, info: &CommandInfo, parts: &[&str], rng: &mut SplitMix64) {
    for key in part_keys(map, info) {
        let part = if rng.below(8) == 0 {
            "zz-missing"
        } else {
            rng.pick(parts)
        };
        map.insert(key, json!(part));
    }
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        if rng.below(10) == 0 {
            map.remove(&key);
        }
    }
    if rng.below(20) == 0 {
        map.insert("zz_unknown".into(), json!(1));
    }
}

/// Whether an argument's schema is one part's name (a `PartRef`, or an optional one), not a list.
fn one_part(schema: &Value) -> bool {
    let named = |s: &Value| s["$ref"].as_str().is_some_and(|r| r.ends_with("/PartRef"));
    named(schema)
        || schema["anyOf"]
            .as_array()
            .is_some_and(|a| a.iter().any(named))
}

/// A copy of `value` with its numbers and some of its strings changed.
pub fn perturb(value: &Value, rng: &mut SplitMix64) -> Value {
    match value {
        Value::Number(n) => number(n, rng),
        Value::String(s) => text(s, rng),
        Value::Array(items) => items.iter().map(|v| perturb(v, rng)).collect(),
        Value::Object(map) => {
            let entries = map.iter().map(|(k, v)| (k.clone(), perturb(v, rng)));
            Value::Object(entries.collect())
        }
        other => other.clone(),
    }
}

fn number(n: &Number, rng: &mut SplitMix64) -> Value {
    if let Some(v) = n.as_u64() {
        let small = rng.below(v.min(1 << 20) as usize + 3) as u64;
        return json!(rng.pick(&[v, small, small, 0, 1, v.saturating_add(1), 10_000_000_000]));
    }
    json!(float(n.as_f64().unwrap_or_default(), rng))
}

fn float(x: f64, rng: &mut SplitMix64) -> f64 {
    const EDGES: [f64; 13] = [
        0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 2.0, 1e-45, -1e-40, 1e39, -1e39, 1e-3, 3.0,
    ];
    const SCALES: [f64; 7] = [-2.0, -1.0, -0.5, 0.25, 0.5, 1.5, 2.0];
    match rng.below(4) {
        0 => x,
        1 => x * rng.pick(&SCALES),
        _ => *rng.pick(&EDGES),
    }
}

/// Percentages get a perturbed number; other strings are kept, or now and then replaced.
fn text(s: &str, rng: &mut SplitMix64) -> Value {
    if let Some(percent) = s.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()) {
        return json!(format!("{}%", float(percent, rng)));
    }
    if rng.below(8) == 0 {
        json!("zz")
    } else {
        json!(s)
    }
}
