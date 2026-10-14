//! Registry properties from the test kit's claims harness: every edit command, on random
//! command sequences over the standard fixtures, keeps the claims it declares plus the ones
//! every edit keeps (determinism, exact undo, atomicity), and every state stays sound. The one
//! full run over the catalog lives here, with the commands it checks; gs-testkit's own tests
//! check the harness on chosen commands.

use gs_protocol::{CommandInfo, Outcome, Step};
use gs_testkit::claims::plan::Plan;
use gs_testkit::claims::run::Harness;
use gs_testkit::claims::{Claim, declared};
use gs_testkit::symmetry::CubeSymmetry;
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

/// A declaration as data to compare: numbers as f64 (`2e-6` and `0.000002` are one value),
/// objects without null fields (an optional argument left out reads back as null).
fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64()),
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(fields) => Value::Object(
            (fields.iter())
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Every edit command keeps its declared claims, and every declaration reaches the harness,
/// whichever commands declare them: the harness reads exactly the catalog's declarations, in
/// catalog order, with every argument intact, and one full run passes and checks each of them
/// (and the claims every edit is held to) at least once, history and effects included. A
/// command that names a setup (`part.apply_transform`, the locked Reset Orientation) is checked
/// in that run on plans of its own that start with it. No declaring command is named here, so
/// a command that starts declaring claims or a setup needs no change to this test.
#[test]
fn every_edit_command_keeps_its_claims() {
    let catalog = Session::new().engine.catalog().clone();
    let written: Vec<(String, Vec<Value>, Vec<Step>)> = (catalog.commands.iter())
        .filter(|c| !c.claims.is_empty() || !c.setup.is_empty())
        .map(|c| {
            let claims = c.claims.iter().map(canonical).collect();
            (c.name.clone(), claims, c.setup.clone())
        })
        .collect();
    assert!(
        written.iter().any(|(_, claims, _)| !claims.is_empty()),
        "some command declares claims, or this checks nothing"
    );
    let found = declared(&catalog).unwrap_or_else(|e| panic!("{e}"));
    let read: Vec<(String, Vec<Value>, Vec<Step>)> = (found.iter())
        .map(|d| {
            let claims = d.claims.iter().map(|c| canonical(&json!(c))).collect();
            (d.command.clone(), claims, d.setup.clone())
        })
        .collect();
    assert_eq!(
        read, written,
        "every declaration is read, in catalog order, unchanged"
    );
    let tally = Harness::check_catalog(&catalog).unwrap_or_else(|report| panic!("{report}"));
    for ((command, claim), seen) in &tally.observed {
        eprintln!(
            "observed {command} {claim}: {:e}, policy tolerance {:e}",
            seen.max,
            seen.tolerance()
        );
    }
    for d in found.iter().filter(|d| !d.claims.is_empty()) {
        let command = &d.command;
        for claim in d.claims.iter().chain(&Claim::UNIVERSAL) {
            assert!(
                tally.count(command, claim.name()) >= 1,
                "{command}: {} reached the harness and was checked",
                claim.name()
            );
        }
    }
    for command in [
        "history.undo",
        "history.redo",
        "history.checkpoint",
        "mesh.translate",
    ] {
        assert!(
            tally.count(command, "history") >= 1,
            "{command}: history compared"
        );
    }
    assert!(
        tally.count("mesh.jitter", "effect") >= 1,
        "effects compared"
    );
}

/// The fixture and example on which a command first makes an undo step, called as random plans
/// call it (an example's arguments, with `part` naming the fixture) in one session holding every
/// standard fixture, or `None` when no call changes anything. A call that fails or changes nothing
/// leaves the scene as it was (the `atomic` claim), so the next one starts from the same scene.
fn first_change(info: &CommandInfo) -> Option<(&'static str, String)> {
    let mut s = Session::new();
    let all = fixtures::all();
    for fixture in &all {
        s.add_part(fixture.name, &fixture.mesh);
    }
    let named = info.args_schema["properties"].get("part").is_some();
    for fixture in &all {
        for example in &info.examples {
            let mut args = example.args.clone();
            if named {
                args["part"] = json!(fixture.name);
            }
            if let Outcome::Ok(done) = s.call(&info.name, args).outcome
                && done.history_id.is_some()
            {
                return Some((fixture.name, example.summary.clone()));
            }
        }
    }
    None
}

/// A command with claims names a setup exactly when none of its examples changes any standard
/// fixture until another call has set a state up (ARCHITECTURE 10.1). Without one, random plans
/// reach its claims only by luck, and whether they do changes with every command added; with one
/// it does not need, its claims leave the whole-catalog run for nothing. No command is named
/// here, so this holds `part.apply_transform`'s setup, the locked Reset Orientation's and any
/// later one's.
#[test]
fn a_command_names_a_setup_exactly_when_the_fixtures_alone_never_change() {
    let catalog = Session::new().engine.catalog().clone();
    let mut wrong = Vec::new();
    for info in catalog.commands.iter().filter(|c| !c.claims.is_empty()) {
        let name = &info.name;
        match (first_change(info), info.setup.is_empty()) {
            (None, true) => wrong.push(format!(
                "{name} changes no standard fixture until another call sets a state up: \
                 name those calls in its SETUP"
            )),
            (Some((fixture, example)), false) => wrong.push(format!(
                "{name} names a setup, but its example \"{example}\" already changes the \
                 {fixture} fixture: name a setup only for a command that needs one"
            )),
            _ => {}
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A part flattened to zero thickness (a zero scale factor, decision 0027 section 4) has a
/// volume that is zero or rounding noise, and moving it keeps its claims. Found by a random
/// seed: the first case is that plan as it was reported; the others flatten across a tilted
/// frame, to a plane or a line, where the points are not exactly on one plane or line.
#[test]
fn a_flattened_part_moves_and_keeps_its_claims() {
    let catalog = Session::new().engine.catalog().clone();
    let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
    let flattens = [
        json!({
            "distance": "straight", "mask": true, "part": "cylinder",
            "frame": { "axes": [2.0, 0.0, 0.0, -2.0], "origin": [0.0, 0.75, -1e-40] },
            "op": { "scale": [0.001, 0.5, -0.0] },
        }),
        json!({
            "part": "cylinder",
            "frame": { "axes": [0.3, 0.5, 0.1, 0.8], "origin": [0.2, 0.1, -0.3] },
            "op": { "scale": [1.0, 1.0, 0.0] },
        }),
        json!({
            "part": "cylinder",
            "frame": { "axes": [0.3, 0.5, 0.1, 0.8], "origin": [0.2, 0.1, -0.3] },
            "op": { "scale": [0.0, 1.0, 0.0] },
        }),
    ];
    let translates = [
        json!({ "direction": [3.0, 1e-45, -0.0], "distance": "0.001%", "part": "cylinder" }),
        json!({ "direction": [0.2, 0.7, -0.4], "distance": 0.3, "part": "cylinder" }),
    ];
    for flatten in &flattens {
        for translate in &translates {
            let steps = vec![
                Step {
                    command: "transform.apply".into(),
                    args: flatten.clone(),
                },
                Step {
                    command: "mesh.translate".into(),
                    args: translate.clone(),
                },
            ];
            let plan = Plan {
                seed: 1,
                fixtures: (fixtures::all().into_iter())
                    .filter(|f| f.name == "cylinder")
                    .collect(),
                mirrors: vec![CubeSymmetry::all()[0]; steps.len()],
                steps,
                setup: 0,
            };
            let tally = harness.check(&plan).unwrap_or_else(|v| panic!("{v}"));
            assert_eq!(
                tally.count("mesh.translate", "volume_non_decreasing"),
                1,
                "the flat part's volume was compared: {flatten} then {translate}"
            );
        }
    }
}
