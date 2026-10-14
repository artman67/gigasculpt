//! Pen feel (decision 0036), through the commands: pen pressure can size the dab, on one brush
//! or on every brush, and does not as shipped; Stroke Smoothing can be set on every brush and
//! steadies a shaky path; Alt can change at any sample of a stroke, and the log replays it.

use std::path::Path;

use gs_engine::{
    CallReport, EngineConfig, PaletteInfo, PenDown, PenReads, PenStroke, PenTaken, Screenshot,
    UiDescription, UiPalette, WindowHost,
};
use gs_protocol::{PenSample, PlainMesh, Role};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::{log_path, sphere_session, stroke_lines, with};

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

/// The unit sphere in a session that logs to `log`, each line with the hash after it.
pub(crate) fn logging(log: &Path) -> Session {
    let config = EngineConfig {
        log_path: Some(log.to_path_buf()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    s
}

/// `p` moved onto the unit sphere.
fn on_sphere(p: [f32; 3]) -> [f32; 3] {
    let n = p.iter().map(|x| x * x).sum::<f32>().sqrt();
    p.map(|x| x / n)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|k| (a[k] - b[k]) * (a[k] - b[k]))
        .sum::<f32>()
        .sqrt()
}

/// Whether `a` and `b` are the same bits.
fn same(a: [f32; 3], b: [f32; 3]) -> bool {
    a.map(f32::to_bits) == b.map(f32::to_bits)
}

/// The start positions of the vertices that moved from `before` to `after`.
fn moved(before: &PlainMesh, after: &PlainMesh) -> Vec<[f32; 3]> {
    (before.positions.iter().zip(&after.positions))
        .filter(|(a, b)| !same(**a, **b))
        .map(|(a, _)| *a)
        .collect()
}

/// Where the pen presses, on the front of the sphere.
const PRESS: [f32; 3] = [0.1, 0.05, 0.99];

/// How far from the press the farthest vertex one dab of `brush` moves lies, at `pressure`,
/// with `extra` arguments. The dab is undone after.
fn reach(s: &mut Session, brush: &str, pressure: f32, extra: &Value) -> f32 {
    let before = s.mesh("sphere");
    let sample = json!({ "position": PRESS, "pressure": pressure });
    let args = json!({ "brush": brush, "size": 0.3, "samples": [sample] });
    s.ok("brush.stroke", with(args, extra));
    let after = s.mesh("sphere");
    s.ok("history.undo", json!({}));
    (moved(&before, &after).into_iter())
        .map(|p| distance(p, PRESS))
        .fold(0.0, f32::max)
}

#[test]
fn pressure_sizes_the_dab_once_a_size_curve_is_on() {
    let curve = json!([[0, 0.25], [1, 1]]);
    let mut s = sphere_session(None);
    let none = json!({});
    let (light, heavy) = (
        reach(&mut s, "Draw", 0.2, &none),
        reach(&mut s, "Draw", 1.0, &none),
    );
    assert!(
        heavy > 0.25 && (light - heavy).abs() < 0.02,
        "as shipped, pressure leaves the size alone: {light} and {heavy}"
    );
    // On the brush in hand: its own curve, carried into the stroke as the window carries it.
    let own = set(
        &mut s,
        json!({ "brush": "Draw", "settings": { "pressure": { "size": curve } } }),
    );
    let carried = json!({ "settings": own["settings"] });
    let (light, heavy) = (
        reach(&mut s, "Draw", 0.2, &carried),
        reach(&mut s, "Draw", 1.0, &carried),
    );
    assert!(
        light < 0.6 * heavy,
        "on the brush, a light touch makes a smaller dab: {light} against {heavy}"
    );
    // On every brush: the session's curve reaches a brush without `pressure.global`.
    set(&mut s, json!({ "global_pressure": { "size": curve } }));
    let (light, heavy) = (
        reach(&mut s, "Inflate", 0.2, &none),
        reach(&mut s, "Inflate", 1.0, &none),
    );
    assert!(
        light < 0.6 * heavy,
        "on every brush, a light touch makes a smaller dab: {light} against {heavy}"
    );
}

#[test]
fn the_global_size_curve_is_pinned_in_the_stroke_and_spares_the_move_brushes() {
    let log = log_path("size-curve");
    let mut s = logging(&log);
    let curve = json!([[0, 0.25], [1, 1]]);
    set(&mut s, json!({ "global_pressure": { "size": curve } }));
    let pressed = |points: &[[f32; 3]]| -> Vec<Value> {
        let n = points.len() as f32;
        (points.iter().enumerate())
            .map(|(i, p)| json!({ "position": p, "pressure": (i as f32 + 1.0) / n }))
            .collect()
    };
    let path = [[-0.2, 0.1, 0.97], [0.0, 0.1, 0.99], [0.2, 0.1, 0.97]].map(on_sphere);
    s.ok(
        "brush.stroke",
        json!({ "brush": "Draw", "size": 0.2, "samples": pressed(&path) }),
    );
    let grab = [[0.0, 0.0, 1.0], [0.1, 0.1, 1.0], [0.2, 0.15, 1.0]];
    s.ok(
        "brush.stroke",
        json!({ "brush": "Move", "size": 0.2, "samples": pressed(&grab) }),
    );
    let lines = stroke_lines(&log);
    assert_eq!(
        lines[0].args["settings"]["pressure"]["size"], curve,
        "Draw strokes with the session's curve, written into the stroke"
    );
    assert!(
        lines[1].args.get("settings").is_none(),
        "Move keeps its own flat size curve: {}",
        lines[1].args
    );
    // A session without the curve replays both lines alone.
    let mut replay = sphere_session(None);
    for line in &lines {
        let request = line.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), "the logged stroke");
        assert_eq!(Some(replay.hash()), line.hash, "replays exactly");
    }
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_size_curve_set_and_taken_back_leaves_strokes_as_they_were() {
    let path: Vec<Value> = ([[-0.3, 0.2, 0.93], [0.0, 0.1, 0.99], [0.3, 0.1, 0.95]])
        .map(on_sphere)
        .iter()
        .zip([0.3, 0.7, 1.0])
        .map(|(p, pressure)| json!({ "position": p, "pressure": pressure }))
        .collect();
    let args = json!({ "brush": "Draw", "size": 0.2, "samples": path });
    let (log_a, log_b) = (log_path("size-never"), log_path("size-back"));
    let mut a = logging(&log_a);
    a.ok("brush.stroke", args.clone());
    let mut b = logging(&log_b);
    let curve = json!([[0, 0.25], [1, 1]]);
    set(&mut b, json!({ "global_pressure": { "size": curve } }));
    set(
        &mut b,
        json!({ "brush": "Draw", "settings": { "pressure": { "size": curve } } }),
    );
    set(&mut b, json!({ "global_pressure": { "size": null } }));
    set(
        &mut b,
        json!({ "settings": { "pressure": { "size": null } } }),
    );
    b.ok("brush.stroke", args);
    assert_eq!(a.hash(), b.hash(), "the same bits");
    let (la, lb) = (stroke_lines(&log_a), stroke_lines(&log_b));
    assert_eq!(la[0].args, lb[0].args, "the same log line");
    assert!(la[0].args.get("settings").is_none(), "{}", la[0].args);
    std::fs::remove_file(&log_a).ok();
    std::fs::remove_file(&log_b).ok();
}

/// A path along y = 0.1 across the front of the sphere that shakes `shake` up and down at
/// every sample after the first.
fn shaky(shake: f32) -> Vec<[f32; 3]> {
    (0..=24)
        .map(|i| {
            let dy = match i {
                0 => 0.0,
                _ if i % 2 == 0 => shake,
                _ => -shake,
            };
            on_sphere([-0.3 + 0.025 * i as f32, 0.1 + dy, 0.95])
        })
        .collect()
}

/// The sphere before and after a stroke of `brush` along `path`, with `extra` arguments.
fn stroked(brush: &str, path: &[[f32; 3]], extra: &Value) -> (PlainMesh, PlainMesh) {
    let mut s = sphere_session(None);
    let before = s.mesh("sphere");
    let samples: Vec<Value> = path.iter().map(|p| json!({ "position": p })).collect();
    let args = json!({ "brush": brush, "size": 0.1, "samples": samples });
    s.ok("brush.stroke", with(args, extra));
    (before, s.mesh("sphere"))
}

/// How far across its line a stroke of `brush` along `path` reached, with `extra` arguments:
/// the largest `|y - 0.1|` of a vertex it moved.
fn spread(brush: &str, path: &[[f32; 3]], extra: &Value) -> f32 {
    let (before, after) = stroked(brush, path, extra);
    (moved(&before, &after).into_iter())
        .map(|p| (p[1] - 0.1).abs())
        .fold(0.0, f32::max)
}

#[test]
fn stroke_smoothing_reaches_every_brush_and_steadies_a_shaky_path() {
    let mut s = Session::new();
    let list = s.query("brush.list", json!({}));
    let names: Vec<String> = (list["brushes"].as_array().into_iter().flatten())
        .filter_map(|b| b["name"].as_str().map(String::from))
        .collect();
    assert!(names.len() >= 19, "{names:?}");
    let on = json!({ "stroke": { "smoothing": { "radius": 1.0 } } });
    for name in &names {
        let got = set(&mut s, json!({ "brush": name, "settings": on }));
        assert_eq!(got["settings"], on, "{name} takes Stroke Smoothing");
    }
    // The string is one brush radius long and the path shakes less than that across its line,
    // so the paint point keeps close to the line.
    let path = shaky(0.04);
    let smoothed = json!({ "settings": on });
    let (without, with_it) = (
        spread("Draw", &path, &json!({})),
        spread("Draw", &path, &smoothed),
    );
    assert!(
        with_it < without - 0.02,
        "smoothing keeps the stroke near its line: {with_it} against {without}"
    );
    // The brushes that hold their path until the stroke ends smooth it too: a curve stroke
    // its path, a gesture its line.
    for brush in ["Clay Line", "Clip Curve"] {
        let (before, without) = stroked(brush, &path, &json!({}));
        let (_, with_it) = stroked(brush, &path, &smoothed);
        assert!(
            !moved(&before, &without).is_empty(),
            "{brush} changed the sphere"
        );
        assert!(
            !moved(&without, &with_it).is_empty(),
            "{brush} follows the smoothed path"
        );
    }
}

/// A Draw stroke along the front of the sphere, from x = -0.4 to 0.4 in 16 steps, with Alt
/// held from sample `alt` on.
fn alt_from(alt: Option<usize>) -> Value {
    let samples: Vec<Value> = (0..=16)
        .map(|i| {
            let p = on_sphere([-0.4 + 0.05 * i as f32, 0.1, 0.92]);
            match alt {
                Some(k) if i >= k => json!({ "position": p, "invert": true }),
                _ => json!({ "position": p }),
            }
        })
        .collect();
    json!({ "brush": "Draw", "size": 0.1, "samples": samples })
}

#[test]
fn alt_pressed_halfway_changes_only_the_second_half_and_replays() {
    let log = log_path("alt-halfway");
    let mut s = logging(&log);
    let before = s.mesh("sphere");
    s.ok("brush.stroke", alt_from(None));
    let added = s.mesh("sphere");
    s.ok("history.undo", json!({}));
    // Alt from sample 8, at x = 0.
    s.ok("brush.stroke", alt_from(Some(8)));
    let switched = s.mesh("sphere");
    let radius = |p: &[f32; 3]| p.iter().map(|x| x * x).sum::<f32>().sqrt();
    let (mut first, mut second) = (0, 0);
    // How far the vertices past x = 0.1 moved out from the centre, without and with Alt.
    let (mut raised, mut dug) = (0.0, 0.0);
    for (i, p) in before.positions.iter().enumerate() {
        let (a, b) = (added.positions[i], switched.positions[i]);
        // The dabs laid up to sample 8 sit at x up to 0, and reach a brush radius (0.1) and a
        // dab's spacing past it; the ones before sit at x up to -0.05.
        if p[0] < -0.2 {
            assert!(same(a, b), "vertex {i} of the first half is the same bits");
            first += usize::from(!same(a, *p));
        } else if p[0] > 0.1 {
            second += usize::from(!same(a, b));
            raised += radius(&a) - radius(p);
            dug += radius(&b) - radius(p);
        }
    }
    // Per vertex, the edge of a dab moves some by less than a radius's last bit, so the sign
    // is read from the whole second half.
    assert!(
        raised > 0.0 && dug < 0.0,
        "Draw raises it ({raised}), Alt digs it ({dug})"
    );
    assert!(first > 0 && second > 0, "{first} and {second} vertices");
    // The log keeps the sign of each sample, and replays the stroke alone.
    let lines = stroke_lines(&log);
    let line = &lines[1];
    let signs: Vec<Option<bool>> = (line.args["samples"].as_array().into_iter().flatten())
        .map(|s| s["invert"].as_bool())
        .collect();
    assert_eq!(signs[..8], [None; 8], "{signs:?}");
    assert!(signs[8..].iter().all(|&x| x == Some(true)), "{signs:?}");
    let mut replay = sphere_session(None);
    let request = line.request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), line.hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_drag_takes_the_sign_of_its_latest_sample_as_a_whole() {
    // A drag goes back to its start at every update, so Alt pressed at the last sample moves
    // the whole grab along the normal, as Alt held from the start does.
    let grab = [[0.0, 0.0, 1.0], [0.1, 0.05, 1.05], [0.2, 0.1, 1.1]];
    let samples = |last: bool| -> Vec<Value> {
        (grab.iter().enumerate())
            .map(|(i, p)| match i {
                2 if last => json!({ "position": p, "invert": true }),
                _ => json!({ "position": p }),
            })
            .collect()
    };
    let mut s = sphere_session(None);
    let move_args =
        |samples: Vec<Value>| json!({ "brush": "Move", "size": 0.2, "samples": samples });
    s.ok("brush.stroke", move_args(samples(true)));
    let late = s.hash();
    s.ok("history.undo", json!({}));
    s.ok(
        "brush.stroke",
        with(move_args(samples(false)), &json!({ "invert": true })),
    );
    assert_eq!(s.hash(), late, "the same as Alt held throughout");
}

/// A window that hands the engine `strokes` at the next call, as the studio hands it the pen's.
pub(crate) struct Window(pub(crate) Vec<PenStroke>);

impl WindowHost for Window {
    fn has_window(&self) -> bool {
        false
    }

    fn pen_input(&mut self, _samples: &[PenSample], _reads: &PenReads) -> PenTaken {
        PenTaken {
            cursor: None,
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

    fn after_call(&mut self, _report: &CallReport) {}

    fn take_strokes(&mut self) -> Vec<PenStroke> {
        std::mem::take(&mut self.0)
    }
}

#[test]
fn the_window_s_alt_reaches_the_samples_after_it_and_the_log_replays() {
    let log = log_path("pen-alt");
    let mut s = logging(&log);
    let sample = |i: u32| {
        let x = 280.0 + 8.0 * f64::from(i);
        let raw = json!({ "x": x, "y": 200.0, "pressure": 0.8, "contact": true });
        PenStroke::Sample(serde_json::from_value(raw).unwrap_or_else(|e| panic!("{e}")))
    };
    let down = PenDown {
        viewport: [0, 0, 640, 400],
        classic: false,
        brush: Some("Draw".into()),
        invert: false,
    };
    let mut strokes = vec![PenStroke::Down(down)];
    strokes.extend((0..6).map(sample));
    strokes.push(PenStroke::Alt(true));
    strokes.extend((6..12).map(sample));
    strokes.push(PenStroke::Up);
    s.engine.attach_window(Box::new(Window(strokes)));
    s.ok("brush.set", json!({}));
    let lines = stroke_lines(&log);
    assert_eq!(lines.len(), 1, "the pen's stroke is logged");
    let signs: Vec<Option<bool>> = (lines[0].args["samples"].as_array().into_iter().flatten())
        .map(|s| s["invert"].as_bool())
        .collect();
    let want: Vec<Option<bool>> = [[None; 6], [Some(true); 6]].concat();
    assert_eq!(signs, want, "only the samples after Alt carry it");
    let mut replay = sphere_session(None);
    let request = lines[0].request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), lines[0].hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}
