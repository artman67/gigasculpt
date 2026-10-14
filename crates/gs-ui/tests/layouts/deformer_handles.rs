//! The open deformer's handles in the 3D view (decision 0041, amended 2026-10-13 for `DF7c`),
//! driven by the pen as the window drives it and found where `gizmo.describe` says they are for
//! the window's own camera: a cone's drag changes its one argument by the distance the pen goes
//! along it, one `update` a frame; a cage dot's drag moves only that point; a pen rectangle on
//! the model selects the dots inside and a drag moves them together; a curve dot's drag moves
//! its point; Accept keeps it as one step; and the box follows a gizmo turned before the pick.
//! A lit handle, the deformer's or the gizmo's, puts the brush's rings and lines away and keeps
//! the lines clear of it; a deformer handle says what it drags; and Escape shuts a slider's fine
//! handle before it cancels the deformer (amended 2026-10-13 for `DF7c-notes`). A held key's
//! line says what the key does to the deformer handle under the pen (UI-words).

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::rig::{Ui, held};
use crate::slider_alone::apart;

/// One deformer handle on screen, its outline in window pixels.
struct Handle {
    name: String,
    outline: Vec<[f64; 2]>,
}

impl Handle {
    /// A dot's middle, or the middle of a cone's base.
    fn middle(&self) -> [f64; 2] {
        let p = &self.outline;
        let (a, b) = if p.len() == 3 {
            (p[1], p[2])
        } else {
            (p[0], p[2])
        };
        [f64::midpoint(a[0], b[0]), f64::midpoint(a[1], b[1])]
    }
}

/// A sphere of radius 0.7 seen from a little right and above, the Gizmo tool on at `size`
/// points, and the deformer `pick` opened from the Brush tray's picker.
fn open(ui: &mut Ui, pick: &str, size: f32) {
    ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
    ui.ok("ui.gizmo", json!({ "tool": true, "size": size }));
    let turn = json!({ "view": "main", "right_deg": 30, "up_deg": 20 });
    ui.ok("view.orbit", turn);
    ui.settle();
    ui.press(&format!("brush > {pick}"));
}

/// `gizmo.describe` for the window's camera, and the 3D view's rectangle.
pub(crate) fn described(ui: &mut Ui) -> (Value, [f64; 4]) {
    let d = ui.describe();
    let v = &d["viewport"];
    let r: [f64; 4] = std::array::from_fn(|i| v["rect"][i].as_f64().unwrap_or_default());
    let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
    let args = json!({ "camera": v["camera"], "viewport": size });
    (ui.ok("gizmo.describe", args), r)
}

/// The handles `list`ed, their outlines moved into the window by the 3D view's corner.
fn listed(list: &Value, r: [f64; 4]) -> Vec<Handle> {
    let list = list.as_array().cloned().unwrap_or_default();
    (list.iter())
        .map(|h| Handle {
            name: h["name"].as_str().unwrap_or_default().into(),
            outline: (h["outline"].as_array().cloned().unwrap_or_default().iter())
                .map(|p| {
                    [
                        r[0] + p[0].as_f64().unwrap_or_default(),
                        r[1] + p[1].as_f64().unwrap_or_default(),
                    ]
                })
                .collect(),
        })
        .collect()
}

/// The open deformer as `gizmo.describe` reports it for the window's camera, and its handles.
fn shown(ui: &mut Ui) -> (Value, Vec<Handle>) {
    let (d, r) = described(ui);
    let handles = listed(&d["deformer"]["handles"], r);
    (d["deformer"].clone(), handles)
}

fn find<'a>(handles: &'a [Handle], name: &str) -> &'a Handle {
    let names: Vec<&str> = handles.iter().map(|h| h.name.as_str()).collect();
    (handles.iter())
        .find(|h| h.name == name)
        .unwrap_or_else(|| panic!("no {name} in {names:?}"))
}

fn sample(at: [f64; 2], down: bool) -> Value {
    let pressure = if down { 0.5 } else { 0.0 };
    json!({ "x": at[0], "y": at[1], "contact": down, "pressure": pressure })
}

/// The pen pressed at `from` with `held` held, then in one `pen.input` moved to `to` in four
/// samples, and one frame; then the lift at `to`. Returns how many `gizmo.deformer` updates
/// that frame asked for.
fn drag(ui: &mut Ui, from: [f64; 2], to: [f64; 2], held: Modifiers) -> usize {
    ui.input(&[], held);
    ui.touch(from[0], from[1], false);
    ui.touch(from[0], from[1], true);
    let along = |f: f64| {
        [
            from[0] + (to[0] - from[0]) * f,
            from[1] + (to[1] - from[1]) * f,
        ]
    };
    let samples: Vec<Value> = [0.25, 0.5, 0.75, 1.0]
        .map(|f| sample(along(f), true))
        .into();
    ui.ok("pen.input", json!({ "samples": samples }));
    let steps = {
        let mut studio = ui.host.lock();
        studio.frame().unwrap_or_else(|e| panic!("{e}"));
        let steps = studio.shell.take_commands();
        for step in &steps {
            studio.shell.queue(step.clone());
        }
        steps
    };
    ui.settle();
    ui.touch(to[0], to[1], false);
    ui.input(&[], Modifiers::default());
    let update =
        |s: &&gs_protocol::Step| s.command == "gizmo.deformer" && s.args["phase"] == "update";
    steps.iter().filter(update).count()
}

/// The open deformer's first call, as `session.state` reports it.
fn call(ui: &mut Ui) -> Value {
    ui.state()["deformer.transforms"][0].clone()
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn xyz(v: &Value) -> [f64; 3] {
    [0, 1, 2].map(|i| number(&v[i]))
}

/// A dot well apart from every other on screen.
fn alone(handles: &[Handle]) -> &Handle {
    let alone = (handles.iter()).find(|h| {
        let others = handles.iter().filter(|o| o.name != h.name);
        others
            .map(|o| apart(o.middle(), h.middle()))
            .fold(f64::INFINITY, f64::min)
            > 25.0
    });
    alone.unwrap_or_else(|| panic!("no dot stands apart"))
}

/// The 3D view's labels and where each shows.
fn said(ui: &mut Ui) -> Vec<(String, [f64; 4])> {
    (ui.widgets("viewport").iter())
        .map(|w| {
            let rect = std::array::from_fn(|i| w["rect"][i].as_f64().unwrap_or_default());
            (w["label"].as_str().unwrap_or_default().to_string(), rect)
        })
        .collect()
}

/// Whether the brush's cursor shows: its rings or its two lines (Draw is the brush).
fn brush_shows(labels: &[String]) -> bool {
    (labels.iter()).any(|l| {
        l == "Size ring"
            || l == "Softness ring"
            || l.starts_with("Draw  ")
            || l.starts_with("Softness ")
    })
}

/// Whether any of `rects` covers a point along `outline`'s edges.
fn covered(rects: &[[f64; 4]], outline: &[[f64; 2]]) -> bool {
    let n = outline.len();
    let mut along = (0..n).flat_map(|i| {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        (0..=10).map(move |k| {
            let f = f64::from(k) / 10.0;
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
        })
    });
    let inside =
        |p: [f64; 2], r: &[f64; 4]| r[0] <= p[0] && p[0] <= r[2] && r[1] <= p[1] && p[1] <= r[3];
    along.any(|p| rects.iter().any(|r| inside(p, r)))
}

/// A cage dot's lattice index, from its name `dot_i_j_k`.
fn index(name: &str) -> Value {
    let at: Vec<u64> = (name.split('_').skip(1))
        .filter_map(|n| n.parse().ok())
        .collect();
    json!(at)
}

#[test]
fn a_cone_drag_changes_its_one_argument_by_the_aimed_distance_one_update_a_frame() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let (deformer, handles) = shown(&mut ui);
    assert_eq!(handles.len(), 1, "{deformer}");
    let cone = find(&handles, "cone_x");
    let before = ui.state()["history.undos"].clone();

    // From the middle of the base to the tip: 0.3 gizmo units along the cone.
    let updates = drag(
        &mut ui,
        cone.middle(),
        cone.outline[0],
        Modifiers::default(),
    );
    assert_eq!(updates, 1, "four samples in one frame, one update");
    let bend = call(&mut ui);
    let degrees = number(&bend["op"]["bend"]["degrees"]);
    // The frame sits at the box's middle, so the origin held in the box is half its span from
    // the face: 2 span / (span / 2) squared, 8 / span radians a scene unit.
    let span = number(&bend["box"]["hi"][0]) - number(&bend["box"]["lo"][0]);
    let by = 0.3 * number(&deformer["unit"]);
    let want = (8.0 / span * by).to_degrees();
    assert!(
        (degrees - want).abs() < 0.01 * want.abs(),
        "{degrees} against {want}"
    );
    assert_eq!(bend["op"]["bend"]["axis"], "x", "only the angle changes");
    assert_eq!(bend["op"]["bend"]["toward"], "y");

    // Accept keeps the drag as one step named after the deformer.
    ui.press("brush > Accept");
    let state = ui.state();
    assert_eq!(state["deformer"], Value::Null);
    assert_eq!(state["history.undo"], "Bend");
    let after = number(&state["history.undos"]);
    assert_eq!(after, number(&before) + 1.0, "one step");
}

#[test]
fn a_cage_dot_drag_moves_only_that_point() {
    let mut ui = Ui::new();
    open(&mut ui, "Soft cage", 75.0);
    let (_, handles) = shown(&mut ui);
    assert_eq!(handles.len(), 27);
    let dot = alone(&handles);
    let at = dot.middle();
    drag(
        &mut ui,
        at,
        [at[0] + 30.0, at[1] - 20.0],
        Modifiers::default(),
    );
    let offsets = call(&mut ui)["op"]["lattice"]["offsets"].clone();
    let offsets = offsets.as_array().cloned().unwrap_or_default();
    assert_eq!(offsets.len(), 1, "{offsets:?}");
    assert_eq!(offsets[0]["at"], index(&dot.name));
    let by = xyz(&offsets[0]["by"]);
    assert!(by.iter().any(|c| c.abs() > 1e-3), "{by:?}");
}

#[test]
fn a_pen_rectangle_on_the_model_selects_the_dots_inside_and_a_drag_moves_them_together() {
    let mut ui = Ui::new();
    // A small gizmo, so the rectangle starts on the model clear of its handles.
    open(&mut ui, "Soft cage", 10.0);
    let (_, handles) = shown(&mut ui);
    let middles: Vec<[f64; 2]> = handles.iter().map(Handle::middle).collect();
    let [a, b] = ["dot_1_1_1", "dot_2_1_1"].map(|n| find(&handles, n).middle());
    let start = [f64::midpoint(a[0], b[0]), f64::midpoint(a[1], b[1])];
    let nearest = middles
        .iter()
        .map(|m| apart(*m, start))
        .fold(f64::INFINITY, f64::min);
    assert!(
        nearest >= 15.0,
        "the rectangle starts off every dot: {nearest}"
    );
    assert!(apart(a, start) >= 30.0, "and off the gizmo at the middle");
    let right = middles
        .iter()
        .map(|m| m[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let top = middles.iter().map(|m| m[1]).fold(f64::INFINITY, f64::min);
    let end = [right + 20.0, top - 20.0];
    let inside =
        |m: &[f64; 2]| m[0] >= start[0] && m[0] <= end[0] && m[1] <= start[1] && m[1] >= end[1];
    let picked: Vec<&Handle> = handles.iter().filter(|h| inside(&h.middle())).collect();
    assert!((2..27).contains(&picked.len()), "{}", picked.len());

    let updates = drag(&mut ui, start, end, Modifiers::default());
    assert_eq!(updates, 0, "a rectangle moves nothing");
    let gizmo = ui.ok("gizmo.describe", json!({}));
    assert_eq!(
        gizmo["placed"], false,
        "the press did not put the gizmo there"
    );

    let grabbed = picked[0].middle();
    drag(
        &mut ui,
        grabbed,
        [grabbed[0] + 20.0, grabbed[1] + 15.0],
        Modifiers::default(),
    );
    let offsets = call(&mut ui)["op"]["lattice"]["offsets"].clone();
    let offsets = offsets.as_array().cloned().unwrap_or_default();
    let mut moved: Vec<Value> = offsets.iter().map(|o| o["at"].clone()).collect();
    let mut want: Vec<Value> = picked.iter().map(|h| index(&h.name)).collect();
    moved.sort_by_key(Value::to_string);
    want.sort_by_key(Value::to_string);
    assert_eq!(moved, want);
    let by = xyz(&offsets[0]["by"]);
    assert!(by.iter().any(|c| c.abs() > 1e-3), "{by:?}");
    for o in &offsets {
        let other = xyz(&o["by"]);
        assert!(
            (0..3).all(|i| (other[i] - by[i]).abs() < 1e-5),
            "{other:?} against {by:?}"
        );
    }
}

#[test]
fn a_curve_dot_drag_moves_its_point() {
    let mut ui = Ui::new();
    open(&mut ui, "Curve", 75.0);
    let (_, handles) = shown(&mut ui);
    let names: Vec<&str> = handles.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["point_0", "point_1"]);
    let at = handles[1].middle();
    drag(
        &mut ui,
        at,
        [at[0] + 25.0, at[1] - 30.0],
        Modifiers::default(),
    );
    let curve = call(&mut ui);
    let axis = ["x", "y", "z"]
        .iter()
        .position(|a| curve["op"]["curve"]["axis"] == *a);
    let axis = axis.unwrap_or_else(|| panic!("{curve}"));
    let points = curve["op"]["curve"]["points"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(points.len(), 2, "{curve}");
    // Points are in the frame: the first stays at the axis line's low end.
    let mut low = [0.0; 3];
    low[axis] = number(&curve["box"]["lo"][axis]);
    let first = xyz(&points[0]);
    assert!(
        (0..3).all(|i| (first[i] - low[i]).abs() < 1e-5),
        "{first:?} against {low:?}"
    );
    let mut high = [0.0; 3];
    high[axis] = number(&curve["box"]["hi"][axis]);
    let second = xyz(&points[1]);
    assert!(
        (0..3).any(|i| (second[i] - high[i]).abs() > 1e-3),
        "{second:?} moved"
    );
}

#[test]
fn with_the_gizmo_turned_before_the_pick_the_box_follows_it() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
    ui.ok("ui.gizmo", json!({ "tool": true }));
    let half = std::f64::consts::PI / 8.0;
    let axes = [0.0, 0.0, libm::sin(half), libm::cos(half)];
    ui.ok("gizmo.place", json!({ "part": "sphere", "axes": axes }));
    ui.settle();
    for (pick, kind) in [("Smooth bend", "bend"), ("Rotate", "rotate")] {
        ui.press(&format!("brush > {pick}"));
        let t = call(&mut ui);
        assert!(t["op"][kind].is_object(), "{t}");
        let got = xyz(&t["frame"]["axes"]);
        let w = number(&t["frame"]["axes"][3]);
        let same = (0..3).all(|i| (got[i] - axes[i]).abs() < 1e-5) && (w - axes[3]).abs() < 1e-5;
        assert!(same, "{pick}: the gizmo's axes, {:?}", t["frame"]);
        let origin = xyz(&t["frame"]["origin"]);
        assert!(
            origin.iter().all(|c| c.abs() < 0.02),
            "the middle: {origin:?}"
        );
        if kind == "bend" {
            // Tight around the sphere in the turned axes, not the part's box seen turned
            // (0.7 times the square root of 2 on X).
            let hi = number(&t["box"]["hi"][0]);
            assert!((hi - 0.7).abs() < 0.02, "{}", t["box"]);
        }
        ui.press("brush > Cancel");
    }
}

/// Off every handle the brush's cursor shows; over a lit cone, hovered or dragged, only the
/// pen's own ring does (`DF7c-notes`, section 1).
#[test]
fn over_a_lit_cone_the_brush_rings_and_lines_give_way_hovering_or_dragging() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let r = ui.rect("viewport");
    ui.pen(r[0] + (r[2] - r[0]) * 0.15, f64::midpoint(r[1], r[3]));
    let labels = ui.labels("viewport");
    assert!(brush_shows(&labels), "off every handle: {labels:?}");
    let (_, handles) = shown(&mut ui);
    let cone = find(&handles, "cone_x");
    let ([x, y], tip) = (cone.middle(), cone.outline[0]);
    ui.pen(x, y);
    let labels = ui.labels("viewport");
    assert!(!brush_shows(&labels), "hovering the cone: {labels:?}");
    ui.touch(x, y, true);
    ui.touch(tip[0], tip[1], true);
    let labels = ui.labels("viewport");
    assert!(!brush_shows(&labels), "dragging it: {labels:?}");
    ui.touch(tip[0], tip[1], false);
}

/// The same over the gizmo's own arrow, which says nothing of its own; a held key's line shows
/// clear of it.
#[test]
fn over_a_lit_gizmo_arrow_too_and_a_held_key_says_its_line_clear_of_it() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let (d, r) = described(&mut ui);
    let own = listed(&d["handles"], r);
    let arrow = find(&own, "move_y");
    let [a, b] = [arrow.outline[0], arrow.outline[1]];
    ui.pen(a[0] + (b[0] - a[0]) * 0.7, a[1] + (b[1] - a[1]) * 0.7);
    let labels = ui.labels("viewport");
    assert!(!brush_shows(&labels), "hovering the arrow: {labels:?}");
    assert!(
        !labels.iter().any(|l| l.contains("Shift snaps")),
        "no words: {labels:?}"
    );
    ui.input(&[], held("Ctrl"));
    let lines = said(&mut ui);
    let copy = lines.iter().find(|(l, _)| l == "Ctrl: Copy");
    let copy = copy.unwrap_or_else(|| panic!("Ctrl's line: {lines:?}"));
    assert!(
        !covered(&[copy.1], &arrow.outline),
        "{copy:?} clear of {:?}",
        arrow.outline
    );
    ui.input(&[], Modifiers::default());
}

/// A held key's line says what the key does to the deformer handle under the pen, which a
/// press grabs before the gizmo's own: over Smooth bend's angle cone Shift snaps its turn to
/// the gizmo's step, as the cone's own words say, never a move the gizmo's arrow would snap;
/// Ctrl copies nothing there. Over a cage's dot, Shift adds it or takes it out and snaps the
/// move, and Ctrl takes it out, as a press does (`DF7c-notes`, section 4).
#[test]
fn over_a_deformer_handle_a_held_key_says_what_it_does_to_that_handle() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let says = |ui: &mut Ui, at: [f64; 2], lines: [(&str, &str); 2]| {
        ui.pen(at[0], at[1]);
        for (key, line) in lines {
            ui.input(&[], held(key));
            let (cursor, status) = (ui.labels("viewport"), ui.labels("status"));
            assert!(cursor.iter().any(|l| l == line), "{line}: {cursor:?}");
            assert!(status.iter().any(|l| l == line), "{line}: {status:?}");
        }
        ui.input(&[], Modifiers::default());
    };
    let (_, handles) = shown(&mut ui);
    let cone = find(&handles, "cone_x").middle();
    let shift = ("Shift", "Shift: Snap turns to 5°");
    says(
        &mut ui,
        cone,
        [shift, ("Ctrl", "Ctrl: Does nothing to this handle")],
    );
    ui.press("brush > Cancel");
    ui.press("brush > Soft cage");
    let (_, handles) = shown(&mut ui);
    let dot = alone(&handles).middle();
    let shift = (
        "Shift",
        "Shift: Add or take out the dot, snap moves to 0.01",
    );
    says(
        &mut ui,
        dot,
        [shift, ("Ctrl", "Ctrl: Take the dot out of the selection")],
    );
}

/// A lit cone says the row it drags as the tray shows it, then the row's hint and its key,
/// clear of the cone; through its drag the value counts. A lit dot says the points row and the
/// dots' hint (`DF7c-notes`, section 2).
#[test]
fn a_lit_handle_says_what_it_drags_clear_of_it_and_counts_through_a_drag() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let (_, handles) = shown(&mut ui);
    let cone = find(&handles, "cone_x");
    let ([x, y], tip) = (cone.middle(), cone.outline[0]);
    ui.pen(x, y);
    let lines = said(&mut ui);
    let words = ["Angle  +0", "The angle, in degrees. Shift snaps to 5°."];
    let found: Vec<[f64; 4]> = (lines.iter())
        .filter(|(l, _)| words.contains(&l.as_str()))
        .map(|w| w.1)
        .collect();
    assert_eq!(found.len(), 2, "{words:?} in {lines:?}");
    assert!(
        !covered(&found, &cone.outline),
        "{found:?} clear of {:?}",
        cone.outline
    );
    ui.touch(x, y, true);
    ui.touch(tip[0], tip[1], true);
    let degrees = number(&call(&mut ui)["op"]["bend"]["degrees"]);
    let want = format!("Angle  {degrees:+.0}");
    let labels = ui.labels("viewport");
    assert!(
        degrees.abs() > 1.0 && labels.contains(&want),
        "{want}: {labels:?}"
    );
    ui.touch(tip[0], tip[1], false);
    ui.press("brush > Cancel");

    ui.press("brush > Soft cage");
    let (_, handles) = shown(&mut ui);
    let [x, y] = alone(&handles).middle();
    ui.pen(x, y);
    let labels = ui.labels("viewport");
    let dots = "Drag a dot; tap to select, Shift adds, Ctrl removes; drag on the model to select \
                several.";
    for w in ["27 points, none moved", dots] {
        assert!(labels.iter().any(|l| l == w), "{w}: {labels:?}");
    }
}

/// One Escape does one thing (`DF7c-notes`, section 3): with a slider's fine handle open in the
/// deformer's rows, the first shuts it and the deformer stays open; the next cancels it.
#[test]
fn with_a_fine_handle_open_escape_shuts_it_first_and_then_cancels_the_deformer() {
    let mut ui = Ui::new();
    open(&mut ui, "Smooth bend", 75.0);
    let r = ui.rect("brush > Angle");
    let [x, y] = [r[0] + 20.0, f64::midpoint(r[1], r[3])];
    // Past egui's triple click after the press on Smooth bend, so the two taps are a double.
    ui.wait(0.7);
    ui.tap_at(x, y);
    ui.tap_at(x, y);
    let none = Modifiers::default();
    ui.input(&[Input::Text("30".into())], none);
    ui.tap("Return", none);
    let labels = ui.labels("brush");
    let open = |l: &str| labels.iter().any(|o| o == l);
    assert!(open("Fine Angle") && !open("Type Angle"), "{labels:?}");
    assert_eq!(number(&call(&mut ui)["op"]["bend"]["degrees"]), 30.0);

    ui.tap("Escape", none);
    let labels = ui.labels("brush");
    assert!(!labels.iter().any(|l| l == "Fine Angle"), "{labels:?}");
    assert!(!ui.state()["deformer"].is_null(), "the deformer stays open");
    ui.tap("Escape", none);
    assert_eq!(ui.state()["deformer"], Value::Null, "the next cancels it");
}
