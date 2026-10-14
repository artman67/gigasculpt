//! The geometry, mask and brush keys and the Ctrl gestures on empty canvas (decision 0017 as
//! amended, items 10 and 11), through the real window host and engine as the window drives
//! them: each new key asks for its command, no two default keys clash, and with Ctrl alone held
//! a click on empty canvas inverts the mask, a drag there clears it or, with no mask, updates a
//! part in Voxel mode at the cell size it keeps (decision 0002), each as a logged call
//! that replays, while on the model Ctrl still strokes with Mask Pen. In an empty scene the
//! gesture runs nothing and the status line says why.

use gs_platform::{Input, Modifiers};
use gs_protocol::{PenSample, ScriptLine, Step};
use gs_testkit::Session;
use gs_ui::{Chord, Keymap};
use serde_json::{Value, json};

use crate::region;
use crate::rig::{Window, log_path, pen_at, with_sphere};

const CTRL: Modifiers = Modifiers {
    ctrl: true,
    shift: false,
    alt: false,
    logo: false,
};

/// The commands one key (or one sequence of keys, each `held`) asks the window for, not run,
/// other than the held keys' brush slot.
fn asked(w: &mut Window, keys: &[&str], held: Modifiers) -> Vec<Step> {
    let mut studio = w.host.lock();
    let mut steps = Vec::new();
    for name in keys {
        for down in [true, false] {
            let key = Input::Key {
                key: (*name).into(),
                down,
                repeat: false,
                modifiers: held,
            };
            studio.input(&[key], held);
            studio.frame().unwrap_or_else(|e| panic!("{e}"));
            steps.extend(studio.shell.take_commands());
        }
    }
    steps.retain(|s| s.command != "brush.slot");
    steps
}

#[test]
fn each_new_key_asks_for_its_command() {
    let mut w = Window::new(None);
    let none = Modifiers::default();
    let with = |ctrl, shift, alt| Modifiers {
        ctrl,
        shift,
        alt,
        logo: false,
    };
    let keys: [(&[&str], Modifiers, &str, Value); 9] = [
        (
            &["D"],
            with(true, false, false),
            "level.subdivide",
            json!({}),
        ),
        (&["D"], none, "level.step", json!({ "by": 1 })),
        (
            &["D"],
            with(false, true, false),
            "level.step",
            json!({ "by": -1 }),
        ),
        (&["R"], CTRL, "mesh.voxel_remesh", json!({})),
        (&["\\"], none, "live_detail.set", json!({ "toggle": true })),
        (&["I"], CTRL, "mask.invert", json!({ "scope": "all" })),
        (&["A"], CTRL, "mask.set", json!({ "value": 1 })),
        (
            &["M"],
            with(false, false, true),
            "mask.set",
            json!({ "value": 0, "scope": "all" }),
        ),
        (&["9"], none, "file.quick_save", json!({})),
    ];
    for (key, held, command, args) in keys {
        let steps = asked(&mut w, key, held);
        let got: Vec<(&str, &Value)> = (steps.iter())
            .map(|s| (s.command.as_str(), &s.args))
            .collect();
        assert_eq!(got, [(command, &args)], "{key:?} with {held:?}");
    }
    for (letters, brush) in [
        (["B", "S", "M"], "Smooth"),
        (["B", "I", "N"], "Inflate"),
        (["B", "P", "I"], "Pinch"),
        (["B", "D", "S"], "Sharp Crease"),
        (["B", "S", "H"], "Pull Out"),
        (["B", "T", "D"], "Trim"),
        (["B", "H", "P"], "Flat Polish"),
    ] {
        let steps = asked(&mut w, &letters, none);
        let got: Vec<(&str, &Value)> = (steps.iter())
            .map(|s| (s.command.as_str(), &s.args))
            .collect();
        assert_eq!(
            got,
            [("brush.set", &json!({ "brush": brush }))],
            "{letters:?}"
        );
    }
}

#[test]
fn no_two_default_keys_clash() {
    let keymap = Keymap::default();
    let parsed: Vec<(String, Vec<Chord>)> = (keymap.bindings.iter())
        .map(|b| {
            let chords = (b.key.split_whitespace())
                .map(|c| Chord::parse(c).unwrap_or_else(|e| panic!("{}: {e}", b.key)))
                .collect();
            (b.key.clone(), chords)
        })
        .collect();
    for (i, (a, first)) in parsed.iter().enumerate() {
        for (b, second) in &parsed[i + 1..] {
            assert!(
                !first.starts_with(second) && !second.starts_with(first),
                "'{a}' and '{b}' clash: one is the other or starts it"
            );
        }
    }
}

/// `main`'s camera as JSON text (equal text is equal bits).
fn camera(w: &mut Window) -> String {
    w.ok("view.get", json!({ "view": "main" }))["camera"].to_string()
}

/// One pen sample, then one frame of the window with Ctrl held and the commands it asks for.
fn frame(w: &mut Window, s: PenSample) {
    w.pen(s);
    w.input(&[], CTRL);
}

/// A drag of the tip from `from` to `to` (a click when they are the same) with Ctrl held, one
/// frame per sample, then lifted.
fn ctrl_drag(w: &mut Window, from: [f32; 2], to: [f32; 2]) {
    drag(w, CTRL, from, to);
}

/// A drag from `from` to `to` with `held` held, over several frames.
fn drag(w: &mut Window, held: Modifiers, from: [f32; 2], to: [f32; 2]) {
    w.input(&[], held);
    frame(w, pen_at(0, from[0], from[1], 0.0));
    for k in 0..5u8 {
        let f = f32::from(k) / 4.0;
        let [x, y] = [0, 1].map(|i| from[i] + (to[i] - from[i]) * f);
        frame(w, pen_at(4 * u64::from(k) + 4, x, y, 0.5));
    }
    frame(w, pen_at(24, to[0], to[1], 0.0));
}

/// The calls the log holds after its first `from` lines, other than pen input and the held
/// keys' brush slot.
pub(crate) fn ran_since(w: &Window, from: usize) -> Vec<(String, Value)> {
    let calls = w.logged().split_off(from).into_iter();
    calls
        .filter(|l| l.0 != "pen.input" && l.0 != "brush.slot")
        .collect()
}

/// The names of [`ran_since`]'s calls.
fn names_since(w: &Window, from: usize) -> Vec<String> {
    ran_since(w, from).into_iter().map(|l| l.0).collect()
}

/// The status line's problem.
pub(crate) fn problem(w: &Window) -> String {
    let studio = w.host.lock();
    studio.shell.status.problem.clone().unwrap_or_default()
}

#[test]
fn ctrl_on_empty_canvas_works_the_mask_and_remeshes_and_on_the_model_masks() {
    let log = log_path("background");
    let (mut w, rect) = with_sphere(Some(log.clone()));
    let corner = [rect[0] + 30.0, rect[3] - 30.0];
    let across = [corner[0] + 80.0, corner[1]];
    let view = camera(&mut w);

    // A click, its samples in one call as an agent may send them: the engine says it fell on
    // empty canvas after the pen has lifted.
    let (hash, start) = (w.hash(), w.logged().len());
    w.input(&[], CTRL);
    let [x, y] = corner;
    let click = [
        pen_at(0, x, y, 0.0),
        pen_at(4, x, y, 0.5),
        pen_at(8, x, y, 0.0),
    ];
    w.call_as(
        gs_protocol::Role::Human,
        "pen.input",
        json!({ "samples": click }),
    );
    w.input(&[], CTRL);
    let ran = ran_since(&w, start);
    let ran: Vec<(&str, &Value)> = (ran.iter())
        .map(|(c, a)| (c.as_str(), &a["scope"]))
        .collect();
    assert_eq!(
        ran,
        [("mask.invert", &json!("all"))],
        "a Ctrl click inverts the whole mask, hidden points too"
    );
    let inverted = w.hash();
    assert_ne!(inverted, hash, "the sphere is masked");

    // A drag, a frame per sample: the mask is cleared.
    let start = w.logged().len();
    ctrl_drag(&mut w, corner, across);
    let ran = ran_since(&w, start);
    assert_eq!(ran.len(), 1, "one call: {ran:?}");
    let (command, args) = &ran[0];
    assert_eq!(
        (command.as_str(), &args["value"], &args["scope"]),
        ("mask.set", &json!(0.0), &json!("all")),
        "a Ctrl drag clears it"
    );
    assert_ne!(w.hash(), inverted, "the mask is cleared");

    // No mask, and the sphere in Voxel mode: a Ctrl drag updates it at its cell size. With the
    // switch off the engine refuses the update (`voxel_mode.rs`).
    w.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    w.ok("mesh.voxel_mode", json!({ "on": true }));
    let (hash, start) = (w.hash(), w.logged().len());
    ctrl_drag(&mut w, corner, across);
    assert_eq!(names_since(&w, start), ["mesh.voxel_update"], "it remeshes");
    assert_ne!(w.hash(), hash, "at the kept cell size");
    assert_eq!(camera(&mut w), view, "no gesture turned the view");

    // On the model, Ctrl strokes with Mask Pen, as before.
    let start = w.logged().len();
    let [cx, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    ctrl_drag(&mut w, [cx - 40.0, cy], [cx + 40.0, cy]);
    let ran = ran_since(&w, start);
    assert_eq!(ran.len(), 1, "one stroke: {ran:?}");
    assert_eq!(
        (ran[0].0.as_str(), &ran[0].1["brush"]),
        ("brush.stroke", &json!("Mask Pen")),
        "on the model Ctrl masks"
    );
    assert_eq!(camera(&mut w), view, "and the view stayed");

    // A locked view does not turn on empty canvas, but Ctrl still works the mask there.
    w.ok("view.lock", json!({ "locked": true }));
    let start = w.logged().len();
    ctrl_drag(&mut w, corner, corner);
    assert_eq!(
        names_since(&w, start),
        ["mask.invert"],
        "a Ctrl click on a locked view inverts the mask"
    );

    // The log replays to the same scene without the window.
    let scene = w.hash();
    let mut replay = Session::new();
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("{e}"));
    for line in text.lines() {
        if let Ok(ScriptLine::Record(r)) = ScriptLine::parse(line) {
            replay.request(r.request().unwrap_or_else(|e| panic!("{e}")));
        }
    }
    let replayed = replay.query("mesh.hash", json!({}))["hash"].clone();
    assert_eq!(replayed, json!(scene), "the log replays to the same scene");
}

#[test]
fn a_ctrl_gesture_in_an_empty_scene_runs_nothing_and_says_why() {
    let mut w = Window::new(Some(log_path("background-empty")));
    let rect = w.view_rect();
    let [x, y] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    // A click, then a drag.
    for to in [[x, y], [x + 80.0, y]] {
        w.host.lock().shell.status.problem = None;
        let start = w.logged().len();
        ctrl_drag(&mut w, [x, y], to);
        let ran = names_since(&w, start);
        assert!(ran.is_empty(), "nothing runs: {ran:?}");
        let said = problem(&w);
        assert!(said.contains("scene has no parts"), "{to:?}: {said}");
    }
}

/// The gesture's problem is said as a person's sentence, a capital first, as every problem on
/// the status line is (`Status::says`), and the next stroke that lands clears it.
#[test]
fn a_gestures_problem_reads_as_a_sentence_and_the_next_stroke_clears_it() {
    let mut w = Window::new(Some(log_path("background-said")));
    let rect = w.view_rect();
    let [x, y] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    ctrl_drag(&mut w, [x, y], [x + 80.0, y]);
    let said = problem(&w);
    assert!(said.starts_with("The scene has no parts"), "{said}");
    let sphere = json!({ "subdivisions": 4 });
    w.call_as(gs_protocol::Role::Agent, "primitive.sphere", sphere);
    assert_eq!(problem(&w), said, "an agent's call leaves it");
    let before = w.hash();
    drag(&mut w, Modifiers::default(), [x, y], [x + 20.0, y]);
    assert_ne!(w.hash(), before, "the stroke landed");
    assert_eq!(problem(&w), "", "and cleared the line");
}
