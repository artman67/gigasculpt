//! The claims harness refutes a false claim of every kind, refuses runs that checked nothing,
//! and reports failures as bug reports. (The full run holding every command to its declared
//! claims is gs-engine's `every_edit_command_keeps_its_claims`, next to the commands: one test
//! per share of the edit commands, `Harness::check_share`, and one per share of the commands
//! that name a setup, `Harness::check_setup_share`.)

use gs_protocol::{Catalog, Step};
use gs_testkit::Session;
use gs_testkit::claims::plan::Plan;
use gs_testkit::claims::run::{Harness, Tally, Violation, replay, settings};
use gs_testkit::claims::{Claim, declared};
use gs_testkit::fixtures;
use gs_testkit::symmetry::CubeSymmetry;
use serde_json::{Value, json};

fn catalog() -> Catalog {
    Session::new().engine.catalog().clone()
}

#[test]
fn declarations_the_harness_cannot_check_are_refused() {
    let with = |command: &str, claim: Value| {
        let mut c = catalog();
        c.commands
            .iter_mut()
            .filter(|i| i.name == command)
            .for_each(|i| i.claims = vec![claim.clone()]);
        declared(&c)
    };
    let unknown = with("mesh.jitter", json!("teleports"));
    assert!(
        unknown.is_err_and(|e| e.contains("does not know")),
        "an unknown claim"
    );
    let typo = with("mesh.jitter", json!({ "local": { "prat": "/part" } }));
    assert!(typo.is_err(), "an unknown field");
    let query = with("mesh.stats", json!("deterministic"));
    assert!(
        query.is_err_and(|e| e.contains("not an edit")),
        "a claim on a query"
    );
    assert!(
        Harness::new(&{
            let mut c = catalog();
            c.commands[0].claims = vec![json!(1)];
            c
        })
        .is_err(),
        "the harness refuses them too"
    );
}

#[test]
fn claims_are_written_as_documented() {
    let written = [
        (json!("preserves_topology"), "preserves_topology"),
        (
            json!({ "local": { "part": "/part", "center": "/c", "radius": "/r" } }),
            "local",
        ),
        (
            json!({ "volume_non_decreasing": { "tolerance": 0.0 } }),
            "volume_non_decreasing",
        ),
        (
            json!({ "mirror_equivariant": { "vectors": [], "tolerance": 0.0 } }),
            "mirror_equivariant",
        ),
        (json!("deterministic"), "deterministic"),
        (json!("undo_exact"), "undo_exact"),
        (json!("atomic"), "atomic"),
        (json!({ "mask_holds": { "switch": "/mask" } }), "mask_holds"),
        (json!("hidden_holds"), "hidden_holds"),
        (json!("positions_only"), "positions_only"),
        (
            json!({ "identity_at_zero": { "set": { "/op/move": [0.0, 0.0, 0.0] } } }),
            "identity_at_zero",
        ),
        (
            json!({ "placement_invariant": { "pinned": { "/frame": "/frame" }, "unless": "/symmetry", "tolerance": 1e-5 } }),
            "placement_invariant",
        ),
        (
            json!({ "keeps_symmetry": { "planes": "/symmetry/mirror", "origin": "/frame/origin", "pinned": { "/frame": "/frame" }, "tolerance": 1e-5 } }),
            "keeps_symmetry",
        ),
    ];
    for (json, name) in written {
        let claim: Claim = serde_json::from_value(json.clone()).unwrap_or(Claim::Atomic);
        assert_eq!(claim.name(), name, "{json}");
        assert_eq!(
            serde_json::to_value(&claim).ok(),
            Some(json),
            "{name} round trip"
        );
    }
    let names: Vec<&str> = Claim::UNIVERSAL.iter().map(Claim::name).collect();
    assert_eq!(
        names,
        ["deterministic", "undo_exact", "atomic"],
        "the universal claims"
    );
}

/// Holds a real command to a claim it does not keep, and requires the harness to refute it.
fn refuted(command: &str, claim: Claim) {
    refuted_in(&catalog(), command, claim, &[]);
}

/// [`refuted`] on plans of `catalog` that start with the calls of `first`.
fn refuted_in(catalog: &Catalog, command: &str, claim: Claim, first: &[Step]) {
    let name = claim.name();
    let harness = Harness::new(catalog).unwrap_or_else(|e| panic!("{e}"));
    let verdict = harness
        .only(&[command])
        .claiming(command, vec![claim])
        .starting_with(first)
        .run();
    let wanted = format!("{name} broken by {command}");
    assert!(
        verdict.as_ref().is_err_and(|e| e.contains(&wanted)),
        "{wanted}: {verdict:?}"
    );
}

fn local(part: &str, support: Option<(&str, &str)>) -> Claim {
    Claim::Local {
        part: part.into(),
        center: support.map(|s| s.0.into()),
        radius: support.map(|s| s.1.into()),
    }
}

#[test]
fn a_false_topology_claim_is_refuted() {
    refuted("primitive.cube", Claim::PreservesTopology);
}

#[test]
fn a_false_support_radius_is_refuted() {
    refuted(
        "mesh.translate",
        local("/part", Some(("/direction", "/distance"))),
    );
}

#[test]
fn a_part_pointer_to_the_wrong_argument_is_refuted() {
    refuted("mesh.jitter", local("/seed", None));
}

#[test]
fn a_false_volume_claim_is_refuted() {
    refuted(
        "mesh.jitter",
        Claim::VolumeNonDecreasing { tolerance: 2e-6 },
    );
}

#[test]
fn a_false_mirror_claim_is_refuted() {
    let claim = Claim::MirrorEquivariant {
        vectors: Vec::new(),
        tolerance: 1e-6,
    };
    refuted("mesh.jitter", claim);
}

/// The jitter moves every vertex, masked 1 or under hidden faces (the patch fixture has both).
#[test]
fn a_false_mask_claim_is_refuted() {
    refuted("mesh.jitter", Claim::MaskHolds { switch: None });
}

#[test]
fn a_false_hidden_claim_is_refuted() {
    refuted("mesh.jitter", Claim::HiddenHolds);
}

/// Inverting the mask changes a column that is not the positions.
#[test]
fn a_false_positions_only_claim_is_refuted() {
    refuted("mask.invert", Claim::PositionsOnly);
}

/// A move by one unit is not the identity.
#[test]
fn a_false_identity_is_refuted() {
    let set = [("/distance".to_string(), json!(1.0))].into();
    refuted("mesh.translate", Claim::IdentityAtZero { set });
}

/// The jitter draws its offsets in the part's own space, so on a part turned a quarter it lands
/// elsewhere than on the same points baked.
#[test]
fn a_false_placement_claim_is_refuted() {
    let pinned = [("/seed".to_string(), "/seed".to_string())].into();
    let claim = Claim::PlacementInvariant {
        pinned,
        unless: None,
        tolerance: 1e-5,
    };
    let turn = json!({ "op": { "rotate": { "axis": "y", "degrees": 90 } }, "mask": false });
    refuted_in(
        &catalog(),
        "mesh.jitter",
        claim,
        &[step("transform.apply", turn)],
    );
}

/// A parallel plane repeats the transform unmirrored, so held to its planes as mirror planes,
/// `transform.apply`'s lean on a mirror image is not the mirror image of the lean. Every plan
/// starts with the lean as written: the random plans' changed numbers make few of theirs land.
#[test]
fn a_false_symmetry_claim_is_refuted() {
    let mut c = catalog();
    let mut lean = Vec::new();
    (c.commands.iter_mut())
        .filter(|i| i.name == "transform.apply")
        .for_each(|i| {
            i.examples
                .retain(|e| e.args.pointer("/symmetry/parallel").is_some());
            lean.extend(i.examples.iter().map(|e| step(&i.name, e.args.clone())));
        });
    assert!(!lean.is_empty(), "transform.apply has a parallel example");
    let claim = Claim::KeepsSymmetry {
        planes: "/symmetry/parallel".into(),
        origin: "/frame/origin".into(),
        pinned: [("/frame".to_string(), "/frame".to_string())].into(),
        tolerance: 1e-5,
    };
    refuted_in(&c, "transform.apply", claim, &lean);
}

#[test]
fn a_run_that_never_checked_a_claim_proves_nothing() {
    let harness = Harness::new(&catalog())
        .unwrap_or_else(|e| panic!("{e}"))
        .only(&["mesh.translate"]);
    let tally = harness.run().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        harness.require_checked(&tally),
        Ok(()),
        "everything was checked"
    );
    for claim in ["local", "mirror_equivariant", "undo_exact", "atomic"] {
        let mut short = tally.clone();
        short
            .checks
            .remove(&("mesh.translate".to_string(), claim.to_string()));
        let verdict = harness.require_checked(&short);
        assert_eq!(
            verdict,
            Err(format!("mesh.translate: {claim} was never checked")),
            "{claim}"
        );
    }
    let quiet = Harness::new(&catalog())
        .unwrap_or_else(|e| panic!("{e}"))
        .only(&["primitive.cube"]);
    let verdict = quiet.require_checked(&Tally::default());
    assert_eq!(
        verdict,
        Err("primitive.cube: deterministic was never checked".into()),
        "uncalled"
    );
}

fn step(command: &str, args: Value) -> Step {
    Step {
        command: command.into(),
        args,
    }
}

/// One hand-written plan, checked step by step, so every check is seen to run.
#[test]
fn a_plan_runs_every_check_on_every_step() {
    let cube = fixtures::all()
        .into_iter()
        .filter(|f| f.name == "cube")
        .collect();
    let translate = json!({ "part": "cube", "direction": [1, 0, 0], "distance": 0.5 });
    let steps = vec![
        step("mesh.translate", translate),
        step("history.undo", json!({})),
        step("history.redo", json!({})),
        step("history.checkpoint", json!({ "name": "base" })),
        step(
            "mesh.translate",
            json!({ "part": "nope", "direction": [1, 0, 0], "distance": 1 }),
        ),
    ];
    let mirrors = vec![CubeSymmetry::all()[13]; steps.len()];
    let plan = Plan {
        seed: 5,
        fixtures: cube,
        steps,
        setup: 0,
        mirrors,
    };
    let harness = Harness::new(&catalog()).unwrap_or_else(|e| panic!("{e}"));
    let tally = harness.check(&plan).unwrap_or_else(|v| panic!("{v}"));
    let count = |command: &str, claim: &str| tally.count(command, claim);
    let translate: Vec<u32> = [
        "deterministic",
        "effect",
        "undo_exact",
        "preserves_topology",
        "local",
        "volume_non_decreasing",
        "mirror_equivariant",
        "atomic",
        "history",
    ]
    .iter()
    .map(|claim| count("mesh.translate", claim))
    .collect();
    assert_eq!(
        translate,
        [2, 1, 1, 1, 1, 1, 1, 1, 2],
        "translate: one success, one failure"
    );
    let history =
        ["history.undo", "history.redo", "history.checkpoint"].map(|c| count(c, "history"));
    assert_eq!(
        history,
        [1, 1, 1],
        "history calls are compared, not claimed"
    );
    assert_eq!(
        count("history.undo", "deterministic"),
        0,
        "only edits are tallied as edits"
    );
    assert!(
        tally
            .observed
            .contains_key(&("mesh.translate".into(), "mirror_equivariant".into())),
        "observed"
    );
}

/// A setup call (`Harness::starting_with`) that fails or changes nothing is a harness error
/// naming the call and the part, where a random step that does is dropped; and a run in which
/// the setup never ran proves nothing.
#[test]
fn a_setup_that_fails_or_does_nothing_is_a_harness_error() {
    let harness = |args: &Value| {
        (Harness::new(&catalog()).unwrap_or_else(|e| panic!("{e}")))
            .only(&["part.apply_transform"])
            .starting_with(&[step("transform.apply", args.clone())])
    };
    let plan = |args: &Value| {
        let cube = fixtures::all().into_iter().filter(|f| f.name == "cube");
        let bare = Plan {
            seed: 3,
            fixtures: cube.collect(),
            steps: Vec::new(),
            setup: 0,
            mirrors: Vec::new(),
        };
        bare.starting_with(&[step("transform.apply", args.clone())])
    };
    let turn = |degrees: f64| json!({ "op": { "rotate": { "axis": [1.0, 2.0, 3.0], "degrees": degrees } }, "mask": false });
    let good = turn(35.0);
    let tally = (harness(&good).check(&plan(&good))).unwrap_or_else(|v| panic!("{v}"));
    assert_eq!(
        tally.count("transform.apply", "setup"),
        1,
        "it ran on the cube"
    );
    let mut misspelled = good.clone();
    misspelled["bogus"] = json!(1);
    for (args, why) in [(misspelled, "failed"), (turn(0.0), "changed nothing")] {
        let Some(v) = harness(&args).check(&plan(&args)).err() else {
            panic!("a setup that {why} went unnoticed");
        };
        assert_eq!(
            (v.claim.as_str(), v.command.as_str(), v.step),
            ("setup", "transform.apply", Some(0)),
            "{why}"
        );
        let named = v.message.contains("on part \"cube\"") && v.message.contains(why);
        assert!(named, "{why}: {}", v.message);
    }
    assert_eq!(
        harness(&good).require_checked(&Tally::default()),
        Err(format!("the setup call transform.apply {good} never ran")),
        "a setup dropped from every plan"
    );
}

/// A setup a command names in the catalog is checked as the harness's own are: one the harness
/// cannot run is refused when the catalog is read, and one that fails or changes nothing fails
/// that command's run, naming the call, the part and the command (review 0008's setup field).
#[test]
fn a_broken_catalog_setup_fails_and_names_its_command() {
    let with = |setup: Value| {
        let mut c = catalog();
        let setup: Vec<Step> = serde_json::from_value(setup).unwrap_or_else(|e| panic!("{e}"));
        (c.commands.iter_mut())
            .filter(|i| i.name == "part.apply_transform")
            .for_each(|i| i.setup = setup.clone());
        c
    };
    let refused = [
        (
            json!([{ "command": "transform.aply" }]),
            "not in the catalog",
        ),
        (
            json!([{ "command": "transform.apply", "args": [1] }]),
            "not an object",
        ),
    ];
    for (setup, why) in refused {
        let verdict = Harness::new(&with(setup)).map(|_| ());
        let named = |e: &String| e.contains("part.apply_transform") && e.contains(why);
        assert!(verdict.as_ref().is_err_and(named), "{why}: {verdict:?}");
    }
    let turn = |rotate: Value| json!([{ "command": "transform.apply", "args": { "op": { "rotate": rotate }, "mask": false } }]);
    let broken = [
        (turn(json!({ "axis": [1, 2, 3], "degree": 35 })), "failed"),
        (
            turn(json!({ "axis": [1, 2, 3], "degrees": 0 })),
            "changed nothing",
        ),
    ];
    for (setup, why) in broken {
        let harness = Harness::new(&with(setup)).unwrap_or_else(|e| panic!("{e}"));
        let verdict = harness.check_setup_of("part.apply_transform").map(|_| ());
        let named = |e: &String| {
            e.starts_with("part.apply_transform, on plans that start with its catalog setup")
                && e.contains("the setup call transform.apply on part")
                && e.contains(why)
        };
        assert!(verdict.as_ref().is_err_and(named), "{why}: {verdict:?}");
    }
}

#[test]
fn a_failing_plan_names_the_claim_the_step_and_the_sequence() {
    let jitter = json!({ "part": "torus", "amount": 0.25, "seed": 3 });
    let plan = Plan {
        seed: 1,
        fixtures: fixtures::all()
            .into_iter()
            .filter(|f| f.name == "torus")
            .collect(),
        steps: vec![
            step("history.redo", json!({})),
            step("mesh.jitter", jitter.clone()),
        ],
        setup: 0,
        mirrors: vec![CubeSymmetry::all()[1]; 2],
    };
    let mirrored = Claim::MirrorEquivariant {
        vectors: Vec::new(),
        tolerance: 1e-6,
    };
    let harness = Harness::new(&catalog())
        .unwrap_or_else(|e| panic!("{e}"))
        .claiming("mesh.jitter", vec![mirrored]);
    let Some(v) = harness.check(&plan).err() else {
        panic!("a jitter claimed to commute with a mirror went unnoticed");
    };
    assert_eq!(
        (v.claim.as_str(), v.command.as_str(), v.step),
        ("mirror_equivariant", "mesh.jitter", Some(1)),
        "{v}"
    );
    assert_eq!(v.args, jitter, "the step's arguments");
    let report = v.to_string();
    assert!(
        report.starts_with("mirror_equivariant broken by mesh.jitter"),
        "{report}"
    );
    assert!(
        report.contains("at step 1") && report.contains("  0. history.redo {}"),
        "{report}"
    );
    assert!(report.contains("  1. mesh.jitter"), "{report}");
    let bare = Violation { step: None, ..v };
    assert!(
        !bare.to_string().contains("at step"),
        "no step number when it was not a step"
    );
}

#[test]
fn replay_finds_the_first_call_that_answers_differently() {
    let mut s = Session::headless_seeded(3).recording();
    s.ok("primitive.cube", json!({}));
    s.ok("mesh.jitter", json!({ "amount": 0.1 }));
    s.hash();
    let recorded = s.transcript().to_vec();
    assert_eq!(recorded.len(), 3, "every call recorded");
    assert_eq!(replay(3, &recorded), None, "the same seed answers the same");
    assert_eq!(
        replay(4, &recorded),
        Some(1),
        "an unseeded jitter follows the session seed"
    );
    let mut tampered = recorded.clone();
    tampered[2].response.push(' ');
    assert_eq!(
        replay(3, &tampered),
        Some(2),
        "a changed answer is found where it is"
    );
    assert!(
        Session::new().transcript().is_empty(),
        "sessions record only when asked"
    );
}

/// A failing run must report within seconds: bounded shrinking, with the case count still read
/// from the environment as the gate sets it.
#[test]
fn the_runner_shrinks_a_bounded_number_of_times() {
    let config = settings();
    assert_eq!(config.max_shrink_iters, 16, "bounded shrinking");
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|c| c.parse().ok());
    assert_eq!(
        config.cases,
        cases.unwrap_or(256),
        "cases from the environment"
    );
}
