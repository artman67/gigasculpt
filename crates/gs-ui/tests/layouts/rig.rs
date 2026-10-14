//! A real engine with the window's studio attached, driven the way the window drives it: keys
//! and modifiers through the studio's input, the pen through `pen.input`, and every command the
//! interface asks for run as the human at the window. No renderer unless a test needs the
//! MatCaps, so most layouts tests run fast.

use gs_engine::{EngineConfig, StateFolder};
use gs_platform::{Input, Modifiers};
use gs_protocol::{CmdResponse, ErrorCode, Outcome, Request, Role, RunParams};
use gs_render::{AdapterKind, Renderer};
use gs_testkit::session::Session;
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

use crate::looks::looks;
use crate::region::{middle, region};

/// The window's size in pixels: room for the trays at density 1. Tests find what they press
/// with [`Ui::rect`], never by counting pixels from the window's edge.
pub const SIZE: [u32; 2] = [1800, 1100];

pub struct Ui {
    pub session: Session,
    pub host: Host,
}

impl Ui {
    /// A window with the shipped layouts and no renderer.
    pub fn new() -> Self {
        Self::with(false)
    }

    /// A window drawing on lavapipe, so it lists the MatCaps and is the session's renderer.
    pub fn with_looks() -> Self {
        Self::with(true)
    }

    /// A window with the shipped layouts, no renderer, and its layout state in `layout` (a
    /// file, so it is saved as a real window saves it).
    pub fn with_layout(layout: LayoutFile) -> Self {
        Self::build(EngineConfig::default(), false, layout)
    }

    /// A window started as the `gigasculpt` window starts (`app::run`): its files read from
    /// `config` by `app::shell` (keymap, layout state, layout files, themes, preferences),
    /// autosave on in the state folder `state`, and what the window runs before its first frame
    /// run by `app::launch`. No frame is drawn yet: [`Ui::settle`] draws the first.
    pub fn launch(config: &std::path::Path, state: &std::path::Path) -> Self {
        let engine = EngineConfig {
            autosave: Some(StateFolder::in_folder(state.to_path_buf())),
            ..EngineConfig::default()
        };
        let mut session = Session::with(engine, Role::Human);
        let mut shell = gs_ui::app::shell(config);
        shell.catalog = session.engine.catalog().clone();
        let host = Host::new(Studio::new(shell, None, SIZE, 1.0));
        session.engine.attach_window(Box::new(host.clone()));
        gs_ui::app::launch(config, &host, |request| Some(session.request(request)));
        Self { session, host }
    }

    fn with(looks: bool) -> Self {
        Self::build(
            EngineConfig::default(),
            looks,
            LayoutFile::in_memory(Layout::default()),
        )
    }

    fn build(config: EngineConfig, looks: bool, layout: LayoutFile) -> Self {
        let mut session = Session::with(config, Role::Human);
        let shell = Shell::new(session.engine.catalog().clone(), Keymap::default(), layout);
        let render = looks.then(|| {
            let renderer = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
            Render::new(renderer, 1).unwrap_or_else(|e| panic!("{e}"))
        });
        let host = Host::new(Studio::new(shell, render, SIZE, 1.0));
        if looks {
            session.engine.attach_renderer(Box::new(host.clone()));
        }
        session.engine.attach_window(Box::new(host.clone()));
        Self { session, host }
    }

    /// Runs a command as the human at the window.
    pub fn call(&mut self, command: &str, args: Value) -> CmdResponse {
        self.session.call(command, args)
    }

    /// Runs a command that must succeed, and returns its result.
    pub fn ok(&mut self, command: &str, args: Value) -> Value {
        match self.call(command, args).outcome {
            Outcome::Ok(ok) => ok.result,
            Outcome::Error(e) => panic!("{command}: {}", e.message),
        }
    }

    /// Runs a command that must fail, and returns its error code and message.
    pub fn err(&mut self, command: &str, args: Value) -> (ErrorCode, String) {
        match self.call(command, args).outcome {
            Outcome::Ok(ok) => panic!("{command} should fail: {}", ok.result),
            Outcome::Error(e) => (e.code, e.message),
        }
    }

    /// Checks a command without running it: the error, if its arguments are refused.
    pub fn dry_run(&mut self, command: &str, args: Value) -> Option<ErrorCode> {
        let request = Request::Run(RunParams::new(command, args).dry());
        match self.session.request(request).outcome {
            Outcome::Ok(_) => None,
            Outcome::Error(e) => Some(e.code),
        }
    }

    pub fn describe(&mut self) -> Value {
        self.ok("ui.describe", json!({}))
    }

    /// The widgets of panel `id`, as `ui.describe` reports them.
    pub fn widgets(&mut self, id: &str) -> Vec<Value> {
        listed(&self.describe(), id, "widgets")
    }

    /// The labels of panel `id`'s widgets, but its scroll bar's.
    pub fn labels(&mut self, id: &str) -> Vec<String> {
        let widgets = self.widgets(id);
        (widgets.iter())
            .filter(|w| w["role"] != "scroll bar")
            .filter_map(|w| Some(w["label"].as_str()?.to_string()))
            .collect()
    }

    /// Where `name` shows now, in pixels (see [`region`]). A `panel > name` in a panel that
    /// scrolls is scrolled into sight first, as the pen scrolls it ([`Ui::scroll_to`]), so a
    /// test reaches it wherever the sections above put it (decision 0025, section 8).
    pub fn rect(&mut self, name: &str) -> [f64; 4] {
        let d = self.describe();
        let Some((id, within)) = name.split_once(" > ") else {
            return region(&d, name);
        };
        let widgets = listed(&d, id, "widgets");
        if !widgets.iter().any(|w| w["role"] == "scroll bar") || shows(&d, id, within) {
            return region(&d, name);
        }
        self.scroll_to(id, within);
        region(&self.describe(), name)
    }

    /// The 3D view's middle, in pixels.
    pub fn middle(&mut self) -> [f64; 2] {
        middle(self.rect("viewport"))
    }

    /// The pen hovering at (`x`, `y`) in pixels, as one window frame sends it, and the frame.
    pub fn pen(&mut self, x: f64, y: f64) {
        let sample = json!({ "samples": [{ "x": x, "y": y }] });
        self.ok("pen.input", sample);
        self.settle();
    }

    /// The pen touching at (`x`, `y`) with half pressure, or lifting there, and the frame.
    pub fn touch(&mut self, x: f64, y: f64, down: bool) {
        self.ok("pen.input", touch_sample(x, y, down));
        self.settle();
    }

    /// A tap of the pen at (`x`, `y`) in pixels: in range, down, lifted.
    pub fn tap_at(&mut self, x: f64, y: f64) {
        for down in [false, true, false] {
            self.touch(x, y, down);
        }
    }

    /// A tap of the pen on the widget `name` (see [`region`]), and the frame that follows.
    pub fn press(&mut self, name: &str) {
        let [x, y] = middle(self.rect(name));
        self.tap_at(x, y);
        self.settle();
    }

    /// A press of the pen on the widget `name` held a second, past the longest click, then
    /// lifted: a header switch with options opens them (decision 0025, rule 8.5).
    pub fn hold(&mut self, name: &str) {
        let [x, y] = middle(self.rect(name));
        self.hold_at(x, y);
    }

    /// A press of the pen at (`x`, `y`) in pixels held a second, then lifted (see [`Ui::hold`]).
    pub fn hold_at(&mut self, x: f64, y: f64) {
        self.touch(x, y, false);
        self.touch(x, y, true);
        self.wait(1.0);
        self.touch(x, y, false);
    }

    /// The button of the brush family `family` on the shelf, as `ui.describe` reports it: the
    /// widget after the family's label (decision 0025, amended 2026-10-12, Shelf-family).
    pub fn face(&mut self, family: &str) -> Value {
        let widgets = self.widgets("shelf");
        let label = |w: &Value| w["label"] == family && w["role"] == "label";
        let at = widgets.iter().position(label);
        let face = at.and_then(|i| widgets.get(i + 1).cloned());
        face.unwrap_or_else(|| panic!("no {family} family on the shelf: {widgets:?}"))
    }

    /// Opens the brush family `family` with a press held on its shelf button.
    pub fn open_family(&mut self, family: &str) {
        let [x, y] = center(&self.face(family)["rect"]);
        self.hold_at(x, y);
    }

    /// Picks `brush` as a hand does on the shelf: its family opened, then its button there.
    pub fn pick(&mut self, brush: &str) {
        let family = (self.host.lock().shell.directions.brushes.iter())
            .find(|b| b.name == brush)
            .map(|b| b.family.clone());
        self.open_family(&family.unwrap_or_else(|| panic!("no brush {brush}")));
        self.press(&format!("family > {brush}"));
    }

    /// Drags the panel `id`, which scrolls, with the pen until `name` (a widget's label or a
    /// section's id) shows in it: its own height at a time toward its foot until it moves no
    /// more, then toward its top, eight drags each way at most. Each drag starts on a widget
    /// that is neither a slider (a press on one would move it), a color swatch (a drag from one
    /// picks a color, decision 0039 section 11) nor the scroll bar.
    pub fn scroll_to(&mut self, id: &str, name: &str) {
        let panel = self.rect(id);
        let tall = panel[3] - panel[1];
        for by in [-tall, tall] {
            let mut before = Vec::new();
            for _ in 0..8 {
                let d = self.describe();
                let widgets = listed(&d, id, "widgets");
                if shows(&d, id, name) {
                    return;
                }
                if widgets == before {
                    break;
                }
                let swatch = |w: &&Value| w["label"] == "Main" || w["label"] == "Secondary";
                let still =
                    |w: &&Value| w["role"] != "slider" && w["role"] != "scroll bar" && !swatch(w);
                let from = widgets.iter().find(still);
                let [x, y] = center(&from.unwrap_or_else(|| panic!("{id} shows a button"))["rect"]);
                self.touch(x, y, false);
                self.touch(x, y, true);
                for step in 1..=4 {
                    self.touch(x, y + by * f64::from(step) / 4.0, true);
                }
                self.touch(x, y + by, false);
                before = widgets;
            }
        }
    }

    /// Opens the header's Look options (see [`Ui::open_options`]).
    pub fn open_look(&mut self) {
        self.open_options("look");
    }

    /// Opens the header's options `id` with a tap on their button, which `ui.describe` lists
    /// as the header's section `id`, and draws a frame with them laid out.
    pub fn open_options(&mut self, id: &str) {
        assert!(self.widgets(id).is_empty(), "the {id} options start shut");
        let [x, y] = middle(self.rect(&format!("header > {id}")));
        self.tap_at(x, y);
        self.settle();
        let open = self.widgets(id);
        assert!(
            !open.is_empty(),
            "a tap on its button opens the {id} options"
        );
    }

    /// Folds every section of the scene tray but `open`, which it opens: the tray is longer
    /// than the window, and `ui.describe` lists only what shows. Then draws the tray, makes a
    /// call so the engine says which of the controls now drawn cannot run (it says after each
    /// call), and draws it again. Returns the tray's sections.
    pub fn open_only(&mut self, open: &[&str]) -> Vec<String> {
        self.open_only_in("tray", open)
    }

    /// As [`Ui::open_only`], in the tray `id`: `tray` or `brush`.
    pub fn open_only_in(&mut self, id: &str, open: &[&str]) -> Vec<String> {
        let mut studio = self.host.lock();
        let shell = &mut studio.shell;
        let shown = shell.directions.get(shell.shown());
        let tray = shown.and_then(|d| d.panels.iter().find(|p| p.id == id));
        let ids = tray.map(|t| t.show.clone()).unwrap_or_default();
        assert!(!ids.is_empty(), "the layout shown has a {id} tray");
        let folded = &mut shell.layout.layout.folded;
        folded.retain(|f| !ids.contains(f));
        let shut = ids.iter().filter(|id| !open.contains(&id.as_str()));
        folded.extend(shut.cloned());
        drop(studio);
        self.settle();
        self.state();
        self.settle();
        ids
    }

    /// Holds `held` and sends `inputs` through the studio, then [`Ui::settle`].
    pub fn input(&mut self, inputs: &[Input], held: Modifiers) {
        self.host.lock().input(inputs, held);
        self.settle();
    }

    /// Lets `secs` pass on the window's clock with no input, as the window's loop does: it
    /// looks every few milliseconds (`looks`) and draws only when the interface asked to draw
    /// again by then (a timer that ran out, egui's frames after an event).
    pub fn wait(&mut self, secs: f64) {
        for step in looks(secs) {
            let asked = {
                let mut studio = self.host.lock();
                let now = studio.now_ns() + step;
                studio.at(now);
                studio.dirty
            };
            if asked {
                self.settle();
            }
        }
    }

    /// Draws frames and runs what the interface asked for, as the human at the window, until
    /// it asks for nothing more (a wheel's pick runs after the `ui.wheel` that picked it).
    pub fn settle(&mut self) {
        for _ in 0..4 {
            let steps = {
                let mut studio = self.host.lock();
                studio.frame().unwrap_or_else(|e| panic!("{e}"));
                studio.shell.take_commands()
            };
            if steps.is_empty() {
                return;
            }
            for step in steps {
                if let Outcome::Error(e) = self.call(&step.command, step.args.clone()).outcome {
                    self.host.lock().shell.refused(Some(step), &e);
                }
            }
        }
    }

    /// Presses and lets go of `key`, one frame each.
    pub fn tap(&mut self, key: &str, held: Modifiers) {
        self.input(&[self::key(key, true, held)], held);
        self.input(&[self::key(key, false, held)], held);
    }

    /// The session's state values (`session.state`).
    pub fn state(&mut self) -> Value {
        self.ok("session.state", json!({}))
    }

    /// Every layout, by name.
    pub fn layouts(&mut self) -> Vec<String> {
        let info = self.ok("ui.layout", json!({}));
        serde_json::from_value(info["layouts"].clone()).unwrap_or_default()
    }

    /// The command log's entries for commands starting with `prefix`, oldest first.
    pub fn logged(&mut self, prefix: &str) -> Vec<Value> {
        self.ok("log.tail", json!({ "prefix": prefix }))["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

/// A fresh, empty folder for a test's files, named `name` (unique among the tests).
pub fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("layouts-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// `pen.input`'s arguments for the pen touching at (`x`, `y`) with half pressure, or lifting.
pub fn touch_sample(x: f64, y: f64, down: bool) -> Value {
    let pressure = if down { 0.5 } else { 0.0 };
    json!({ "samples": [{ "x": x, "y": y, "contact": down, "pressure": pressure }] })
}

/// A key going down or up with `held`.
pub fn key(name: &str, down: bool, held: Modifiers) -> Input {
    Input::Key {
        key: name.into(),
        down,
        repeat: false,
        modifiers: held,
    }
}

/// Modifiers with only Shift, Ctrl or Alt held.
pub fn held(name: &str) -> Modifiers {
    Modifiers {
        shift: name == "Shift",
        ctrl: name == "Ctrl",
        alt: name == "Alt",
        ..Modifiers::default()
    }
}

/// Panel `id`'s `key` (`widgets` or `sections`) in `ui.describe`'s answer `d`.
pub fn listed(d: &Value, id: &str, key: &str) -> Vec<Value> {
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let panel = panels.into_iter().find(|p| p["id"] == id);
    panel
        .and_then(|p| p[key].as_array().cloned())
        .unwrap_or_default()
}

/// Whether a widget labelled `name`, whole (not cut by the panel's edge), or a section with the
/// id `name` shows in panel `id`.
fn shows(d: &Value, id: &str, name: &str) -> bool {
    let [_, top, _, foot] = region(d, id);
    let whole = |w: &Value| w["rect"][1].as_f64() > Some(top) && w["rect"][3].as_f64() < Some(foot);
    let widgets = listed(d, id, "widgets");
    let widget = widgets.iter().any(|w| w["label"] == name && whole(w));
    widget || listed(d, id, "sections").iter().any(|s| s["id"] == name)
}

/// The middle of a rectangle `ui.describe` reports.
pub fn center(rect: &Value) -> [f64; 2] {
    middle(std::array::from_fn(|i| {
        rect[i].as_f64().unwrap_or_default()
    }))
}
