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
/// catalog order, with every argument intact, and the runs over the shares of the catalog's
/// edit commands pass and check each of them (and the claims every edit is held to) at least
/// once, history and effects included. A command that names a setup (`part.apply_transform`,
/// the locked Reset Orientation) is checked in its share's test on plans of its own that start
/// with it. No declaring command is named here, so a command that starts declaring claims or a
/// setup needs no change to these tests. One test per share, so nextest runs them side by side
/// (decision 0008, amended 2026-10-14).
mod every_edit_command_keeps_its_claims {
    use super::{Claim, Harness, Session, Step, Value, canonical, declared, json};

    /// One test per share of the catalog's edit commands; `SHARES` holds their numbers.
    macro_rules! shares {
        ($($share:ident = $k:literal),+ $(,)?) => {
            const SHARES: &[usize] = &[$($k),+];
            $(
                #[test]
                fn $share() {
                    keeps_its_claims($k);
                }
            )+
        };
    }

    shares!(
        share_0 = 0,
        share_1 = 1,
        share_2 = 2,
        share_3 = 3,
        share_4 = 4,
        share_5 = 5,
        share_6 = 6,
        share_7 = 7,
        share_8 = 8,
        share_9 = 9,
    );

    /// The catalog and the harness over all of it.
    fn whole() -> (gs_protocol::Catalog, Harness) {
        let catalog = Session::new().engine.catalog().clone();
        let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
        (catalog, harness)
    }

    /// The harness reads exactly the catalog's declarations, in catalog order, with every
    /// argument intact.
    #[test]
    fn every_declaration_is_read_unchanged() {
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
    }

    /// The shares are numbered 0 to their count, and together hold every edit command random
    /// plans call once, with every command that declares claims or names a setup and the two
    /// whose history and effects are compared by name.
    #[test]
    fn the_shares_cover_every_edit_command() {
        assert_eq!(
            SHARES,
            (0..SHARES.len()).collect::<Vec<_>>(),
            "numbered in order"
        );
        let (catalog, harness) = whole();
        let mut dealt: Vec<String> = (SHARES.iter())
            .flat_map(|&k| harness.share(k, SHARES.len()))
            .collect();
        dealt.sort();
        let mut every = harness.share(0, 1);
        every.sort();
        assert_eq!(dealt, every, "each edit command in one share");
        let found = declared(&catalog).unwrap_or_else(|e| panic!("{e}"));
        let named = ["mesh.translate", "mesh.jitter"].map(String::from);
        let declaring = found.iter().map(|d| &d.command).chain(&named);
        for command in declaring {
            assert!(every.contains(command), "{command} is in a share");
        }
    }

    /// Share `k`: its commands keep their declared claims, and the run over them checks each,
    /// and the claims every edit is held to, at least once; history is compared on the three
    /// history steps, and on `mesh.translate` and effects on `mesh.jitter` in their shares.
    fn keeps_its_claims(k: usize) {
        let (catalog, harness) = whole();
        let mine = harness.share(k, SHARES.len());
        let tally = (harness.check_share(k, SHARES.len()))
            .unwrap_or_else(|report| panic!("share {k}: {report}"));
        for ((command, claim), seen) in &tally.observed {
            eprintln!(
                "observed {command} {claim}: {:e}, policy tolerance {:e}",
                seen.max,
                seen.tolerance()
            );
        }
        let found = declared(&catalog).unwrap_or_else(|e| panic!("{e}"));
        let declaring = found.iter().filter(|d| !d.claims.is_empty());
        for d in declaring.filter(|d| mine.contains(&d.command)) {
            let command = &d.command;
            for claim in d.claims.iter().chain(&Claim::UNIVERSAL) {
                assert!(
                    tally.count(command, claim.name()) >= 1,
                    "{command}: {} reached the harness and was checked",
                    claim.name()
                );
            }
        }
        let history = ["history.undo", "history.redo", "history.checkpoint"].map(String::from);
        let translate = ["mesh.translate".to_string()];
        for command in history
            .iter()
            .chain(translate.iter().filter(|c| mine.contains(c)))
        {
            assert!(
                tally.count(command, "history") >= 1,
                "{command}: history compared"
            );
        }
        if mine.iter().any(|c| c == "mesh.jitter") {
            assert!(
                tally.count("mesh.jitter", "effect") >= 1,
                "effects compared"
            );
        }
    }
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
