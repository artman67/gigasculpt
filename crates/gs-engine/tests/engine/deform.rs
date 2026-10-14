//! Deformers through `transform.apply` (decision 0041, amended 2026-10-12): the logged line
//! holds the box, radius, axes and gradient a call resolved, so a replay measures nothing again
//! and lands on the same scene on 1 and 24 threads; no deformer, and no call with a gradient or
//! a component off, moves a part's transform instead of its points, and each lands on a placed
//! part where it lands on the part baked; what has no meaning is refused; and each operation
//! keeps `transform.apply`'s claims on its examples as given and on random plans, over parts
//! with a transform, with a mirror plane and without. The cage's line holds only the points it
//! moved (DF3).

use std::path::Path;

use gs_protocol::{ExampleInfo, Step};
use gs_testkit::claims::plan::Plan;
use gs_testkit::claims::run::Harness;
use gs_testkit::symmetry::CubeSymmetry;
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::support::{EngineSession, agent, log_records, worst};
use crate::views::log_path;

fn remove(log: &Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}

/// The scene's own frame moved to `origin`.
fn at(origin: [f32; 3]) -> Value {
    json!({ "origin": origin, "axes": [0.0, 0.0, 0.0, 1.0] })
}

#[test]
fn a_deformer_line_holds_what_it_resolved_and_replays_alike_on_any_thread_count() {
    let log = log_path("deform-resolved");
    let mut s = seeded(5, Some(log.clone()));
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let bend = json!({
        "op": { "bend": { "axis": "y", "toward": "x", "degrees": 70 } },
        "axes": { "x": true, "y": true },
        "gradient": { "axis": "z", "from": 0.0, "to": 1.0 },
    });
    let bent = s.ok("transform.apply", bend).result;
    let spherize = json!({ "op": { "spherize": { "amount": 0.5 } } });
    let rounded = s.ok("transform.apply", spherize).result;
    let target = s.hash();
    let (_, records) = log_records(&log);
    let lines: Vec<&Value> = (records.iter())
        .filter(|r| r.command == "transform.apply")
        .map(|r| &r.args)
        .collect();
    assert_eq!(lines.len(), 2, "one line a call");
    let numbers = |v: &Value| {
        v.as_array()
            .is_some_and(|a| a.len() == 3 && a.iter().all(Value::is_number))
    };
    let pinned = &lines[0]["box"];
    assert!(
        numbers(&pinned["lo"]) && numbers(&pinned["hi"]),
        "the box written in: {pinned}"
    );
    assert_eq!(pinned, &bent["box"], "as the result reports it");
    assert_eq!(
        lines[0]["gradient"],
        json!({ "axis": "z", "from": 0.0, "to": 1.0 })
    );
    let axes = &lines[0]["axes"];
    assert_eq!(
        (&axes["x"], &axes["y"], &axes["z"]),
        (&json!(true), &json!(true), &json!(false))
    );
    let radius = &lines[1]["op"]["spherize"]["radius"];
    assert!(
        radius.as_f64().is_some_and(|r| r > 0.5 && r < 1.5),
        "the radius written in: {radius}"
    );
    assert_eq!(radius, &rounded["radius"], "as the result reports it");
    for threads in [1, 24] {
        let replayed = gs_par::with_threads(threads, || {
            let mut again = seeded(5, None);
            again.replay_log(&log);
            again.hash()
        });
        assert_eq!(replayed, target, "the replay on {threads} threads");
    }
    remove(&log);
}

#[test]
fn a_cage_line_holds_only_its_moved_points_and_the_cage_and_curve_replay_alike() {
    let log = log_path("deform-cage");
    let mut s = seeded(7, Some(log.clone()));
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let still = json!([0.0, 0.0, 0.0]);
    // Each number exact in f32, so the logged line reads back as written.
    let moved = json!({ "at": [2, 2, 1], "by": [0.125, 0.25, 0.0] });
    let cage = json!({ "op": { "lattice": { "size": [4, 3, 3], "offsets": [
        { "at": [1, 1, 1], "by": still },
        moved,
        { "at": [0, 0, 2], "by": still },
    ] } } });
    let caged = s.ok("transform.apply", cage).result;
    let curve = json!({
        "op": { "curve": {
            "axis": "z",
            "points": [[0.0, 0.0, -1.0], [0.3, 0.0, 0.0], [0.0, 0.2, 1.0]],
            "twist": [0, 45, 90],
            "scale": [1, 1.2, 0.8],
        } },
        "gradient": { "axis": "z", "from": 0.2, "to": 0.9 },
    });
    let curved = s.ok("transform.apply", curve).result;
    let target = s.hash();
    let (_, records) = log_records(&log);
    let lines: Vec<&Value> = (records.iter())
        .filter(|r| r.command == "transform.apply")
        .map(|r| &r.args)
        .collect();
    assert_eq!(lines.len(), 2, "one line a call");
    assert_eq!(
        lines[0]["op"]["lattice"]["offsets"],
        json!([moved]),
        "only the moved point"
    );
    for (line, done) in lines.iter().zip([&caged, &curved]) {
        let pinned = &line["box"];
        assert!(pinned["lo"].is_array(), "the box written in: {line}");
        assert_eq!(pinned, &done["box"], "as the result reports it");
    }
    for threads in [1, 24] {
        let replayed = gs_par::with_threads(threads, || {
            let mut again = seeded(7, None);
            again.replay_log(&log);
            again.hash()
        });
        assert_eq!(replayed, target, "the replay on {threads} threads");
    }
    remove(&log);
}

/// A sphere given a transform: a turn about a slanted axis, an even scale and a move.
fn placed() -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let ops = [
        json!({ "rotate": { "axis": [1.0, 2.0, 3.0], "degrees": 35 } }),
        json!({ "scale": [1.5, 1.5, 1.5] }),
        json!({ "move": [0.3, -0.2, 0.7] }),
    ];
    for op in ops {
        s.ok(
            "transform.apply",
            json!({ "frame": at([0.0; 3]), "op": op }),
        );
    }
    s
}

/// The bytes the newest undo step holds.
fn newest_step_bytes(s: &mut Session) -> u64 {
    s.history()["steps"][0]["bytes"]
        .as_u64()
        .unwrap_or_default()
}

#[test]
fn no_deformer_routes_to_the_part_transform_and_each_lands_where_it_lands_baked() {
    let calls = [
        json!({ "op": { "bend": { "axis": "y", "toward": "x", "degrees": 60 } } }),
        json!({ "op": { "fold": { "axis": "x", "toward": "y", "degrees": 50 } } }),
        json!({ "op": { "twist": { "axis": "z", "degrees": 120 } } }),
        json!({ "op": { "taper": { "axis": "y", "amount": 0.5, "exponent": 2 } } }),
        json!({ "op": { "shear": { "axis": "y", "toward": "z", "amount": 0.4, "profile": "smooth" } } }),
        json!({ "op": { "clip": { "axis": "x", "depth": 0.6 } } }),
        json!({ "op": { "spherize": { "amount": 0.7 } } }),
        json!({ "op": { "noise": { "amount": 0.05, "scale": 0.4, "seed": 9 } } }),
        json!({ "op": { "lattice": { "size": [3, 3, 3], "offsets": [{ "at": [1, 2, 1], "by": [0.0, 0.3, 0.1] }] } } }),
        json!({ "op": { "lattice": { "kind": "hard", "size": [2, 3, 2], "offsets": [{ "at": [1, 2, 0], "by": [0.2, 0.0, -0.1] }] } } }),
        json!({ "op": { "curve": { "axis": "y", "points": [[0.0, -1.6, 0.0], [0.4, -0.6, 0.0], [0.0, 0.6, 0.2]], "twist": [0, 30, 60] } } }),
        json!({ "op": { "move": [0.0, 0.4, 0.0] }, "gradient": { "axis": "x", "from": 0.2, "to": 0.8 } }),
        json!({ "op": { "move": [0.3, 0.4, 0.0] }, "axes": { "y": true } }),
        json!({ "op": { "rotate": { "axis": "z", "degrees": 40 } }, "axes": { "x": true, "z": true } }),
    ];
    for call in calls {
        let mut args = call.clone();
        args["frame"] = at([0.4, 0.5, 0.6]);
        let mut s = placed();
        let start = s.mesh("sphere").positions;
        let done = s.ok("transform.apply", args.clone()).result;
        let got = s.mesh("sphere").positions;
        let bits = |p: &[f32; 3]| p.map(f32::to_bits);
        let moved = (start.iter().zip(&got))
            .filter(|(a, b)| bits(a) != bits(b))
            .count() as u64;
        let bytes = newest_step_bytes(&mut s);
        assert!(moved > 100, "{call}: moved {moved} points");
        assert!(
            bytes >= 12 * moved,
            "{call}: kept in the points, {bytes} bytes for {moved}"
        );
        let mut baked = placed();
        baked.ok("part.apply_transform", json!({}));
        for key in ["box", "radius"] {
            if let Some(v) = done.get(key).filter(|v| !v.is_null()) {
                match key {
                    "box" => args["box"] = v.clone(),
                    _ => args["op"]["spherize"]["radius"] = v.clone(),
                }
            }
        }
        baked.ok("transform.apply", args);
        let want = baked.mesh("sphere").positions;
        assert!(
            worst(&got, &want) < 1e-5,
            "{call}: off by {}",
            worst(&got, &want)
        );
    }
}

#[test]
fn a_deformer_refuses_what_has_no_meaning() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let up = json!({ "move": [0.0, 0.0, 1.0] });
    let refused = [
        (
            json!({ "op": { "bend": { "axis": "y", "toward": "y", "degrees": 30 } } }),
            "/op/bend/toward",
        ),
        (
            json!({ "op": { "taper": { "axis": "y", "amount": 1.5 } } }),
            "/op/taper/amount",
        ),
        (
            json!({ "op": { "taper": { "axis": "y", "amount": 0.5, "exponent": 0 } } }),
            "/op/taper/exponent",
        ),
        (
            json!({ "op": { "noise": { "amount": 0.1, "scale": 0 } } }),
            "/op/noise/scale",
        ),
        (
            json!({ "op": { "spherize": { "amount": 2 } } }),
            "/op/spherize/amount",
        ),
        (
            json!({ "op": { "spherize": { "amount": 0.5, "radius": -1 } } }),
            "/op/spherize/radius",
        ),
        (
            json!({ "op": { "clip": { "axis": "z", "depth": 1e39 } } }),
            "/op/clip/depth",
        ),
        (json!({ "op": up, "axes": {} }), "/axes"),
        (
            json!({ "op": up, "gradient": { "axis": "x", "from": 0.5, "to": 0.5 } }),
            "/gradient",
        ),
        (
            json!({ "op": up, "gradient": { "axis": "x", "from": 0.0, "to": 1.5 } }),
            "/gradient/to",
        ),
        (
            json!({ "op": up, "box": { "lo": [0.0, 1.0, 0.0], "hi": [1.0, 0.0, 1.0] } }),
            "/box",
        ),
    ];
    for (args, path) in refused {
        let e = s.err("transform.apply", args.clone());
        assert_eq!(e.path.as_deref(), Some(path), "{args}: {}", e.message);
    }
}

#[test]
fn the_cage_and_the_curve_refuse_what_has_no_meaning() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let refused = [
        (
            json!({ "op": { "lattice": { "size": [1, 3, 3] } } }),
            "/op/lattice/size",
        ),
        (
            json!({ "op": { "lattice": { "size": [3, 3, 3], "offsets": [{ "at": [0, 3, 0], "by": [0.0, 0.1, 0.0] }] } } }),
            "/op/lattice/offsets/0/at",
        ),
        (
            json!({ "op": { "lattice": { "size": [3, 3, 3], "offsets": [
                { "at": [1, 1, 1], "by": [0.0, 0.1, 0.0] },
                { "at": [1, 1, 1], "by": [0.1, 0.0, 0.0] },
            ] } } }),
            "/op/lattice/offsets/1/at",
        ),
        (
            json!({ "op": { "lattice": { "size": [3, 3, 3], "offsets": [{ "at": [1, 1, 1], "by": [1e39, 0.0, 0.0] }] } } }),
            "/op/lattice/offsets/0/by",
        ),
        (
            json!({ "op": { "curve": { "axis": "x", "points": [[0.0, 0.0, 0.0]] } } }),
            "/op/curve/points",
        ),
        (
            json!({ "op": { "curve": { "axis": "x", "points": [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] } } }),
            "/op/curve/points/1",
        ),
        (
            json!({ "op": { "curve": { "axis": "x", "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]] } } }),
            "/op/curve/points/1",
        ),
        (
            json!({ "op": { "curve": { "axis": "x", "twist": [0, 10, 20] } } }),
            "/op/curve/twist",
        ),
        (
            json!({ "op": { "curve": { "axis": "x", "scale": [1, 11] } } }),
            "/op/curve/scale/1",
        ),
    ];
    for (args, path) in refused {
        let e = s.err("transform.apply", args.clone());
        assert_eq!(e.path.as_deref(), Some(path), "{args}: {}", e.message);
    }
}

/// One plan of every fixture placed by `placing`, then each example of `examples` that runs,
/// as given, on each fixture.
fn as_given(examples: &[ExampleInfo], placing: &[Step]) -> Plan {
    let fixtures = fixtures::all();
    let steps: Vec<Step> = (fixtures.iter())
        .flat_map(|f| {
            (examples.iter().filter(|e| e.fails_with.is_none())).map(move |e| {
                let mut args = e.args.clone();
                args["part"] = json!(f.name);
                Step {
                    command: "transform.apply".into(),
                    args,
                }
            })
        })
        .collect();
    let plan = Plan {
        seed: 7,
        mirrors: vec![CubeSymmetry::all()[0]; steps.len()],
        fixtures,
        steps,
        setup: 0,
    };
    plan.starting_with(placing)
}

/// The operation `op` keeps `transform.apply`'s claims on its own examples, as given and on
/// random plans, without symmetry and mirrored across x about a frame off the plane, over parts
/// placed by a half turn, an even scale and a move that keep them symmetric across x and z: so
/// `placement_invariant` (which a plane skips) meets parts with a transform, and
/// `keeps_symmetry` a mirror plane that splits them. The examples also run as given because
/// the random plans perturb every number, and the cage's and the curve's dozen or so numbers
/// leave few of their calls valid (a size of 0 or 1, an index past it, a scale below 0).
fn keeps_the_claims_of_transform_apply(op: &str) {
    let placing: Vec<Step> = [
        json!({ "rotate": { "axis": "y", "degrees": 180 } }),
        json!({ "scale": [2.0, 2.0, 2.0] }),
        json!({ "move": [0.0, 0.3, 0.0] }),
    ]
    .into_iter()
    .map(|op| Step {
        command: "transform.apply".into(),
        args: json!({ "frame": at([0.0; 3]), "op": op }),
    })
    .collect();
    let mut own = Session::new().engine.catalog().clone();
    let Some(info) = own
        .commands
        .iter_mut()
        .find(|c| c.name == "transform.apply")
    else {
        panic!("no transform.apply");
    };
    info.examples.retain(|e| e.args["op"].get(op).is_some());
    (info.examples.iter_mut())
        .filter_map(|e| e.args.as_object_mut())
        .for_each(|args| {
            args.remove("symmetry");
        });
    let mirrored: Vec<_> = (info.examples.iter())
        .filter(|e| e.fails_with.is_none())
        .map(|e| {
            let mut e = e.clone();
            e.args["frame"] = at([0.4, 0.1, -0.2]);
            e.args["symmetry"] = json!({ "mirror": { "x": true } });
            e
        })
        .collect();
    assert!(!mirrored.is_empty(), "{op} has an example that runs");
    info.examples.extend(mirrored);
    let given = as_given(&info.examples, &placing);
    let harness = (Harness::new(&own).unwrap_or_else(|e| panic!("{e}")))
        .only(&["transform.apply"])
        .starting_with(&placing);
    let mut tally =
        (harness.check(&given)).unwrap_or_else(|v| panic!("{op}, its examples as given: {v}"));
    let random = harness
        .run()
        .unwrap_or_else(|report| panic!("{op}: {report}"));
    for (key, n) in random.checks {
        *tally.checks.entry(key).or_default() += n;
    }
    (harness.require_checked(&tally)).unwrap_or_else(|e| panic!("{op}: {e}"));
    for ((_, claim), seen) in &tally.observed {
        eprintln!("observed {op} {claim}: {:e}", seen.max);
    }
}

/// Every deformer, one test for each operation as `op` names it, so nextest runs them side by
/// side (decision 0008, amended 2026-10-14).
mod every_deformer_keeps_the_claims_of_transform_apply_on_placed_and_mirrored_parts {
    macro_rules! each {
        ($($op:ident),+ $(,)?) => {
            $(
                #[test]
                fn $op() {
                    super::keeps_the_claims_of_transform_apply(stringify!($op));
                }
            )+
        };
    }

    each!(
        bend, fold, twist, taper, shear, clip, spherize, noise, lattice, curve
    );
}
