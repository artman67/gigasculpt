//! The session's symmetry (decision 0044): `symmetry.set` keeps each axis' switch and type and
//! the radial count, the master switch turns them all off and back on, `brush.set`'s shortcuts
//! set the same value, and the window's pen strokes with what is on.

use std::path::Path;

use gs_engine::{
    CallReport, PaletteInfo, PenDown, PenReads, PenStroke, PenTaken, Screenshot, UiDescription,
    UiPalette, WindowHost,
};
use gs_protocol::PenSample;
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke, stroke_lines};

/// A window that hands the engine `strokes` at the next call, as the studio hands it the pen's.
struct Window(Vec<PenStroke>);

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

/// A Draw stroke of the window's pen across the front of the sphere, right of its middle.
fn pen_stroke() -> Vec<PenStroke> {
    let sample = |i: u32| {
        let x = 360.0 + 6.0 * f64::from(i);
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
    strokes.push(PenStroke::Up);
    strokes
}

/// The window's pen stroke on the sphere of `s`, logging to `log`, undone after: the x of each
/// point it moved, and the `symmetry` its log line carries.
fn pen(s: &mut Session, log: &Path) -> (Vec<f32>, Value) {
    let before = s.mesh("sphere");
    s.engine.attach_window(Box::new(Window(pen_stroke())));
    s.ok("brush.set", json!({}));
    let after = s.mesh("sphere");
    s.ok("history.undo", json!({}));
    let xs = (before.positions.iter().zip(&after.positions))
        .filter(|(a, b)| a.map(f32::to_bits) != b.map(f32::to_bits))
        .map(|(a, _)| a[0])
        .collect();
    let lines = stroke_lines(log);
    let last = lines
        .last()
        .unwrap_or_else(|| panic!("the pen's stroke is logged"));
    (xs, last.args["symmetry"].clone())
}

/// The symmetry values of `session.state`, in the order the decision lists them.
fn values(s: &mut Session) -> Value {
    let state = s.query("session.state", json!({}));
    let names = ["on", "x", "y", "z", "x_type", "y_type", "z_type"];
    let names = names.into_iter().chain(["radial_axis", "radial_count"]);
    names
        .map(|n| state[format!("symmetry.{n}")].clone())
        .collect()
}

#[test]
fn the_master_switch_off_mirrors_nothing_and_on_brings_the_axes_back() {
    let log = log_path("symmetry-master");
    let mut s = sphere_session(Some(log.clone()));
    s.ok("symmetry.set", json!({ "x": true }));
    let (xs, sym) = pen(&mut s, &log);
    let sides = |xs: &[f32]| (xs.iter().any(|&x| x > 0.1), xs.iter().any(|&x| x < -0.1));
    assert_eq!(sides(&xs), (true, true), "X on: both sides moved");
    assert_eq!(sym["mirror"]["x"], json!(true), "{sym}");

    let off = s.query("symmetry.set", json!({ "on": false }));
    assert_eq!(
        [&off["on"], &off["x"]],
        [&json!(false), &json!(true)],
        "{off}"
    );
    let (xs, sym) = pen(&mut s, &log);
    assert_eq!(sides(&xs), (true, false), "off: only the pen's side moved");
    let none = json!({ "mirror": { "x": false, "y": false, "z": false } });
    assert_eq!(sym, none, "off: the stroke carries no symmetry");
    let shown = values(&mut s);
    assert_eq!(
        [&shown[0], &shown[1]],
        [&json!(false), &json!(true)],
        "{shown}"
    );

    s.ok("symmetry.set", json!({ "on": true }));
    let (xs, _) = pen(&mut s, &log);
    assert_eq!(sides(&xs), (true, true), "on again: X is back");
    std::fs::remove_file(&log).ok();
}

#[test]
fn symmetry_set_keeps_each_setting_and_one_axis_at_most_is_radial() {
    let mut s = Session::new();
    let fresh = json!([
        true, false, false, false, "mirror", "mirror", "mirror", null, 8
    ]);
    assert_eq!(
        values(&mut s),
        fresh,
        "the master on, every axis off in Mirror"
    );

    let set = json!({ "x": true, "x_type": "parallel", "z_type": "radial", "radial_count": 5.6 });
    assert_eq!(
        s.query("symmetry.set", set)["radial_count"],
        json!(6),
        "rounds"
    );
    let parallel = json!({ "x": true, "y": false, "z": false });
    let brush = s.query("brush.set", json!({}))["symmetry"].clone();
    assert_eq!(
        brush["parallel"], parallel,
        "X goes Parallel, Z is off: {brush}"
    );
    assert!(brush.get("radial").is_none(), "{brush}");
    s.ok("symmetry.set", json!({ "z": true }));
    let brush = s.query("brush.set", json!({}))["symmetry"].clone();
    assert_eq!(
        brush["radial"],
        json!({ "axis": "z", "count": 6 }),
        "{brush}"
    );

    // Making Y Radial makes Z, which was, Mirror: on, it now mirrors.
    let moved = s.query("symmetry.set", json!({ "y_type": "radial" }));
    assert_eq!(
        [&moved["y_type"], &moved["z_type"]],
        ["radial", "mirror"],
        "{moved}"
    );
    let brush = s.query("brush.set", json!({}))["symmetry"].clone();
    assert_eq!(brush["mirror"]["z"], json!(true), "{brush}");
    assert!(brush.get("radial").is_none(), "Y is off: {brush}");

    let before = values(&mut s);
    let two = s.err(
        "symmetry.set",
        json!({ "x_type": "radial", "z_type": "radial" }),
    );
    assert_eq!(two.path.as_deref(), Some("/z_type"), "{two:?}");
    for bad in [65.0, 1.4] {
        let refused = s.err("symmetry.set", json!({ "radial_count": bad }));
        assert_eq!(code(refused.code), "E_ARGS_INVALID");
        assert_eq!(refused.path.as_deref(), Some("/radial_count"), "{bad}");
    }
    assert_eq!(values(&mut s), before, "a refused call changes nothing");
    let most = s.query("symmetry.set", json!({ "radial_count": 63.6 }));
    assert_eq!(most["radial_count"], json!(64), "rounds to the most");
}

#[test]
fn brush_set_s_shortcuts_set_the_same_value_and_strokes_refuse_two_types() {
    let mut s = sphere_session(None);
    s.ok("symmetry.set", json!({ "on": false, "y_type": "parallel" }));
    let old = json!({ "mirror": { "x": true }, "radial": { "axis": "z", "count": 4 } });
    let got = s.query("brush.set", json!({ "symmetry": old }))["symmetry_settings"].clone();
    let want = json!({
        "on": true, "x": true, "y": false, "z": true, "x_type": "mirror", "y_type": "parallel",
        "z_type": "radial", "radial_count": 4
    });
    assert_eq!(got, want, "the master on, Y keeps its type");

    // The X key's flip turns an axis on in its type.
    let flipped = s.query("brush.set", json!({ "flip_mirror": "y" }))["symmetry"].clone();
    assert_eq!(
        flipped["parallel"],
        json!({ "x": false, "y": true, "z": false })
    );
    // A radial axis from the stroke's form takes Radial from the one that had it.
    let around_x = json!({ "symmetry": { "radial": { "axis": "x", "count": 3 } } });
    let got = s.query("brush.set", around_x)["symmetry_settings"].clone();
    let types = [&got["x_type"], &got["y_type"], &got["z_type"]];
    assert_eq!(types, ["radial", "parallel", "mirror"], "{got}");

    let two = json!({ "mirror": { "z": true }, "radial": { "axis": "z", "count": 4 } });
    let refused = s.err("brush.set", json!({ "symmetry": two }));
    assert_eq!(refused.path.as_deref(), Some("/symmetry"), "{refused:?}");
    let both = json!({ "symmetry": { "mirror": { "x": true }, "parallel": { "x": true } } });
    let points = line([0.3, 0.2, 0.93], [0.4, 0.2, 0.89], 4);
    let refused = s.err("brush.stroke", stroke("Draw", &points, &both));
    assert_eq!(
        refused.path.as_deref(),
        Some("/symmetry/parallel"),
        "{refused:?}"
    );
    let moved = json!({ "op": { "move": [0.0, 0.1, 0.0] }, "symmetry": both["symmetry"] });
    let refused = s.err("transform.apply", moved);
    assert_eq!(
        refused.path.as_deref(),
        Some("/symmetry/parallel"),
        "{refused:?}"
    );
}
