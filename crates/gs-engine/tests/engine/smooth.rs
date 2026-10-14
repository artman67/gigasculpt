//! `mesh.smooth` (decision 0041, section 3, amended 2026-10-12 for DF5): each mode moves a
//! small grid as worked out by hand; an amount is whole rounds, the same as its rounds run one
//! by one and as a live drag through the open deformer's session; a negative amount sharpens
//! once; Polish with `keep` holds the volume; boundary points move along the boundary only; a
//! fully masked point keeps its bits; a mirror plane gives a mirrored result; a call is one
//! undo step and one line that replays it; and 1 and 24 threads give the same bits.

use std::collections::BTreeMap;
use std::path::Path;

use gs_engine::EngineConfig;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh, Role};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::support::{EngineSession, agent, bits, log_records};
use crate::views::log_path;

/// The five modes, as `mode` and `keep`.
const MODES: [(&str, bool); 5] = [
    ("smooth", true),
    ("strong", true),
    ("polish", true),
    ("polish", false),
    ("relax", true),
];

/// The amount of one round: amounts up to it run one round.
const ROUND: f32 = 50.0;

/// How far a point at full weight moves toward its target in a round, `1 - e^-0.5`.
fn step() -> f64 {
    1.0 - libm::exp(-0.5)
}

/// `mode` at `amount` on `part`.
fn call(part: &str, (mode, keep): (&str, bool), amount: f32) -> Value {
    json!({ "part": part, "mode": mode, "keep": keep, "amount": amount })
}

/// `mode` at `amount` on the sphere, fading across y, about a frame off its points.
fn faded(mode: (&str, bool), amount: f32) -> Value {
    let mut args = call("sphere", mode, amount);
    args["frame"] = json!({ "origin": [0.05, 0.03, 0.02], "axes": [0.0, 0.0, 0.0, 1.0] });
    args["gradient"] = json!({ "axis": "y", "from": 1.0, "to": 0.0 });
    args
}

fn smooth(s: &mut Session, args: Value) -> Value {
    s.ok("mesh.smooth", args).result
}

fn points(s: &mut Session, part: &str) -> Vec<[f32; 3]> {
    s.mesh(part).positions
}

fn undoable(s: &mut Session) -> u64 {
    s.history()["undoable"].as_u64().unwrap_or_default()
}

fn remove(log: &Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}

/// A lumpy sphere, "sphere": 2562 points jittered by 2%.
fn lumpy(s: &mut Session) {
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let jitter = json!({ "part": "sphere", "amount": "2%", "seed": 3 });
    s.ok("mesh.jitter", jitter);
}

/// The lumpy sphere's points after one call of `args`, in a fresh session.
fn one_call(args: Value) -> Vec<[u32; 3]> {
    let mut s = agent();
    lumpy(&mut s);
    smooth(&mut s, args);
    bits(&points(&mut s, "sphere"))
}

/// A flat grid in z = 0 of quads facing +z, its points at `at` along x and along y, row by row.
fn grid(at: &[f32]) -> PlainMesh {
    let n = at.len();
    let positions = (0..n * n).map(|i| [at[i % n], at[i / n], 0.0]).collect();
    let k = n as u32;
    let faces = (0..k - 1)
        .flat_map(|r| (0..k - 1).map(move |c| r * k + c))
        .map(|a| [a, a + 1, a + k + 1, a + k])
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

/// `mesh` with a vertex mask.
fn masked(mut mesh: PlainMesh, mask: Vec<f32>) -> PlainMesh {
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), column);
    mesh
}

#[test]
fn each_mode_moves_a_small_grid_as_worked_out_by_hand() {
    // The middle point of a 5 by 5 grid, raised 1: its four neighbours' mean is 0 below it,
    // the mean of their means a quarter above 0, and after a shrinking step its neighbours
    // stand at a quarter of the step. Moved 0.3 along x in the flat grid instead, its edges are
    // 0.7, 1.3 and twice sqrt(1.09) long.
    let l = step();
    let mu = l / (0.1 * l - 1.0);
    let edges = [[0.7, 0.0], [-1.3, 0.0], [-0.3, 1.0], [-0.3, -1.0]];
    let length = |e: [f64; 2]| libm::hypot(e[0], e[1]);
    let mean = edges.iter().map(|&e| length(e)).sum::<f64>() / 4.0;
    let relax = edges
        .iter()
        .map(|&e| e[0] * (1.0 - mean / length(e)))
        .sum::<f64>()
        / 4.0;
    let (up, aside) = ([0.0, 0.0, 1.0], [0.3, 0.0, 0.0]);
    let cases = [
        (MODES[0], up, [0.0, 0.0, 1.0 - l]),
        (MODES[1], up, [0.0, 0.0, 1.0 - 0.75 * l]),
        (
            MODES[2],
            up,
            [0.0, 0.0, (1.0 - l) * (1.0 - mu) + mu * l / 4.0],
        ),
        // Along the surface only: the pull is along the normal, so it stays.
        (MODES[3], up, up),
        (MODES[4], aside, [0.3 + l * relax, 0.0, 0.0]),
    ];
    for (mode, start, want) in cases {
        let mut mesh = grid(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
        mesh.positions[12] = start.map(|c| c as f32);
        let mut s = agent();
        s.add_part("grid", &mesh);
        let result = smooth(&mut s, call("grid", mode, ROUND));
        assert_eq!(result["rounds"], json!(1), "{mode:?}");
        let got = points(&mut s, "grid")[12];
        assert!(
            (0..3).all(|k| (f64::from(got[k]) - want[k]).abs() < 1e-6),
            "{mode:?}: the middle point is at {got:?}, by hand {want:?}"
        );
    }
}

#[test]
fn an_amount_is_its_rounds_run_one_by_one() {
    for mode in MODES {
        let mut once = agent();
        lumpy(&mut once);
        let result = smooth(&mut once, call("sphere", mode, 3.0 * ROUND));
        assert_eq!(result["rounds"], json!(3), "{mode:?}");
        let want = bits(&points(&mut once, "sphere"));
        let mut by_one = agent();
        lumpy(&mut by_one);
        for _ in 0..3 {
            smooth(&mut by_one, call("sphere", mode, ROUND));
        }
        let got = bits(&points(&mut by_one, "sphere"));
        assert!(got == want, "{mode:?}: three rounds one by one");
        let lower = one_call(call("sphere", mode, 2.0 * ROUND + 1.0));
        assert!(lower == want, "{mode:?}: any amount of three rounds");
        let start = one_call(call("sphere", mode, 0.0));
        assert!(start != want, "{mode:?}: it smooths");
    }
}

#[test]
fn a_live_drag_through_the_session_lands_where_one_call_at_its_amount_does() {
    for mode in MODES {
        let log = log_path(&format!("smooth-drag-{}-{}", mode.0, mode.1));
        let config = EngineConfig {
            log_path: Some(log.clone()),
            ..EngineConfig::default()
        };
        let mut s = Session::with(config, Role::Human);
        lumpy(&mut s);
        let (before, steps) = (s.hash(), undoable(&mut s));
        let deformer = |s: &mut Session, phase: &str, amount: f32| {
            let args = json!({ "phase": phase, "smooth": faded(mode, amount) });
            success(s.as_human("gizmo.deformer", args), phase);
        };
        deformer(&mut s, "open", 0.0);
        // Up adds rounds, the same rounds again change nothing, down reverts and runs fewer.
        for amount in [30.0, 70.0, 110.0, 120.0, 60.0, 200.0] {
            deformer(&mut s, "update", amount);
            let got = bits(&points(&mut s, "sphere"));
            assert!(
                got == one_call(faded(mode, amount)),
                "{mode:?}: dragged to {amount}"
            );
        }
        let state = s.query("session.state", json!({}));
        assert_eq!(state["deformer"], json!("smooth"), "{mode:?}");
        assert_eq!(state["deformer.smooth"]["amount"], json!(200.0), "{mode:?}");
        assert_eq!(
            undoable(&mut s),
            steps,
            "{mode:?}: nothing committed while open"
        );
        success(
            s.as_human("gizmo.deformer", json!({ "phase": "accept" })),
            "accept",
        );
        let after = s.hash();
        assert_eq!(undoable(&mut s), steps + 1, "{mode:?}: one undo step");
        let (_, records) = log_records(&log);
        let lines: Vec<&Value> = (records.iter())
            .filter(|r| r.command == "mesh.smooth")
            .map(|r| &r.args)
            .collect();
        assert!(
            lines.len() == 1 && lines[0]["amount"] == json!(200.0),
            "{mode:?}: one line at the last amount: {lines:?}"
        );
        let mut replay = agent();
        replay.replay_log(&log);
        assert_eq!(replay.hash(), after, "{mode:?}: the line replays it");
        s.ok("history.undo", json!({}));
        assert_eq!(s.hash(), before, "{mode:?}: undo puts every bit back");
        remove(&log);
    }
}

#[test]
fn a_negative_amount_sharpens_once_away_from_each_points_start_mean() {
    // The middle point raised 1 over its neighbours' mean of 0, and the point beside it a
    // quarter below its own mean; for "strong" the middle point is three quarters above the
    // mean of its neighbours' means, and the point beside it on its own.
    let mut mesh = grid(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
    mesh.positions[12][2] = 1.0;
    let cases = [
        ("smooth", -50.0, 1.5, -0.125),
        ("smooth", -100.0, 2.0, -0.25),
        ("strong", -50.0, 1.375, 0.0),
    ];
    for (mode, amount, middle, beside) in cases {
        let mut s = agent();
        s.add_part("grid", &mesh);
        let result = smooth(&mut s, call("grid", (mode, true), amount));
        assert_eq!(result["rounds"], json!(0), "{mode} at {amount}");
        let got = points(&mut s, "grid");
        assert!(
            (f64::from(got[12][2]) - middle).abs() < 1e-6
                && (f64::from(got[13][2]) - beside).abs() < 1e-6,
            "{mode} at {amount}: {:?} and {:?}, by hand {middle} and {beside}",
            got[12],
            got[13]
        );
    }
    let mut s = agent();
    s.add_part("grid", &mesh);
    for mode in [("polish", true), ("polish", false), ("relax", true)] {
        let refused = s.err("mesh.smooth", call("grid", mode, -10.0));
        assert_eq!(refused.code, ErrorCode::ArgsInvalid, "{mode:?}");
    }
}

#[test]
fn polish_keeping_the_volume_holds_it_within_the_measured_tolerance() {
    let volume = |s: &mut Session| {
        let stats = s.query("mesh.stats", json!({ "part": "sphere" }));
        stats["parts"][0]["volume"].as_f64().unwrap_or(f64::NAN)
    };
    let change = |mode| {
        let mut s = agent();
        lumpy(&mut s);
        let start = volume(&mut s);
        smooth(&mut s, call("sphere", mode, 500.0));
        volume(&mut s) / start - 1.0
    };
    let kept = change(("polish", true));
    let shrunk = change(("smooth", true));
    eprintln!("polish with keep at 500: {kept:+.6}; smooth at 500: {shrunk:+.6}");
    assert!(
        kept.abs() < KEPT,
        "polish with keep changed the volume by {kept}"
    );
    assert!(shrunk < -10.0 * KEPT, "smooth shrinks it: {shrunk}");
}

/// The most Polish with `keep` at 500 changes the lumpy sphere's volume, a share: measured
/// +0.083 percent (smooth shrinks it by 54 percent), held at 0.2 percent, inside the catalog's
/// Check of 1 percent.
const KEPT: f64 = 2e-3;

#[test]
fn boundary_points_move_along_the_boundary_only() {
    // A flat open grid, unevenly spaced, its inside points lifted and its corners masked.
    let at = [-1.0, -0.7, -0.1, 0.2, 0.6, 1.0];
    let n = at.len();
    let side = |i: usize| {
        [
            i.is_multiple_of(n) || i % n == n - 1,
            i / n == 0 || i / n == n - 1,
        ]
    };
    let mut mesh = grid(&at);
    for (i, p) in mesh.positions.iter_mut().enumerate() {
        if side(i) == [false, false] {
            p[2] = 0.3 * libm::sinf(i as f32 * 1.7);
        }
    }
    let corners = (0..n * n).map(|i| f32::from(u8::from(side(i) == [true, true])));
    let mesh = masked(mesh, corners.collect());
    for mode in MODES {
        let mut s = agent();
        s.add_part("grid", &mesh);
        smooth(&mut s, call("grid", mode, 5.0 * ROUND));
        let got = points(&mut s, "grid");
        let mut along = false;
        for (i, (p, q)) in got.iter().zip(&mesh.positions).enumerate() {
            let [across, up] = side(i);
            if !(across || up) {
                continue;
            }
            assert_eq!(
                p[2].to_bits(),
                q[2].to_bits(),
                "{mode:?}: point {i} left the plane"
            );
            // Strong's mean of means rounds the masked corners' neighbours inward.
            if mode.0 != "strong" {
                let same = |k: usize| p[k].to_bits() == q[k].to_bits();
                let kept = (!across || same(0)) && (!up || same(1));
                assert!(kept, "{mode:?}: point {i} left its side: {q:?} to {p:?}");
            }
            along |= p.map(f32::to_bits) != q.map(f32::to_bits);
        }
        assert!(along, "{mode:?}: the sides' points slide along them");
    }
}

#[test]
fn a_fully_masked_point_keeps_every_bit() {
    let mut s = agent();
    lumpy(&mut s);
    let sphere = s.mesh("sphere");
    let mask: Vec<f32> = (sphere.positions.iter())
        .map(|p| match p[0] {
            x if x > 0.3 => 1.0,
            x if x < -0.8 => 3.0,
            _ if p[1].abs() < 0.1 => 0.5,
            _ => 0.0,
        })
        .collect();
    let held: Vec<bool> = mask.iter().map(|&w| w >= 1.0).collect();
    let mesh = masked(sphere, mask);
    let calls = (MODES.iter().map(|&m| call("part", m, 200.0)))
        .chain([("smooth", true), ("strong", true)].map(|m| call("part", m, -60.0)));
    for args in calls {
        let mut s = agent();
        s.add_part("part", &mesh);
        smooth(&mut s, args.clone());
        let got = points(&mut s, "part");
        let (mut moved, mut kept) = (false, true);
        for ((p, q), &h) in got.iter().zip(&mesh.positions).zip(&held) {
            kept &= !h || p.map(f32::to_bits) == q.map(f32::to_bits);
            moved |= !h && p.map(f32::to_bits) != q.map(f32::to_bits);
        }
        assert!(
            kept && moved,
            "{args}: masked points kept, the others moved"
        );
        let mut free = args;
        free["mask"] = json!(false);
        smooth(&mut s, free.clone());
        let after = points(&mut s, "part");
        let unheld = (after.iter().zip(&got).zip(&held))
            .any(|((p, q), &h)| h && p.map(f32::to_bits) != q.map(f32::to_bits));
        assert!(unheld, "{free}: without the mask they move");
    }
}

#[test]
fn symmetry_gives_a_mirrored_result_on_a_symmetric_part() {
    let mut start = agent();
    lumpy(&mut start);
    start.ok("mesh.symmetrize", json!({ "part": "sphere", "axis": "x" }));
    let before = points(&mut start, "sphere");
    // Keyed by bits with -0 read as 0, so a point on the plane is its own partner.
    let key = |p: [f32; 3]| p.map(|c| (c + 0.0).to_bits());
    let at: BTreeMap<[u32; 3], usize> = (before.iter().enumerate())
        .map(|(i, &p)| (key(p), i))
        .collect();
    let partner: Vec<usize> = (before.iter())
        .map(|p| at.get(&key([-p[0], p[1], p[2]])))
        .map(|j| j.copied().unwrap_or(usize::MAX))
        .collect();
    assert!(partner.iter().all(|&j| j != usize::MAX), "a symmetric part");
    let off = |p: &[[f32; 3]]| {
        (0..p.len())
            .map(|i| {
                let q = p[partner[i]];
                let d = [p[i][0] + q[0], p[i][1] - q[1], p[i][2] - q[2]];
                d.iter().fold(0.0f32, |m, c| m.max(c.abs()))
            })
            .fold(0.0, f32::max)
    };
    let fade = |mode, mirror: bool| {
        let mut args = call("sphere", mode, 3.0 * ROUND);
        args["frame"] = json!({ "origin": [0.5, 0.03, 0.02], "axes": [0.0, 0.0, 0.0, 1.0] });
        args["gradient"] = json!({ "axis": "x", "from": 1.0, "to": 0.0 });
        if mirror {
            args["symmetry"] = json!({ "mirror": { "x": true } });
        }
        args
    };
    for mode in MODES {
        let mut s = agent();
        lumpy(&mut s);
        s.ok("mesh.symmetrize", json!({ "part": "sphere", "axis": "x" }));
        smooth(&mut s, fade(mode, true));
        let got = points(&mut s, "sphere");
        let worst = off(&got);
        eprintln!("{mode:?}: the furthest from mirrored, {worst:e}");
        assert!(worst < 1e-5, "{mode:?}: mirrored within 1e-5, {worst}");
        let plane = (got.iter().zip(&before)).all(|(p, q)| q[0] != 0.0 || p[0] == 0.0);
        assert!(plane, "{mode:?}: points on the plane stay on it");
    }
    let mut s = agent();
    lumpy(&mut s);
    s.ok("mesh.symmetrize", json!({ "part": "sphere", "axis": "x" }));
    smooth(&mut s, fade(MODES[0], false));
    assert!(
        off(&points(&mut s, "sphere")) > 1e-3,
        "without it, not mirrored"
    );
}

#[test]
fn one_call_is_one_undo_step_and_one_resolved_line_that_replays_it() {
    let log = log_path("smooth-line");
    let mut s = seeded(5, Some(log.clone()));
    lumpy(&mut s);
    let (before, steps) = (s.hash(), undoable(&mut s));
    let fade = json!({ "amount": 150, "gradient": { "axis": "z", "from": 0.0, "to": 1.0 } });
    let result = smooth(&mut s, fade);
    let after = s.hash();
    assert_ne!(after, before);
    assert_eq!(undoable(&mut s), steps + 1, "one undo step");
    let state = s.query("session.state", json!({}));
    assert_eq!(state["history.undo"], json!("Smooth"), "named Smooth");
    let (_, records) = log_records(&log);
    let lines: Vec<&Value> = (records.iter())
        .filter(|r| r.command == "mesh.smooth")
        .map(|r| &r.args)
        .collect();
    assert_eq!(lines.len(), 1, "one line");
    let numbers = |v: &Value| {
        v.as_array()
            .is_some_and(|a| a.len() == 3 && a.iter().all(Value::is_number))
    };
    let line = lines[0];
    assert!(
        numbers(&line["frame"]["origin"]) && numbers(&line["box"]["lo"]),
        "the frame and the box written in: {line}"
    );
    assert_eq!(
        (&line["frame"], &line["box"]),
        (&result["frame"], &result["box"])
    );
    assert_eq!(result["rounds"], json!(3));
    let mut replay = seeded(5, None);
    replay.replay_log(&log);
    assert_eq!(replay.hash(), after, "the line replays it to the bit");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo puts every bit back");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "and redo every bit of it");
    remove(&log);
}

#[test]
fn the_same_bits_on_1_and_24_threads() {
    let run = |threads| {
        gs_par::with_threads(threads, || {
            let mut s = agent();
            lumpy(&mut s);
            for (i, mode) in MODES.into_iter().enumerate() {
                let mut args = faded(mode, 90.0 + 40.0 * i as f32);
                args["symmetry"] = json!({ "mirror": { "x": true } });
                smooth(&mut s, args);
            }
            smooth(&mut s, call("sphere", ("strong", true), -40.0));
            s.hash()
        })
    };
    assert_eq!(run(1), run(24));
}
