//! Header > Deform (decision 0041, amended 2026-10-12, step `DF7a`, moved from the scene tray by
//! `D1`, decision 0060 section 18), a dropdown panel: a slider pulls the whole
//! part live through one `gizmo.deformer` session, lands the pull as one undo step and one
//! `transform.apply` line and springs back to 0; a typed value is that line at once; the shared
//! X, Y, Z row sends a call per lit axis in one batch, or `axes`; Fit to unit size, Flip mirror
//! and Repeat on other Parts are one step each; with the Gizmo tool on, a pull asks for the
//! gizmo again only once it lands. The Smoothing group's sliders (step `DF-smooth`) do the same
//! through a `smooth` session and one `mesh.smooth` line. The window here sends what it asks for
//! as the window does, a `cmd.batch` step as a batch, and its log is a file, so each line's
//! arguments can be read.

use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_platform::{Input, Modifiers};
use gs_protocol::{LogRecord, Outcome, Role, ScriptLine, Step};
use gs_testkit::session::Session;
use gs_ui::app::request;
use gs_ui::{Host, Keymap, Layout, LayoutFile, Shell, Studio};
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{SIZE, Ui, key, touch_sample};

/// The window with Header > Deform open, what it sent, and its log file.
struct Tray {
    ui: Ui,
    sent: Vec<Step>,
    log: PathBuf,
}

impl Tray {
    /// A sphere of radius 0.7 at the origin, named `sphere`, and the header's Deform open.
    fn new(name: &str) -> Self {
        let log = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("deform-{}-{name}.log", std::process::id()));
        std::fs::remove_file(&log).ok();
        let config = EngineConfig {
            log_path: Some(log.clone()),
            ..EngineConfig::default()
        };
        let mut session = Session::with(config, Role::Human);
        let layout = LayoutFile::in_memory(Layout::default());
        let shell = Shell::new(session.engine.catalog().clone(), Keymap::default(), layout);
        let host = Host::new(Studio::new(shell, None, SIZE, 1.0));
        session.engine.attach_window(Box::new(host.clone()));
        let mut tray = Self {
            ui: Ui { session, host },
            sent: Vec::new(),
            log,
        };
        tray.ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
        crate::header::open(&mut tray.ui, "Deform");
        tray
    }

    /// Draws frames and sends what the window asked for, as the window sends it, until it asks
    /// for nothing more.
    fn run(&mut self) {
        for _ in 0..4 {
            let steps = {
                let mut studio = self.ui.host.lock();
                studio.frame().unwrap_or_else(|e| panic!("{e}"));
                studio.shell.take_commands()
            };
            if steps.is_empty() {
                return;
            }
            for step in steps {
                self.send(step);
            }
        }
    }

    fn send(&mut self, step: Step) {
        self.sent.push(step.clone());
        let response = self.ui.session.request(request(step.clone()));
        if let Outcome::Error(e) = response.outcome {
            self.ui.host.lock().shell.refused(Some(step), &e);
        }
    }

    /// The pen at `at`, touching or not, and what the window then asks for.
    fn touch(&mut self, [x, y]: [f64; 2], down: bool) {
        self.ui.ok("pen.input", touch_sample(x, y, down));
        self.run();
    }

    /// A tap of the pen on the panel's widget `name`.
    fn press(&mut self, name: &str) {
        let at = middle(self.ui.rect(&format!("menu > {name}")));
        for down in [false, true, false] {
            self.touch(at, down);
        }
    }

    /// Where a pull of slider `name` starts: on its rail, near its left end.
    fn rail(&mut self, name: &str) -> [f64; 2] {
        let r = self.ui.rect(&format!("menu > {name}"));
        [r[0] + 20.0, middle(r)[1]]
    }

    /// A pull of slider `name`: pressed on its rail, moved `by` each in turn (pixels from the
    /// press), then let go.
    fn pull(&mut self, name: &str, by: &[f64]) {
        let [x, y] = self.rail(name);
        self.touch([x, y], false);
        self.touch([x, y], true);
        for dx in by {
            self.touch([x + dx, y], true);
        }
        let end = by.last().copied().unwrap_or_default();
        self.touch([x + end, y], false);
    }

    /// `text` typed into slider `name`'s field, which a tap on its value opens, and Enter.
    fn type_value(&mut self, name: &str, text: &str) {
        let r = self.ui.rect(&format!("menu > {name}"));
        for down in [false, true, false] {
            self.touch([r[2] - 4.0, middle(r)[1]], down);
        }
        self.keys(&[Input::Text(text.into())]);
        self.keys(&[key("Return", true, Modifiers::default())]);
        self.keys(&[key("Return", false, Modifiers::default())]);
    }

    fn keys(&mut self, inputs: &[Input]) {
        self.ui.host.lock().input(inputs, Modifiers::default());
        self.run();
    }

    /// Every line the log holds but the pen's samples, oldest first.
    fn lines(&self) -> Vec<LogRecord> {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        (text.lines())
            .filter_map(|l| match ScriptLine::parse(l) {
                Ok(ScriptLine::Record(r)) if r.command != "pen.input" => Some(r),
                _ => None,
            })
            .collect()
    }

    /// The lines after the first `from`.
    fn since(&self, from: usize) -> Vec<LogRecord> {
        self.lines().split_off(from)
    }

    fn undos(&mut self) -> Value {
        self.ui.state()["history.undos"].clone()
    }

    /// The `gizmo.deformer` steps sent, with their phases.
    fn sessions(&self) -> Vec<(String, Value)> {
        (self.sent.iter())
            .filter(|s| s.command == "gizmo.deformer")
            .map(|s| (text(&s.args["phase"]), s.args.clone()))
            .collect()
    }

    /// The panel's widget `label`, as `ui.describe` reports it.
    fn widget(&mut self, label: &str) -> Value {
        let widgets = self.ui.widgets("menu");
        let found = widgets.iter().find(|w| w["label"] == label).cloned();
        found.unwrap_or_else(|| panic!("no {label} in Deform: {widgets:?}"))
    }

    /// Every point of part `part`, as bits.
    fn bits(&mut self, part: &str) -> Vec<[u32; 3]> {
        let mesh = self.ui.session.mesh(part);
        (mesh.positions.iter())
            .map(|p| p.map(f32::to_bits))
            .collect()
    }

    /// The largest side of `part`'s box as `mesh.bounds` measures it.
    fn largest(&mut self, part: &str) -> f64 {
        let bounds = self.ui.ok("mesh.bounds", json!({ "part": part }));
        (0..3)
            .map(|i| bounds["size"][i].as_f64().unwrap_or_default())
            .fold(0.0, f64::max)
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// The calls of a batch line, each as `(command, args)`.
fn calls(line: &LogRecord) -> Vec<(String, Value)> {
    assert_eq!(line.command, "cmd.batch", "{line:?}");
    let commands = line.args["commands"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    (commands.iter())
        .map(|c| (text(&c["command"]), c["args"].clone()))
        .collect()
}

#[test]
fn a_bend_pull_opens_updates_and_accepts_one_step_named_bend_and_springs_back() {
    let mut tray = Tray::new("bend");
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.sent.clear();
    tray.pull("Smooth bend", &[20.0, 40.0, 60.0]);

    let sessions = tray.sessions();
    let phases: Vec<&str> = sessions.iter().map(|(p, _)| p.as_str()).collect();
    assert!(phases.len() >= 3, "{phases:?}");
    assert_eq!(phases[0], "open", "the press opens: {phases:?}");
    assert_eq!(
        phases[phases.len() - 1],
        "accept",
        "the release accepts: {phases:?}"
    );
    let between = &phases[1..phases.len() - 1];
    assert!(between.iter().all(|p| *p == "update"), "{phases:?}");
    let degrees = |args: &Value| number(&args["transforms"][0]["op"]["bend"]["degrees"]);
    assert_eq!(degrees(&sessions[0].1), 0.0, "the press opens at 0");
    let last = degrees(&sessions[sessions.len() - 2].1);
    assert!(last > 0.0, "a pull to the right bends: {last}");

    let state = tray.ui.state();
    assert_eq!(state["history.undos"], json!(number(&undos) as u64 + 1));
    assert_eq!(state["history.undo"], "Bend", "{state}");
    assert_eq!(state["deformer"], Value::Null, "the pull is accepted");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "one line: {lines:?}");
    assert_eq!(lines[0].command, "transform.apply");
    let bend = &lines[0].args["op"]["bend"];
    assert_eq!(
        (text(&bend["axis"]), text(&bend["toward"])),
        ("x".into(), "y".into())
    );
    assert_eq!(
        number(&bend["degrees"]) as f32,
        last as f32,
        "the line is the last update's, as the engine holds it"
    );
    assert!(lines[0].args["box"].is_object(), "{:?}", lines[0].args);

    let shown = tray.widget("Smooth bend");
    let at = number(&shown["command"]["args"]["op"]["bend"]["degrees"]);
    assert_eq!(at, 0.0, "the slider reads 0 after: {shown}");
}

#[test]
fn deform_opens_from_the_header_and_stays_open_after_a_pull_of_one_step_until_escape() {
    let mut tray = Tray::new("header");
    let titles = crate::header::titles_and_ends(&mut tray.ui).0;
    let titles = crate::header::left_to_right(&titles);
    let at = |t: &str| titles.iter().position(|x| x == t);
    assert_eq!(at("Deform"), at("Groups").map(|i| i + 1), "{titles:?}");
    assert_eq!(at("View"), at("Deform").map(|i| i + 1), "{titles:?}");

    let undos = tray.undos();
    tray.pull("Twist", &[30.0, 60.0]);
    assert_eq!(tray.undos(), json!(number(&undos) as u64 + 1), "one step");
    let shown = crate::header::shown_menu(&mut tray.ui);
    assert_eq!(shown.as_deref(), Some("Deform"), "open after the pull");

    tray.keys(&[key("Escape", true, Modifiers::default())]);
    tray.keys(&[key("Escape", false, Modifiers::default())]);
    assert_eq!(
        crate::header::shown_menu(&mut tray.ui),
        None,
        "Escape shuts it"
    );
}

#[test]
fn with_x_and_z_lit_a_twist_pull_is_one_batch_and_an_inflate_pull_sends_axes() {
    let mut tray = Tray::new("axes");
    tray.press("Z");
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.pull("Twist", &[30.0, 60.0]);
    assert_eq!(tray.undos(), json!(number(&undos) as u64 + 1), "one step");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let axes: Vec<(String, String)> = (calls(&lines[0]).iter())
        .map(|(c, a)| (c.clone(), text(&a["op"]["twist"]["axis"])))
        .collect();
    let apply = || "transform.apply".to_string();
    assert_eq!(axes, [(apply(), "x".into()), (apply(), "z".into())]);

    let from = tray.lines().len();
    tray.pull("Inflate", &[30.0, 60.0]);
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].command, "transform.apply", "{lines:?}");
    let kept = &lines[0].args["axes"];
    assert_eq!(kept, &json!({ "x": true, "y": false, "z": true }));
}

#[test]
fn escape_during_a_pull_leaves_the_part_bit_for_bit_and_no_step() {
    escape_mid_pull("Smooth bend", "bend");
}

/// Escape while the pen pulls `slider`, whose session is a `kind`: every point as it was, no
/// step, no line, nothing sent after the cancel, and the panel still open (D1).
fn escape_mid_pull(slider: &str, kind: &str) {
    let mut tray = Tray::new(&format!("escape-{kind}"));
    let (bits, undos, from) = (tray.bits("sphere"), tray.undos(), tray.lines().len());
    let [x, y] = tray.rail(slider);
    tray.touch([x, y], false);
    tray.touch([x, y], true);
    tray.touch([x + 60.0, y], true);
    assert_eq!(tray.ui.state()["deformer"], kind, "the pull is live");
    assert_ne!(tray.bits("sphere"), bits, "the pull shows on the part");

    tray.keys(&[key("Escape", true, Modifiers::default())]);
    tray.keys(&[key("Escape", false, Modifiers::default())]);
    tray.touch([x + 90.0, y], true);
    tray.touch([x + 90.0, y], false);

    assert_eq!(tray.bits("sphere"), bits, "every point as it was");
    assert_eq!(tray.undos(), undos, "no step");
    assert!(tray.since(from).is_empty(), "no line");
    assert_eq!(tray.ui.state()["deformer"], Value::Null, "the pull is over");
    let shown = crate::header::shown_menu(&mut tray.ui);
    assert_eq!(shown.as_deref(), Some("Deform"), "the panel stays open");
    let sessions = tray.sessions();
    let phases: Vec<&str> = sessions.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        phases.last(),
        Some(&"cancel"),
        "nothing after Escape: {phases:?}"
    );
}

#[test]
fn a_typed_value_is_one_step_with_the_line_a_pull_of_it_leaves() {
    let degrees = typed_as_pulled("Twist", |args| number(&args["op"]["twist"]["degrees"]));
    assert_eq!(
        degrees.abs(),
        360.0,
        "the rail's end is 100, 180 degrees at each end"
    );
}

/// A pull of `slider` to its rail's end, undone, then the end's value typed: one step each with
/// the same line, no session for the typed value, and the slider reading 0 after. Gives the
/// pulled line's value as `at` reads it from a call's arguments.
fn typed_as_pulled(slider: &str, at: fn(&Value) -> f64) -> f64 {
    let mut tray = Tray::new(&format!("typed-{slider}"));
    let [x, _] = tray.rail(slider);
    // Past the rail's end on whichever side the window has room: the end exactly.
    let (to, typed) = if x > 900.0 {
        (5.0, "-100")
    } else {
        (f64::from(SIZE[0]) - 5.0, "100")
    };
    let from = tray.lines().len();
    tray.pull(slider, &[to - x]);
    let pulled = tray.since(from);
    assert_eq!(pulled.len(), 1, "{pulled:?}");

    tray.ui.ok("history.undo", json!({}));
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.sent.clear();
    tray.type_value(slider, typed);
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].command, pulled[0].command, "the same command");
    assert_eq!(lines[0].args, pulled[0].args, "the same line");
    assert_eq!(tray.undos(), json!(number(&undos) as u64 + 1), "one step");
    assert!(tray.sessions().is_empty(), "no session for a typed value");
    let shown = tray.widget(slider);
    let reads = at(&shown["command"]["args"]);
    assert_eq!(reads, 0.0, "the slider reads 0 after: {shown}");
    at(&pulled[0].args)
}

#[test]
fn fit_to_unit_size_scales_the_largest_side_to_two_as_mesh_bounds_says() {
    let mut tray = Tray::new("fit");
    let largest = tray.largest("sphere");
    let from = tray.lines().len();
    tray.press("Fit to unit size");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].command, "transform.apply");
    let scale = lines[0].args["op"]["scale"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(scale.len(), 3, "{:?}", lines[0].args);
    for k in &scale {
        let k = number(k);
        assert!(
            (k - 2.0 / largest).abs() < 1e-5,
            "{k} against 2 / {largest}"
        );
    }
    let after = tray.largest("sphere");
    assert!(
        (after - 2.0).abs() < 1e-4,
        "the largest side is now {after}"
    );
}

#[test]
fn offset_100_with_only_y_lit_moves_the_part_half_its_height_along_y_alone() {
    let mut tray = Tray::new("offset");
    tray.press("X");
    tray.press("Y");
    let hash = tray.ui.session.hash();
    tray.type_value("Offset", "0");
    assert_eq!(tray.ui.session.hash(), hash, "0 changes nothing");

    let before = tray.ui.ok("mesh.bounds", json!({ "part": "sphere" }));
    tray.type_value("Offset", "100");
    let after = tray.ui.ok("mesh.bounds", json!({ "part": "sphere" }));
    let at = |b: &Value, field: &str, i: usize| number(&b[field][i]);
    let moved = at(&after, "center", 1) - at(&before, "center", 1);
    let half = at(&before, "size", 1) / 2.0;
    assert!((moved - half).abs() < 1e-6, "{moved} against {half}");
    for i in [0, 2] {
        assert_eq!(at(&after, "center", i), at(&before, "center", i), "only Y");
    }
    assert_eq!(after["size"], before["size"], "every point moves alike");
}

#[test]
fn flip_mirror_with_two_axes_lit_is_one_step() {
    let mut tray = Tray::new("flip");
    tray.press("Y");
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.press("Flip mirror");
    assert_eq!(tray.undos(), json!(number(&undos) as u64 + 1), "one step");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let flips: Vec<(String, String)> = (calls(&lines[0]).iter())
        .map(|(c, a)| (c.clone(), text(&a["axis"])))
        .collect();
    let flip = || "mesh.flip".to_string();
    assert_eq!(flips, [(flip(), "x".into()), (flip(), "y".into())]);
}

#[test]
fn repeat_on_other_parts_runs_the_last_call_on_each_shown_part_in_one_step_and_its_mask_switch() {
    let mut tray = Tray::new("repeat");
    for (name, center) in [("b", [3, 0, 0]), ("c", [0, 3, 0]), ("hidden", [0, 0, 3])] {
        let sphere = json!({ "name": name, "radius": 0.5, "center": center });
        tray.ui.ok("primitive.sphere", sphere);
    }
    tray.ui.ok(
        "part.set_visible",
        json!({ "parts": ["hidden"], "visible": false }),
    );
    tray.ui.ok("part.select", json!({ "part": "sphere" }));
    tray.type_value("Rotate", "30");
    let (hidden, before) = (tray.bits("hidden"), tray.ui.session.hash());
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.press("Repeat on other Parts");

    assert_eq!(tray.undos(), json!(number(&undos) as u64 + 1), "one step");
    assert_eq!(tray.bits("hidden"), hidden, "a hidden part is untouched");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let repeated = calls(&lines[0]);
    let mut origins: Vec<[i64; 3]> = (repeated.iter())
        .map(|(command, args)| {
            assert_eq!(command, "transform.apply");
            assert_eq!(number(&args["op"]["rotate"]["degrees"]), 30.0, "{args}");
            assert_ne!(
                args["mask"],
                json!(false),
                "the mask holds by default: {args}"
            );
            let origin = &args["frame"]["origin"];
            std::array::from_fn(|i| number(&origin[i]).round() as i64)
        })
        .collect();
    origins.sort_unstable();
    assert_eq!(origins, [[0, 3, 0], [3, 0, 0]], "each about its own middle");
    tray.ui.ok("history.undo", json!({}));
    assert_eq!(tray.ui.session.hash(), before, "one undo restores them all");

    for part in ["b", "c"] {
        tray.ui
            .ok("mask.set", json!({ "part": part, "value": 1.0 }));
    }
    tray.ui.ok("part.select", json!({ "part": "sphere" }));
    let masked = tray.ui.session.hash();
    tray.press("Repeat on other Parts");
    assert_eq!(
        tray.ui.session.hash(),
        masked,
        "Mask on: masked points stay"
    );

    tray.press("Mask");
    let from = tray.lines().len();
    tray.press("Repeat on other Parts");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    for (_, args) in calls(&lines[0]) {
        assert_eq!(args["mask"], json!(false), "Mask off: {args}");
    }
    assert_ne!(
        tray.ui.session.hash(),
        masked,
        "Mask off: masked parts turn"
    );
}

#[test]
fn pen_moves_over_several_frames_send_one_update_with_the_last_value() {
    let mut tray = Tray::new("coalesce");
    let [x, y] = tray.rail("Smooth bend");
    tray.touch([x, y], false);
    tray.touch([x, y], true);
    assert_eq!(tray.ui.state()["deformer"], "bend", "the press opened it");
    for dx in [15.0, 30.0, 45.0] {
        tray.ui.ok("pen.input", touch_sample(x + dx, y, true));
        tray.ui
            .host
            .lock()
            .frame()
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let taken = tray.ui.host.lock().shell.take_commands();
    assert_eq!(taken.len(), 1, "one step: {taken:?}");
    assert_eq!(taken[0].command, "gizmo.deformer");
    assert_eq!(taken[0].args["phase"], "update");
    let sent = number(&taken[0].args["transforms"][0]["op"]["bend"]["degrees"]);
    tray.send(taken[0].clone());
    let shown = tray.widget("Smooth bend");
    let held = number(&shown["command"]["args"]["op"]["bend"]["degrees"]);
    assert!(held > 0.0, "{shown}");
    assert_eq!(sent, held, "the last value");
    tray.touch([x + 45.0, y], false);
}

#[test]
fn with_the_gizmo_tool_on_a_pull_asks_for_the_gizmo_again_only_after_its_accept() {
    let mut tray = Tray::new("gizmo");
    tray.ui.ok("ui.gizmo", json!({ "tool": true }));
    tray.run();
    tray.sent.clear();
    tray.pull("Smooth bend", &[20.0, 40.0, 60.0]);
    let names: Vec<String> = (tray.sent.iter())
        .map(|s| format!("{} {}", s.command, text(&s.args["phase"])))
        .map(|n| n.trim_end().to_string())
        .collect();
    let accept = names.iter().position(|n| n == "gizmo.deformer accept");
    let accept = accept.unwrap_or_else(|| panic!("no accept: {names:?}"));
    let refreshes = |of: &[String]| of.iter().filter(|n| *n == "ui.gizmo").count();
    assert_eq!(
        refreshes(&names[..accept]),
        0,
        "none while it is open: {names:?}"
    );
    assert_eq!(refreshes(&names[accept..]), 1, "one after: {names:?}");
}

/// The amount a smoothing slider's widget runs now, as `ui.describe` reports it.
fn amount(tray: &mut Tray, slider: &str) -> f64 {
    let shown = tray.widget(slider);
    number(&shown["command"]["args"]["amount"])
}

#[test]
fn a_smooth_pull_opens_a_smoothing_sends_the_last_value_once_a_frame_and_lands_one_step() {
    let mut tray = Tray::new("smooth");
    let (undos, from) = (tray.undos(), tray.lines().len());
    tray.sent.clear();
    let [x, y] = tray.rail("Smooth");
    tray.touch([x, y], false);
    tray.touch([x, y], true);
    assert_eq!(tray.ui.state()["deformer"], "smooth", "the press opened it");
    let open = tray.sessions()[0].1.clone();
    assert_eq!(
        (text(&open["phase"]), open.get("transforms")),
        ("open".into(), None)
    );
    assert_eq!(number(&open["smooth"]["amount"]), 0.0, "at 0: {open}");

    for dx in [15.0, 30.0, 45.0] {
        tray.ui.ok("pen.input", touch_sample(x + dx, y, true));
        let frame = tray.ui.host.lock().frame();
        frame.unwrap_or_else(|e| panic!("{e}"));
    }
    let taken = tray.ui.host.lock().shell.take_commands();
    assert_eq!(taken.len(), 1, "one step for three frames: {taken:?}");
    assert_eq!(taken[0].args["phase"], "update", "{taken:?}");
    let sent = number(&taken[0].args["smooth"]["amount"]);
    tray.send(taken[0].clone());
    assert!(sent > 0.0, "a pull to the right smooths: {sent}");
    assert_eq!(amount(&mut tray, "Smooth"), sent, "the last value");
    tray.touch([x + 45.0, y], false);

    let sessions = tray.sessions();
    assert_eq!(sessions.last().map(|(p, _)| p.as_str()), Some("accept"));
    let state = tray.ui.state();
    assert_eq!(state["history.undos"], json!(number(&undos) as u64 + 1));
    assert_eq!(state["history.undo"], "Smooth", "{state}");
    assert_eq!(state["deformer"], Value::Null, "the pull is accepted");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "one line: {lines:?}");
    assert_eq!(lines[0].command, "mesh.smooth");
    assert_eq!(lines[0].args["mode"], "smooth");
    let landed = number(&lines[0].args["amount"]);
    assert_eq!(landed as f32, sent as f32, "the last update's amount");
    assert_eq!(amount(&mut tray, "Smooth"), 0.0, "the slider reads 0 after");
}

#[test]
fn escape_during_a_smooth_pull_leaves_the_part_bit_for_bit_and_no_step() {
    escape_mid_pull("Smooth", "smooth");
}

#[test]
fn a_typed_smooth_is_the_line_a_pull_to_it_leaves() {
    let landed = typed_as_pulled("Smooth", |args| number(&args["amount"]));
    assert_eq!(landed.abs(), 100.0, "the rail's end");
}

#[test]
fn polish_keeps_the_volume_and_polish_free_does_not_in_the_session_and_the_line() {
    let mut tray = Tray::new("polish");
    let from = tray.lines().len();
    tray.sent.clear();
    tray.pull("Polish free", &[30.0, 60.0]);
    let open = tray.sessions()[0].1["smooth"].clone();
    assert_eq!(
        (text(&open["mode"]), &open["keep"]),
        ("polish".into(), &json!(false))
    );
    tray.type_value("Polish", "40");
    let lines = tray.since(from);
    let kept: Vec<(&str, String, &Value)> = (lines.iter())
        .map(|l| (l.command.as_str(), text(&l.args["mode"]), &l.args["keep"]))
        .collect();
    let polish = |keep| ("mesh.smooth", "polish".to_string(), keep);
    assert_eq!(kept, [polish(&json!(false)), polish(&json!(true))]);
}

/// A Smooth typed at 100: its line's `axes`, and how far the sphere's points moved most along
/// X, Y and Z.
fn smoothed(tray: &mut Tray) -> (Value, [f32; 3]) {
    let before = tray.ui.session.mesh("sphere").positions;
    let from = tray.lines().len();
    tray.type_value("Smooth", "100");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let after = tray.ui.session.mesh("sphere").positions;
    let moved = |k: usize| {
        (before.iter().zip(&after))
            .map(|(b, a)| (a[k] - b[k]).abs())
            .fold(0.0, f32::max)
    };
    (lines[0].args["axes"].clone(), [0, 1, 2].map(moved))
}

#[test]
fn with_only_x_lit_a_smoothing_says_so_in_its_axes_and_moves_points_along_x_alone() {
    let mut tray = Tray::new("smooth-axes");
    // Y lit and put out again: the row is touched, X alone lit.
    tray.press("Y");
    tray.press("Y");
    let (axes, moved) = smoothed(&mut tray);
    assert_eq!(axes, json!({ "x": true, "y": false, "z": false }));
    assert!(moved[0] > 1e-3, "x smooths: {moved:?}");
    assert!(moved[1] < 1e-6 && moved[2] < 1e-6, "y and z stay");
}

/// Until the X, Y, Z row is touched the Smoothing sliders work along all three, though the row
/// starts with X alone lit for Reshape (decision 0062).
#[test]
fn until_the_axis_row_is_touched_a_smoothing_works_along_x_y_and_z() {
    let mut tray = Tray::new("smooth-untouched");
    let (axes, moved) = smoothed(&mut tray);
    assert_eq!(axes, json!({ "x": true, "y": true, "z": true }));
    assert!(moved.iter().all(|m| *m > 1e-3), "all three: {moved:?}");
    let lit = |tray: &mut Tray, axis: &str| tray.widget(axis)["selected"].clone();
    assert_eq!(lit(&mut tray, "X"), json!(true), "the row as it was");
    assert_eq!(lit(&mut tray, "Y"), json!(false), "the row as it was");
}

#[test]
fn a_relax_pull_on_a_part_with_levels_relaxes_the_shown_level_and_a_switch_carries_it_up() {
    let mut tray = Tray::new("levels");
    tray.press("Y");
    tray.press("Z");
    tray.ui.ok("level.subdivide", json!({ "part": "sphere" }));
    let top = tray.bits("sphere");
    tray.ui
        .ok("level.set", json!({ "part": "sphere", "level": 1 }));
    let (base, undos, from) = (tray.bits("sphere"), tray.undos(), tray.lines().len());
    tray.pull("Relax", &[30.0, 60.0]);

    let state = tray.ui.state();
    assert_eq!(state["history.undos"], json!(number(&undos) as u64 + 1));
    assert_eq!(state["history.undo"], "Smooth", "{state}");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].command, "mesh.smooth");
    assert_eq!(lines[0].args["mode"], "relax");
    let relaxed = tray.bits("sphere");
    assert_eq!(relaxed.len(), base.len(), "the base level, still shown");
    assert_ne!(relaxed, base, "the shown level relaxed");

    tray.ui
        .ok("level.set", json!({ "part": "sphere", "level": 2 }));
    let carried = tray.bits("sphere");
    assert_eq!(carried.len(), top.len(), "the top level");
    assert_ne!(carried, top, "the switch carried the relaxing up");
}

#[test]
fn repeat_on_other_parts_runs_a_polish_again_with_mask_off_written_in() {
    let mut tray = Tray::new("repeat-polish");
    let b = json!({ "name": "b", "radius": 0.5, "center": [3, 0, 0] });
    tray.ui.ok("primitive.sphere", b);
    tray.ui.ok("part.select", json!({ "part": "sphere" }));
    tray.type_value("Polish", "40");
    tray.press("Mask");
    let from = tray.lines().len();
    tray.press("Repeat on other Parts");
    let lines = tray.since(from);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (command, args) = (lines[0].command.as_str(), &lines[0].args);
    assert_eq!((command, text(&args["part"])), ("mesh.smooth", "b".into()));
    assert_eq!(
        (text(&args["mode"]), &args["keep"]),
        ("polish".into(), &json!(true))
    );
    assert_eq!(args["mask"], json!(false), "Mask off: {args}");
}
