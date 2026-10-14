//! The open deformer's preview on the graphics card (decision 0041, amended 2026-10-12 for
//! `DF9a`), as the engine sees it: while one of DF2's closed-form operations is open, an update
//! moves no point and changes no node, so the window has nothing to upload; Accept is today's,
//! bit for bit; Cancel and Reset change no node; every other path still moves the points at each
//! update; a query catches up to the committed points and the next update puts them back; and
//! the window's own traffic sees the points as they were at the open. Amended for `DF9b`: noise,
//! the cage and the curve go to the card too.

use std::sync::{Arc, Mutex, PoisonError};

use gs_core::{MeshStore, Work};
use gs_engine::{
    CallReport, PaletteInfo, PenReads, PenTaken, Screenshot, UiDescription, UiPalette, WindowHost,
};
use gs_protocol::PenSample;
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::support::human;

/// The deformers' frame: unturned, off the sphere's vertices.
const ORIGIN: [f32; 3] = [0.05, 0.03, 0.02];

/// What the window was handed after one call: how many nodes changed, and the sphere's store.
struct Seen {
    changed: usize,
    store: Option<MeshStore>,
}

/// A window that keeps what each call handed it.
#[derive(Clone, Default)]
struct Watch(Arc<Mutex<Vec<Seen>>>);

impl Watch {
    /// How many nodes the last call changed, the sphere's store after it and its work so far.
    fn last(&self) -> (usize, MeshStore, Work) {
        let seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let last = seen.last().unwrap_or_else(|| panic!("no call yet"));
        let store = (last.store.as_ref()).unwrap_or_else(|| panic!("no sphere"));
        (last.changed, store.share(), store.work())
    }
}

impl WindowHost for Watch {
    fn has_window(&self) -> bool {
        true
    }

    fn pen_input(&mut self, samples: &[PenSample], _reads: &PenReads) -> PenTaken {
        let cursor = samples.last().copied().filter(|s| s.in_proximity);
        PenTaken {
            cursor,
            gizmo: false,
        }
    }

    fn describe(&mut self) -> UiDescription {
        UiDescription::default()
    }

    fn screenshot(&mut self) -> Option<Result<Screenshot, String>> {
        None
    }

    fn palette(&mut self, _request: &UiPalette) -> Option<Result<PaletteInfo, String>> {
        None
    }

    fn after_call(&mut self, report: &CallReport) {
        let changed = report.parts.iter().map(|p| p.dirty.len()).sum();
        let sphere = report.parts.iter().find(|p| p.name == "sphere");
        let store = sphere.map(|p| p.store.share());
        let mut seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        seen.push(Seen { changed, store });
    }
}

/// A human's session with a sphere, watched by a window.
fn watched() -> (Session, Watch) {
    let mut s = human();
    let watch = Watch::default();
    s.engine.attach_window(Box::new(watch.clone()));
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    (s, watch)
}

/// `op` on the sphere about the deformers' frame, with the box the window always sends.
fn boxed(op: Value) -> Value {
    let frame = json!({ "origin": ORIGIN, "axes": [0.0, 0.0, 0.0, 1.0] });
    let bounds = json!({ "lo": [-1.05, -1.03, -1.02], "hi": [0.95, 0.97, 0.98] });
    let mut t = json!({ "part": "sphere", "frame": frame, "box": bounds });
    t["op"] = op;
    t
}

/// A bend of `degrees` toward x along y.
fn bend(degrees: f32) -> Value {
    boxed(json!({ "bend": { "axis": "y", "toward": "x", "degrees": degrees } }))
}

/// A twist of `degrees` about `axis`.
fn twist(axis: &str, degrees: f32) -> Value {
    boxed(json!({ "twist": { "axis": axis, "degrees": degrees } }))
}

/// A `gizmo.deformer` call with transforms.
fn deformer(s: &mut Session, phase: &str, transforms: &[Value]) {
    s.ok(
        "gizmo.deformer",
        json!({ "phase": phase, "transforms": transforms }),
    );
}

/// A `gizmo.deformer` call without transforms.
fn phase(s: &mut Session, phase: &str) {
    s.ok("gizmo.deformer", json!({ "phase": phase }));
}

/// The sphere's points, as `part.get` reports them.
fn points(s: &mut Session) -> Vec<[f32; 3]> {
    s.mesh("sphere").positions
}

/// Asserts the last call changed no node and left the sphere as `open` holds it, with no point
/// work since `work`.
fn untouched(watch: &Watch, open: &MeshStore, work: Work, what: &str) {
    let (changed, store, now) = watch.last();
    assert_eq!(changed, 0, "{what}: no node changed");
    assert!(
        store.same_content(open),
        "{what}: the points are the open's"
    );
    assert_eq!(
        (now.examined, now.normals),
        (work.examined, work.normals),
        "{what}: no point was worked on"
    );
}

#[test]
fn an_update_on_the_card_moves_no_point_and_changes_no_node() {
    let (mut s, watch) = watched();
    let (_, open, work) = watch.last();
    deformer(&mut s, "open", &[bend(0.0)]);
    untouched(&watch, &open, work, "the open");
    for degrees in [20.0, 45.0, 70.0] {
        deformer(&mut s, "update", &[bend(degrees)]);
        untouched(&watch, &open, work, "an update");
    }
    phase(&mut s, "lift");
    untouched(&watch, &open, work, "the lift");
    // Two twists, as a pull with X and Z lit sends them: one chain on the card.
    phase(&mut s, "cancel");
    deformer(&mut s, "open", &[twist("x", 0.0), twist("z", 0.0)]);
    deformer(&mut s, "update", &[twist("x", 40.0), twist("z", 60.0)]);
    untouched(&watch, &open, work, "a chain");
}

#[test]
fn accept_is_one_step_and_the_points_one_transform_apply_gives() {
    for transforms in [vec![bend(70.0)], vec![twist("x", 40.0), twist("z", 60.0)]] {
        let (mut s, _) = watched();
        let steps = s.history()["undoable"].as_u64();
        deformer(&mut s, "open", &transforms);
        for t in [0.5_f64, 1.0] {
            let scaled: Vec<Value> = (transforms.iter()).map(|call| scaled(call, t)).collect();
            deformer(&mut s, "update", &scaled);
        }
        phase(&mut s, "accept");
        let history = s.history();
        assert_eq!(
            history["undoable"].as_u64(),
            steps.map(|n| n + 1),
            "one undo step"
        );
        let label = s.query("session.state", json!({}))["history.undo"].clone();
        let named = if transforms.len() == 1 {
            "Bend"
        } else {
            "Twist"
        };
        assert!(
            label.as_str().is_some_and(|l| l.starts_with(named)),
            "named after the deformer: {label}"
        );
        let mut direct = human();
        direct.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        for call in &transforms {
            direct.ok("transform.apply", call.clone());
        }
        let (accepted, applied) = (points(&mut s), points(&mut direct));
        assert!(
            (accepted.iter().zip(&applied))
                .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)),
            "Accept's points are transform.apply's, bit for bit"
        );
    }
}

/// `call` with its angle at `t` of the way.
fn scaled(call: &Value, t: f64) -> Value {
    let mut call = call.clone();
    for op in ["bend", "twist"] {
        if let Some(d) = call["op"][op]["degrees"].as_f64() {
            call["op"][op]["degrees"] = json!(d * t);
        }
    }
    call
}

#[test]
fn cancel_and_reset_change_no_node() {
    let (mut s, watch) = watched();
    let before = s.hash();
    let (_, open, work) = watch.last();
    deformer(&mut s, "open", &[bend(0.0)]);
    deformer(&mut s, "update", &[bend(60.0)]);
    phase(&mut s, "reset");
    untouched(&watch, &open, work, "Reset");
    deformer(&mut s, "update", &[bend(30.0)]);
    phase(&mut s, "cancel");
    untouched(&watch, &open, work, "Cancel");
    assert_eq!(s.hash(), before, "Cancel leaves every bit");
}

/// Noise, the hard and the soft cage, and curves through three points and 32, as the card takes
/// them since `DF9b`: each with the call that opens it and the one that updates it.
fn drawn() -> Vec<(&'static str, Value, Value)> {
    let noise =
        |amount: f32| boxed(json!({ "noise": { "amount": amount, "scale": 0.4, "seed": 9 } }));
    let cage = |kind: &str, lift: f32| {
        let offsets = json!([{ "at": [1, 2, 1], "by": [0.0, lift, 0.1] }]);
        boxed(json!({ "lattice": { "kind": kind, "size": [3, 3, 3], "offsets": offsets } }))
    };
    let three = |degrees: f32| {
        let points = json!([[0.0, -1.0, 0.0], [0.4, 0.0, 0.0], [0.0, 1.0, 0.2]]);
        let twist = json!([0.0, degrees, 2.0 * degrees]);
        let curve =
            json!({ "axis": "y", "points": points, "twist": twist, "scale": [1.0, 1.2, 0.8] });
        boxed(json!({ "curve": curve }))
    };
    let wave = |degrees: f32| {
        let points: Vec<[f32; 3]> = (0..32)
            .map(|i| {
                let y = -1.0 + 2.0 * i as f32 / 31.0;
                [0.3 * libm::sinf(3.0 * y), y, 0.2 * libm::cosf(2.0 * y)]
            })
            .collect();
        let twist: Vec<f32> = (0..32).map(|i| degrees * i as f32 / 31.0).collect();
        let curve =
            json!({ "axis": "y", "points": points, "twist": twist, "scale": vec![1.1; 32] });
        let mut call = boxed(json!({ "curve": curve }));
        call["gradient"] = json!({ "axis": "y", "from": 0.0, "to": 1.0 });
        call
    };
    vec![
        ("noise", noise(0.0), noise(0.05)),
        ("the hard cage", cage("hard", 0.0), cage("hard", 0.3)),
        ("the soft cage", cage("soft", 0.0), cage("soft", 0.3)),
        ("a curve through three points", three(0.0), three(40.0)),
        ("a curve through 32 under a gradient", wave(0.0), wave(90.0)),
    ]
}

#[test]
fn an_update_of_noise_the_cage_or_the_curve_moves_no_point_and_changes_no_node() {
    for (what, open, update) in drawn() {
        let (mut s, watch) = watched();
        let (_, before, work) = watch.last();
        deformer(&mut s, "open", &[open]);
        untouched(&watch, &before, work, what);
        deformer(&mut s, "update", std::slice::from_ref(&update));
        untouched(&watch, &before, work, what);
        phase(&mut s, "accept");
        let mut direct = human();
        direct.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        direct.ok("transform.apply", update);
        assert_eq!(s.hash(), direct.hash(), "{what}: Accept is today's");
    }
}

/// Sessions the card does not take: each update moves the points on the CPU, as before.
#[test]
fn every_other_session_still_moves_the_points_at_each_update() {
    // Noise of more than ten cells, which the card's rounding of the place would miss by more
    // than its tolerance allows (decision 0041, amended 2026-10-13, section 1).
    let speckle = boxed(json!({ "noise": { "amount": 0.5, "scale": 0.04, "seed": 9 } }));
    let mut mirrored = bend(50.0);
    mirrored["symmetry"] = json!({ "mirror": { "x": true } });
    let soft = |mut call: Value| {
        call["falloff"] = json!(0.0);
        call["reach"] = json!(1.5);
        call
    };
    let turned = soft(boxed(json!({ "rotate": { "axis": "z", "degrees": 30 } })));
    let soft = soft(twist("y", 50.0));
    let cases: Vec<(&str, Vec<Value>)> = vec![
        ("noise of more than ten cells", vec![speckle]),
        ("a split by a mirror plane", vec![mirrored]),
        ("five in a chain", vec![bend(10.0); 5]),
        ("a chain under a falloff", vec![soft.clone(), soft]),
        ("a soft rotate", vec![turned]),
    ];
    for (what, transforms) in cases {
        let (mut s, watch) = watched();
        let (_, open, _) = watch.last();
        deformer(&mut s, "open", &transforms);
        deformer(&mut s, "update", &transforms);
        let (changed, store, _) = watch.last();
        assert!(
            changed > 0 && !store.same_content(&open),
            "{what}: the update moved the points"
        );
    }
    // Smoothing, and a part with detail levels.
    let (mut s, watch) = watched();
    let (_, open, _) = watch.last();
    for (phase, amount) in [("open", 0), ("update", 60)] {
        let smooth = json!({ "part": "sphere", "mode": "smooth", "keep": true, "amount": amount });
        s.ok(
            "gizmo.deformer",
            json!({ "phase": phase, "smooth": smooth }),
        );
    }
    let (changed, store, _) = watch.last();
    assert!(changed > 0 && !store.same_content(&open), "smoothing");
    let (mut s, watch) = watched();
    s.ok("level.subdivide", json!({}));
    let (_, open, _) = watch.last();
    deformer(&mut s, "open", &[bend(0.0)]);
    deformer(&mut s, "update", &[bend(40.0)]);
    let (changed, store, _) = watch.last();
    assert!(
        changed > 0 && !store.same_content(&open),
        "a part with levels"
    );
}

#[test]
fn a_query_sees_the_committed_points_and_the_next_update_previews_again() {
    let (mut s, watch) = watched();
    let (_, open, _) = watch.last();
    let mut direct = human();
    direct.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    direct.ok("transform.apply", bend(70.0));
    deformer(&mut s, "open", &[bend(0.0)]);
    deformer(&mut s, "update", &[bend(70.0)]);
    // The gizmo stays where it was at the open while one is open, so `gizmo.describe` is no
    // probe of the points here.
    assert_eq!(
        s.query("mesh.bounds", json!({})),
        direct.query("mesh.bounds", json!({})),
        "mesh.bounds sees the committed points"
    );
    assert_eq!(points(&mut s), points(&mut direct), "part.get, bit for bit");
    deformer(&mut s, "update", &[bend(50.0)]);
    let (changed, store, _) = watch.last();
    assert!(
        changed > 0 && store.same_content(&open),
        "the next update puts the points back for the preview"
    );
    phase(&mut s, "accept");
    let mut once = human();
    once.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    once.ok("transform.apply", bend(50.0));
    assert_eq!(s.hash(), once.hash(), "Accept after a catch-up is today's");
}

#[test]
fn the_window_s_own_calls_see_the_points_of_the_open() {
    let (mut s, watch) = watched();
    let (_, open, work) = watch.last();
    deformer(&mut s, "open", &[bend(0.0)]);
    deformer(&mut s, "update", &[bend(70.0)]);
    let hover = json!({ "samples": [{ "x": 320.0, "y": 200.0 }] });
    let calls = [
        ("session.state", json!({})),
        ("ui.describe", json!({})),
        ("pen.input", hover),
    ];
    for (command, args) in calls {
        s.ok(command, args);
        untouched(&watch, &open, work, command);
    }
    let state = s.query("session.state", json!({}));
    assert_eq!(state["deformer"], json!("bend"), "still open");
}
