//! The generator reaches every case it documents, often enough to matter: generators that
//! never hit an edge make properties pass by never asking (research 06, section 2.4).

use gs_protocol::{CommandInfo, Kind};
use gs_testkit::claims::plan::{Plan, perturb, plan};
use gs_testkit::symmetry::CubeSymmetry;
use gs_testkit::{Session, SplitMix64};
use serde_json::{Value, json};

use crate::data::tally;

fn pool() -> Vec<CommandInfo> {
    let catalog = Session::new().engine.catalog().clone();
    catalog
        .commands
        .into_iter()
        .filter(|c| c.kind == Kind::Edit)
        .collect()
}

/// Enough plans of three steps that each edit command, undo, redo and checkpoint comes up
/// about 150 times, however many edit commands the catalog holds.
fn plans() -> Vec<Plan> {
    let pool = pool();
    let refs: Vec<&CommandInfo> = pool.iter().collect();
    let n = 50 * (pool.len() as u64 + 3);
    (0..n)
        .map(|k| plan(&refs, &[k, k + 100_000, k + 200_000, k + 300_000]))
        .collect()
}

#[test]
fn plans_mix_fixtures_commands_and_history_calls() {
    let all = plans();
    let fixtures = tally(all.iter().map(|p| p.fixtures.len()));
    assert_eq!(
        fixtures.keys().copied().collect::<Vec<_>>(),
        [1, 2],
        "one or two fixtures"
    );
    assert!(
        fixtures.values().all(|&n| n >= 200),
        "both often: {fixtures:?}"
    );
    let names = tally(all.iter().flat_map(|p| p.fixtures.iter().map(|f| f.name)));
    assert_eq!(names.len(), 5, "every fixture: {names:?}");
    let seeds = tally(all.iter().map(|p| p.seed));
    assert!(seeds.len() >= all.len() * 98 / 100, "session seeds vary");
    let commands = tally(
        all.iter()
            .flat_map(|p| p.steps.iter().map(|s| s.command.clone())),
    );
    let edits = pool().len();
    assert_eq!(
        commands.len(),
        edits + 3,
        "every edit, undo, redo and checkpoint: {commands:?}"
    );
    assert!(
        commands.values().all(|&n| n >= 100),
        "each often: {commands:?}"
    );
    let saves = tally(
        all.iter()
            .flat_map(|p| p.steps.iter())
            .filter_map(|s| s.args["name"].as_str()),
    );
    assert!(
        saves.contains_key("base") && saves.contains_key("spare"),
        "both checkpoints: {saves:?}"
    );
    assert!(
        all.iter().all(|p| p.mirrors.len() == p.steps.len()),
        "one symmetry per step"
    );
    let mirrors = tally(
        all.iter()
            .flat_map(|p| p.mirrors.iter().map(|g| format!("{g:?}"))),
    );
    assert_eq!(
        mirrors.len(),
        CubeSymmetry::all().len(),
        "every symmetry is used"
    );
}

#[test]
fn part_arguments_name_fixtures_missing_parts_or_nothing() {
    let all = plans();
    let (mut fixture, mut missing, mut absent, mut unknown) = (0, 0, 0, 0);
    for p in &all {
        for step in p.steps.iter().filter(|s| s.command == "mesh.translate") {
            match step.args.get("part").and_then(Value::as_str) {
                Some("zz-missing") => missing += 1,
                Some(name) if p.fixtures.iter().any(|f| f.name == name) => fixture += 1,
                Some(other) => panic!("part {other} is neither a fixture nor the missing one"),
                None => absent += 1,
            }
            unknown += u32::from(step.args.get("zz_unknown").is_some());
        }
    }
    // Each case only has to appear. The draws follow the Edit command pool, so a new command
    // reshuffles them: about 14 of 144 steps drop `part`, and a floor of 10 failed one time in 10.
    assert!(
        fixture > missing * 3 && missing >= 5 && absent >= 5,
        "{fixture} {missing} {absent}"
    );
    assert!(unknown >= 3, "unknown arguments: {unknown}");
}

fn perturbed(value: &Value, n: u64) -> Vec<Value> {
    (0..n)
        .map(|seed| perturb(value, &mut SplitMix64::new(seed)))
        .collect()
}

#[test]
fn numbers_are_kept_scaled_or_pushed_to_the_edges() {
    let floats = tally(
        perturbed(&json!(0.75), 2000)
            .iter()
            .map(ToString::to_string),
    );
    for edge in [
        "0.75", "0.0", "-0.0", "0.5", "-0.5", "1.0", "-1.0", "2.0", "1e-45", "-1e-40", "1e+39",
        "-1e+39", "0.001", "3.0",
    ] {
        assert!(floats.contains_key(edge), "{edge} missing from {floats:?}");
    }
    for scaled in ["-1.5", "-0.75", "-0.375", "0.1875", "0.375", "1.125"] {
        assert!(
            floats.contains_key(scaled),
            "0.75 scaled to {scaled}: {floats:?}"
        );
    }
    let kept = floats["0.75"];
    assert!(
        (350..=650).contains(&kept),
        "kept about a quarter of the time: {kept}"
    );
    let ints: Vec<u64> = perturbed(&json!(5), 3000)
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let seen = tally(ints.iter().copied());
    for want in [0, 1, 2, 3, 4, 5, 6, 10_000_000_000] {
        assert!(seen.contains_key(&want), "{want} missing from {seen:?}");
    }
    assert!(
        seen.keys().all(|&v| v <= 7 || v == 10_000_000_000),
        "small, or far too big: {seen:?}"
    );
    let huge = perturbed(&json!(u64::MAX), 50);
    assert!(
        huge.iter().all(Value::is_u64),
        "even the largest integer stays an integer"
    );
}

#[test]
fn strings_and_structure_are_perturbed_too() {
    let percents = tally(
        perturbed(&json!("10%"), 2000)
            .iter()
            .filter_map(|v| v.as_str().map(String::from)),
    );
    for want in [
        "10%",
        "20%",
        "-5%",
        "0%",
        "-0%",
        "1000000000000000000000000000000000000000%",
    ] {
        assert!(
            percents.contains_key(want),
            "{want} missing from {percents:?}"
        );
    }
    let names = tally(
        perturbed(&json!("base"), 800)
            .iter()
            .map(ToString::to_string),
    );
    assert_eq!(
        names.keys().cloned().collect::<Vec<_>>(),
        ["\"base\"", "\"zz\""],
        "kept or replaced"
    );
    assert!(
        names["\"zz\""] >= 50 && names["\"base\""] >= 600,
        "mostly kept: {names:?}"
    );
    let nested = json!({ "a": [1.5, { "b": true, "c": null }], "d": "x%" });
    let shapes = perturbed(&nested, 200);
    assert!(
        shapes
            .iter()
            .all(|v| v["a"][1]["b"] == json!(true) && v["a"][1]["c"].is_null()),
        "others kept"
    );
    assert!(
        shapes.iter().any(|v| v["a"][0] != json!(1.5)),
        "numbers inside arrays change"
    );
    let plain = |v: &Value| v["d"] == json!("x%") || v["d"] == json!("zz");
    assert!(
        shapes.iter().all(plain),
        "a non-number percentage is a plain string"
    );
}

#[test]
fn arguments_are_sometimes_dropped() {
    let all = plans();
    let directions = all
        .iter()
        .flat_map(|p| p.steps.iter())
        .filter(|s| s.command == "mesh.translate");
    let (with, without) = directions.fold((0, 0), |(w, wo), s| {
        if s.args.get("direction").is_some() {
            (w + 1, wo)
        } else {
            (w, wo + 1)
        }
    });
    assert!(
        without >= 5 && with >= without * 5,
        "direction dropped now and then: {with} {without}"
    );
}
