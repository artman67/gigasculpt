//! The status line along the bottom of the window: the window's job while one runs (its bar and
//! Cancel), a low disk warning, what the pen will do (the brush, symmetry, the held modifier),
//! whether Isolate or Dynamic isolate is on, which step the last undo or redo took, what agents
//! did, how big the scene is and how much memory the app holds, the session, and the agents'
//! pause toggle. What does not fit is cut short.

use egui::{Align, FontSelection, Layout, RichText, TextWrapMode, WidgetText};
use gs_engine::server::JobWatch;
use gs_engine::{Shows, StateValues};
use gs_protocol::Step;
use serde_json::json;

use crate::keymap::Keymap;
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// The status line's height, in points.
pub const HEIGHT: f32 = 36.0;

/// What the status line shows of the session.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub session: String,
    /// Document revision.
    pub rev: u64,
    /// What an agent did last: who, which commands, and whether it worked.
    pub agent: Option<String>,
    /// The last thing that went wrong in the window.
    pub problem: Option<String>,
    /// What the window waits for, first on the line: the key to bind to a control.
    pub prompt: Option<String>,
    /// Time to build the last frame, in milliseconds.
    pub frame_ms: Option<f64>,
    /// The scene's size as the engine last reported it.
    pub scene: Scene,
    /// The memory the app holds, in mebibytes, as the window last measured it; `None` where the
    /// system does not say, and in tests, so their frames stay the same.
    pub memory_mb: Option<u64>,
    /// A step refused while the pen held the session, to send again once it lifts
    /// (`gizmo::deformer::held_back`).
    pub waiting: Option<Step>,
    /// The window's job while it runs, which the bar at the start of the line shows.
    pub busy: Option<Busy>,
}

/// The window's job while it runs (decision 0010 as amended 2026-10-13): what the bar at the
/// start of the status line shows, as the window last read it.
#[derive(Clone, Debug)]
pub struct Busy {
    pub job: JobWatch,
    /// How far it got (0 to 1), where its command says.
    pub done: Option<f32>,
    /// What it is doing now, where its command says.
    pub stage: Option<String>,
}

impl Busy {
    pub fn new(job: JobWatch) -> Self {
        let (done, stage) = job.progress();
        Self { job, done, stage }
    }
}

/// Reads the window's job again, each pass of the window's loop: true when the bar changed, so
/// the window draws it again. The bar goes once the job ends; its result or its failure reaches
/// the line through the call report, as every call's does.
pub fn follow(status: &mut Status) -> bool {
    let Some(busy) = &mut status.busy else {
        return false;
    };
    if busy.job.ended() {
        status.busy = None;
        return true;
    }
    let (done, stage) = busy.job.progress();
    let changed = (done, &stage) != (busy.done, &busy.stage);
    (busy.done, busy.stage) = (done, stage);
    changed
}

/// How big the scene is: its parts, and their points and faces at the detail level shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    pub parts: usize,
    pub points: u64,
    pub faces: u64,
}

impl Scene {
    /// As the status line writes it: `1 part  2,562 points  5,120 faces`, or `No parts`.
    pub fn text(&self) -> String {
        match self.parts {
            0 => "No parts".into(),
            1 => format!("1 part  {}", self.counts()),
            n => format!("{n} parts  {}", self.counts()),
        }
    }

    fn counts(&self) -> String {
        let (points, faces) = (grouped(self.points), grouped(self.faces));
        format!("{points} points  {faces} faces")
    }
}

/// `n` with its thousands set apart: 1234567 is `1,234,567`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let threes: Vec<&[u8]> = digits.as_bytes().rchunks(3).rev().collect();
    String::from_utf8_lossy(&threes.join(&b',')).into_owned()
}

/// Memory as the status line writes it: `Memory 412 MB`, or `Memory 1.4 GB` from a gibibyte.
pub fn memory_text(mb: u64) -> String {
    if mb < 1024 {
        format!("Memory {mb} MB")
    } else {
        format!("Memory {:.1} GB", mb as f64 / 1024.0)
    }
}

/// The memory the app holds now (its resident set), in mebibytes, where the system says:
/// Linux's `/proc/self/status`. `None` elsewhere.
pub fn resident_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = text.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// What the last undo or redo took, as the status line writes it (`Undid Draw stroke`,
/// `Redid Draw stroke`), from `state`'s `history.moved` and the step it moved over. `None`
/// until an undo or redo, and again once a new step is made.
fn history_text(state: &StateValues) -> Option<String> {
    let text = |name: &str| state.get(name).and_then(serde_json::Value::as_str);
    match text("history.moved")? {
        "undo" => Some(format!("Undid {}", text("history.redo")?)),
        "redo" => Some(format!("Redid {}", text("history.undo")?)),
        _ => None,
    }
}

/// Whether the view shows only some parts, as the status line writes it (decision 0028,
/// section 10): `Parts isolated` while Isolate is on, else `Dynamic isolate` while that option
/// is on, as it may hide the other parts at any camera move.
fn isolate_text(state: &StateValues) -> Option<&'static str> {
    let on = |name: &str| state.get(name).and_then(serde_json::Value::as_bool) == Some(true);
    if on("parts.isolated") {
        Some("Parts isolated")
    } else if on("settings.dynamic_isolate") {
        Some("Dynamic isolate")
    } else {
        None
    }
}

/// The status line, starting with the window's job's bar while one runs, then `sculpt` (what
/// the pen will do). Its pause toggle shows the `session.paused` value of `state`. Returns the
/// command the toggle or the bar's Cancel asks for.
pub fn status_bar(
    ui: &mut egui::Ui,
    status: &Status,
    state: &StateValues,
    sculpt: &[String],
    keymap: &Keymap,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut asked = None;
    egui::Panel::bottom("status")
        .exact_size(HEIGHT)
        .show(ui, |ui| {
            rec.panel("status", "Status bar", ui.max_rect());
            // The toggle first, at the right end, so a long line never pushes it out of reach.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let paused = state.get(PAUSED).and_then(serde_json::Value::as_bool);
                asked = pause_toggle(ui, paused.unwrap_or(false), keymap, rec);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let rec = &mut *rec;
                    let row = &mut Line {
                        ui,
                        rec,
                        gap: false,
                        full: false,
                    };
                    if let Some(busy) = &status.busy {
                        asked = bar(row, busy).or(asked.take());
                    }
                    line(row, status, state, sculpt);
                });
            });
        });
    asked
}

/// The bar's size, in points.
const BAR: egui::Vec2 = egui::vec2(96.0, 8.0);

/// How long the moving band takes to go along the bar and back, in seconds.
const CROSSING: f64 = 2.0;

/// The window's job, first on the line (decision 0010 as amended 2026-10-13): its title, a bar
/// filled to the fraction done, or with a band moving along it where its command says none,
/// Cancel, then what it is doing now. Cancel's step, when it was clicked.
fn bar(line: &mut Line, busy: &Busy) -> Option<Step> {
    let title = busy.job.title();
    line.label(title.into(), None);
    // Room for the bar and Cancel, which are never cut short.
    let room = BAR.x + line.ui.spacing().interact_size.x * 2.0;
    if line.full || line.ui.available_width() < room {
        line.full = true;
        return None;
    }
    let ui = &mut *line.ui;
    let (rect, _) = ui.allocate_exact_size(BAR, egui::Sense::hover());
    let (track, fill) = {
        let colors = &theme(ui.ctx()).colors;
        (colors.control, colors.lit)
    };
    let round = BAR.y / 2.0;
    ui.painter().rect_filled(rect, round, track);
    let (filled, said) = if let Some(done) = busy.done {
        let x = rect.min.x + rect.width() * done.clamp(0.0, 1.0);
        let filled = egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.max.y));
        (filled, format!("{:.0}%", done * 100.0))
    } else {
        // Back and forth, at an even pace, from the left end.
        let band = rect.width() / 4.0;
        let phase = (ui.input(|i| i.time) / CROSSING).fract() as f32;
        let x = rect.min.x + (1.0 - (2.0 * phase - 1.0).abs()) * (rect.width() - band);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        let band = egui::Rect::from_x_y_ranges(x..=x + band, rect.y_range());
        (band, "working".into())
    };
    ui.painter().rect_filled(filled, round, fill);
    line.rec.widget(Widget::plain(said, Role::Label), rect);
    let cancel = json!({ "id": busy.job.id(), "cancel": true });
    let step = with_args(gs_protocol::method::JOB_STATUS, cancel);
    let hint =
        format!("Stops {title} at once; the scene stays as it was and History gains no step.");
    let cancel = line.ui.button("Cancel").on_hover_text(&hint);
    let widget = Widget::runs("Cancel", Role::Button, step.clone(), None);
    line.rec.widget(widget, cancel.rect);
    line.rec.hint(&hint);
    if let Some(stage) = &busy.stage {
        let mut letters = stage.chars();
        let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
        line.label(
            format!("{}{}", first.unwrap_or_default(), letters.as_str()).into(),
            None,
        );
    }
    line.gap();
    cancel.clicked().then_some(step)
}

/// The status line's labels, in order: the low disk warning, then what the pen will do, then
/// what changed last, so a narrow window cuts off the end of the line.
fn line(line: &mut Line, status: &Status, state: &StateValues, sculpt: &[String]) {
    let (warned, refused) = {
        let colors = &theme(line.ui.ctx()).colors;
        (colors.warning(), colors.problem)
    };
    if let Some((text, hover)) = low_disk_text(state) {
        line.label(RichText::new(text).color(warned), Some(&hover));
        line.gap();
    }
    for (text, hover) in crate::gizmo::deformer::waits(status.waiting.as_ref(), state) {
        line.label(text.into(), hover);
        line.gap();
    }
    for part in sculpt {
        line.label(part.into(), None);
        line.gap();
    }
    if let Some(isolate) = isolate_text(state) {
        line.label(isolate.into(), None);
        line.gap();
    }
    if let Some(moved) = history_text(state) {
        line.label(moved.into(), None);
        line.gap();
    }
    let agent = status.agent.as_deref().unwrap_or("no agent activity");
    line.label(format!("Agent: {agent}").into(), None);
    if let Some(problem) = &status.problem {
        let text = RichText::new(format!("Problem: {problem}"));
        line.label(text.color(refused), None);
    }
    line.gap();
    line.label(status.scene.text().into(), None);
    if let Some(mb) = status.memory_mb {
        line.label(memory_text(mb).into(), None);
    }
    line.gap();
    line.label(format!("Session {}", status.session).into(), None);
    line.label(format!("rev {}", status.rev).into(), None);
    if let Some(ms) = status.frame_ms {
        line.label(format!("{ms:.1} ms").into(), None);
    }
}

/// The low disk warning (decision 0016, section 17, item 9), from `state`'s `file.low_disk`:
/// the line's text and what it says on hover. `None` while there is room.
fn low_disk_text(state: &StateValues) -> Option<(String, String)> {
    let low = state.get("file.low_disk").filter(|v| !v.is_null())?;
    let mb = |key: &str| low[key].as_u64().unwrap_or_default() >> 20;
    let folder = low["folder"].as_str().unwrap_or_default();
    let (free, threshold) = (mb("free"), mb("threshold"));
    let text = format!("Low disk space: {free} MB free");
    let hover = format!(
        "{folder} has {free} MB free, under the {threshold} MB a save and the next one need \
         there. Free some space, or save somewhere else."
    );
    Some((text, hover))
}

/// The status line's labels, laid left to right in the room beside the toggle (decision 0016,
/// section 17, item 10): the first that does not fit is cut short with an ellipsis, all of it
/// said on hover, and the rest are left out. A separator shows only between two labels.
struct Line<'u, 'r> {
    ui: &'u mut egui::Ui,
    rec: &'r mut Recorder,
    /// A separator goes before the next label.
    gap: bool,
    /// A label was cut short: the line is full.
    full: bool,
}

/// The least room a label is shown in, cut short: an ellipsis and a letter or two.
const LEAST: f32 = 24.0;

impl Line<'_, '_> {
    /// A separator before the next label, if one fits.
    fn gap(&mut self) {
        self.gap = true;
    }

    /// `text`, saying `hover` on hover, if there is room.
    fn label(&mut self, text: RichText, hover: Option<&str>) {
        let ui = &mut *self.ui;
        let gap = std::mem::take(&mut self.gap);
        // A separator takes its own spacing and the gap after it.
        let separator = if gap {
            ui.spacing().item_spacing.x * 2.0
        } else {
            0.0
        };
        if self.full || ui.available_width() < LEAST + separator {
            self.full = true;
            return;
        }
        if gap {
            ui.separator();
        }
        let wrap = Some(TextWrapMode::Extend);
        let wide = (WidgetText::from(text.clone()))
            .into_galley(ui, wrap, f32::INFINITY, FontSelection::Default)
            .size()
            .x;
        let fits = wide <= ui.available_width();
        let widget = Widget::plain(text.text(), Role::Label);
        let label = egui::Label::new(text);
        let shown = ui.add(if fits { label } else { label.truncate() });
        let shown = match hover {
            Some(hover) => shown.on_hover_text(hover),
            None => shown,
        };
        self.full = !fits;
        (self.rec).widget(widget, shown.rect);
    }
}

/// The state value the pause toggle shows.
const PAUSED: &str = "session.paused";

/// "Pause agents": one click asks the engine to pause or resume agents. It shows the engine's
/// state, not its own, so it is never out of step.
fn pause_toggle(
    ui: &mut egui::Ui,
    paused: bool,
    keymap: &Keymap,
    rec: &mut Recorder,
) -> Option<Step> {
    let text = if paused {
        "Agents paused"
    } else {
        "Pause agents"
    };
    let toggle = ui.selectable_label(paused, text);
    let command = with_args("session.control", json!({ "paused": !paused }));
    let key = keymap
        .bindings
        .iter()
        .find(|b| b.command == "session.control")
        .map(|b| b.key.as_str());
    let shows = Shows {
        value: PAUSED.into(),
        on: true.into(),
    };
    let widget = Widget::runs(text, Role::Toggle, command.clone(), key);
    rec.widget(widget.toggle(shows, paused), toggle.rect);
    toggle.clicked().then_some(command)
}
