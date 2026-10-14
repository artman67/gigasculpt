//! The adjust panel (decision 0060, section 5, as amended 2026-10-13, step H2). After a run
//! from a header menu, the palette or a key that made one new undo step of an edit, it shows the
//! step's arguments in the palette's form, in the 3D view's lower corner away from the pen. A
//! change undoes the step and runs it again with the new value; a refused one puts the old step
//! back with `history.redo` and says why under its field. It closes once its step is no longer
//! the newest, on its × and on Escape over it; F9 (`ui.adjust`) shows it again while its step is
//! the newest.

use egui::{Align2, RichText, vec2};
use gs_engine::{AdjustInfo, CallReport, Hand, UiAdjust};
use gs_protocol::{Catalog, CommandError, CommandInfo, Kind, Role, Step};
use serde_json::json;

use crate::arrange::{Asked, titled_row};
use crate::palette::Form;
use crate::pieces::Now;
use crate::record::{Recorder, Role as Shown, Widget, with_args};
use crate::shell::Shell;
use crate::theme::theme;

/// The panel's width, in points.
const WIDTH: f32 = 260.0;
/// Its section's id in the layout's folded list.
const FOLD_ID: &str = "adjust";
/// The pinned arguments that read the view or the pointer: a step with one runs again from what
/// the view was when it ran (a bare `view` only names a view).
const VIEWED: [&str; 4] = ["camera", "viewport", "pivot", "point"];
/// How many commands the doors handed on that it remembers until their calls are heard.
const OFFERED: usize = 16;

/// What × says on hover (decision 0060, rule 8 of section 8 and section 11).
const CLOSE: &str = "Closes the panel and keeps the step as it is. Key: Escape over the panel; \
                     F9 shows it again.";
/// What the title, the panel's fold, says on hover.
const FOLD: &str = "Folds the panel to this row or opens it again. Its fields change the step \
                    just made: it is undone and run again with the new value, as one step. Key: \
                    F9 shows it again. (Blender: Adjust Last Operation.)";
/// The line under the title for a step that read the view or the pointer, and its hint.
const VIEWED_LINE: &str = "Uses the view as it was when it ran";
const VIEWED_HINT: &str = "This step read the view or the pointer when it ran. A change runs it \
                           again from the same view and pick, not from the view as it is now.";

/// The adjust panel, and what the window's doors handed on.
#[derive(Default)]
pub struct Adjust {
    /// The commands the doors (the open menu, the palette, the keymap) handed on, oldest first,
    /// each until its call is heard.
    offered: Vec<String>,
    /// The last panel, kept while closed so F9 can show it again.
    panel: Option<Panel>,
    /// Whether it shows.
    open: bool,
}

/// One step's panel.
struct Panel {
    /// Its step's id in the history.
    id: u64,
    /// The step's arguments, as the palette's form draws them.
    form: Form,
    /// The command reads the view or the pointer.
    viewed: bool,
    /// The change on its way.
    change: Option<Change>,
    /// The field the last change was to, where its refusal shows.
    field: String,
    /// The command whose refusal is the panel's to show.
    awaiting: Option<String>,
}

/// Where a change is: each call queued once the one before it is heard.
enum Change {
    /// The step's undo is queued; this step runs once it is heard.
    Undoing(Step),
    /// The changed step is queued.
    Running,
    /// The changed step was refused; the old step's redo is queued.
    PuttingBack,
}

impl Adjust {
    /// Hands on what a door asked to run, remembering each command until its call is heard:
    /// the one place a run from a menu, the palette or a key passes (section 1 of the
    /// amendment).
    pub fn offer(&mut self, steps: impl IntoIterator<Item = Step>) -> Vec<Step> {
        let steps: Vec<Step> = steps.into_iter().collect();
        self.offered.extend(steps.iter().map(|s| s.command.clone()));
        let over = self.offered.len().saturating_sub(OFFERED);
        self.offered.drain(..over);
        steps
    }

    /// Takes in a call, `before` the undo step's id before it: follows the panel's own change,
    /// closes it once its step is no longer the newest, and opens one for a door's run that made
    /// a new undo step of an edit. Returns what to run next.
    pub fn heard(
        &mut self,
        report: &CallReport,
        before: Option<u64>,
        catalog: &Catalog,
    ) -> Vec<Step> {
        let newest = report.state.history.undo_id;
        if let Some(panel) = self.panel.as_mut().filter(|p| p.change.is_some()) {
            let Some(next) = panel.follow(report) else {
                return Vec::new();
            };
            if panel.change.is_none() && Some(panel.id) != newest {
                self.open = false;
            }
            return next;
        }
        if self.panel.as_ref().is_some_and(|p| Some(p.id) != newest) {
            self.open = false;
        }
        self.opened(report, before, catalog);
        Vec::new()
    }

    /// Opens a panel when `report` is the call of a command a door handed on, an edit that made
    /// a new undo step with a field to show.
    fn opened(&mut self, report: &CallReport, before: Option<u64>, catalog: &Catalog) {
        let Some(command) = self.ran(report) else {
            return;
        };
        let history = &report.state.history;
        let made = history.undo_id.filter(|id| before != Some(*id));
        let step = (history.undo_step.as_ref()).filter(|(c, _)| *c == command);
        let edit = |c: &&CommandInfo| c.name == command && c.kind == Kind::Edit;
        let info = catalog.commands.iter().find(edit);
        let (Some(id), Some((command, args)), Some(info)) = (made, step, info) else {
            return;
        };
        let step = with_args(command, (**args).clone());
        if let Some(panel) = Panel::of(id, step, info) {
            self.panel = Some(panel);
            self.open = true;
        }
    }

    /// The command `report` ran when it is the human's call of one command a door handed on;
    /// that and every command handed on before it are forgotten.
    fn ran(&mut self, report: &CallReport) -> Option<String> {
        let [command] = report.commands else {
            return None;
        };
        if report.role != Role::Human {
            return None;
        }
        let at = self.offered.iter().position(|c| c == command)?;
        self.offered.drain(..=at);
        Some(command.clone())
    }

    /// Takes the refusal `e` of `step` when it is the panel's own change: its reason, under the
    /// field that changed. Returns whether it was.
    pub fn refused(&mut self, step: &Step, e: &CommandError) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        if panel.awaiting.as_deref() != Some(step.command.as_str()) {
            return false;
        }
        panel.awaiting = None;
        panel.form.reason = Some((panel.field.clone(), e.message.clone()));
        true
    }

    /// Whether it shows, and its command and step while it does.
    fn info(&self) -> AdjustInfo {
        let shown = self.panel.as_ref().filter(|_| self.open);
        AdjustInfo {
            open: shown.is_some(),
            command: shown.map(|p| p.form.step.command.clone()),
            step: shown.map(|p| p.id),
        }
    }

    /// Draws the panel while it shows, in `view`'s lower corner away from the pen (section 12):
    /// its title row, the line about the view, then its fields, scrolling past the view's
    /// height. Escape with the pointer over it closes it. Returns what it asked to run and the
    /// fold it asked for.
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        view: egui::Rect,
        now: &Now,
        rec: &mut Recorder,
    ) -> Asked {
        let mut asked = Asked::default();
        let Some(panel) = self.panel.as_mut().filter(|_| self.open) else {
            return asked;
        };
        let inset = theme(ui.ctx()).spacing.margin;
        let (corner, pivot) = match now.layout.hand {
            Hand::Right => (
                view.left_bottom() + vec2(inset, -inset),
                Align2::LEFT_BOTTOM,
            ),
            Hand::Left => (
                view.right_bottom() + vec2(-inset, -inset),
                Align2::RIGHT_BOTTOM,
            ),
        };
        let folded = now.layout.is_folded(FOLD_ID);
        let room = (view.height() - 2.0 * inset).max(0.0);
        let shown = egui::Window::new("Adjust")
            .id(egui::Id::new("adjust"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .pivot(pivot)
            .fixed_pos(corner)
            .show(ui.ctx(), |ui| {
                ui.set_width(WIDTH);
                let title = panel.form.title().to_string();
                rec.panel("adjust", &title, ui.max_rect());
                let (close, fold) = title_bar(ui, &title, folded, rec);
                asked.steps.extend(close);
                if fold {
                    asked.fold.push(FOLD_ID.into());
                }
                if !folded {
                    let room = room - (ui.cursor().top() - ui.max_rect().top());
                    asked.steps.extend(panel.body(ui, room, now, rec));
                }
            });
        let Some(shown) = shown else {
            return asked;
        };
        let rect = shown.response.rect;
        rec.fit(rect);
        let over = (ui.ctx().pointer_latest_pos()).is_some_and(|p| rect.contains(p));
        if over && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
        }
        asked
    }
}

impl Panel {
    /// The panel for the step `id`, `step` of the command `info`; `None` when the form has no
    /// field to show for it.
    fn of(id: u64, step: Step, info: &CommandInfo) -> Option<Panel> {
        let form = Form::adjusting(step, info);
        let viewed = (info.pinned.iter()).any(|p| VIEWED.contains(&p.as_str()));
        (!form.fields.is_empty()).then(|| Panel {
            id,
            form,
            viewed,
            change: None,
            field: String::new(),
            awaiting: None,
        })
    }

    /// Follows the change through `report` when it is the call the change waits for; `None`
    /// for any other call. The undo heard runs the changed step; a refused step puts the old one
    /// back; the step heard, or the old one put back, is the panel's step from then on (one that
    /// made no new step leaves the panel to close). Returns what to run next.
    fn follow(&mut self, report: &CallReport) -> Option<Vec<Step>> {
        let change = self.change.take()?;
        let waits = match &change {
            Change::Undoing(_) => "history.undo".to_string(),
            Change::Running => self.form.step.command.clone(),
            Change::PuttingBack => "history.redo".to_string(),
        };
        if report.role != Role::Human || !matches!(report.commands, [c] if *c == waits) {
            self.change = Some(change);
            return None;
        }
        let history = &report.state.history;
        Some(match (change, report.ok) {
            (Change::Undoing(step), true) => {
                self.change = Some(Change::Running);
                vec![step]
            }
            (Change::Running, false) => {
                self.awaiting = Some(waits);
                self.change = Some(Change::PuttingBack);
                vec![with_args("history.redo", json!({ "expect": self.id }))]
            }
            (Change::Undoing(_), false) => {
                self.awaiting = Some(waits);
                Vec::new()
            }
            (change, true) => {
                let ran = matches!(change, Change::Running);
                let newest = history.undo_id.zip(history.undo_step.as_ref());
                let ours = |id: u64, command: &str| {
                    let fresh = if ran { id > self.id } else { id == self.id };
                    fresh && command == self.form.step.command
                };
                if let Some((id, (command, args))) = newest.filter(|(id, (c, _))| ours(*id, c)) {
                    self.id = id;
                    self.form.fill(with_args(command, (**args).clone()));
                }
                Vec::new()
            }
            (Change::PuttingBack, false) => Vec::new(),
        })
    }

    /// The line about the view and the fields, disabled while a change is on its way, in a list
    /// `room` tall at most. A control that sets a value, or Enter in a typed field, changes the
    /// step: its undo is queued, and the panel waits for it.
    fn body(&mut self, ui: &mut egui::Ui, room: f32, now: &Now, rec: &mut Recorder) -> Vec<Step> {
        if self.viewed {
            let line = ui.label(RichText::new(VIEWED_LINE).small().weak());
            let line = line.on_hover_text(VIEWED_HINT);
            rec.widget(Widget::plain(VIEWED_LINE, Shown::Label), line.rect);
            rec.hint(VIEWED_HINT);
        }
        let idle = self.change.is_none();
        let before = self.form.chosen.clone();
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let outer = rec.listing();
        // The room is given, not left to the window, which offers only what it had last frame.
        let fields = |ui: &mut egui::Ui| {
            let area = egui::ScrollArea::vertical().id_salt("adjust fields");
            area.max_height(room).show(ui, |ui| {
                rec.list(Some(ui.clip_rect()));
                self.form.arguments(ui, &mut false, now, rec);
            });
        };
        ui.add_enabled_ui(idle, |ui| ui.allocate_ui(vec2(WIDTH, room), fields));
        rec.list(outer);
        let chose = (self.form.chosen.iter()).any(|(n, v)| before.get(n) != Some(v));
        let field = (idle && (chose || enter)).then(|| self.form.changed());
        let Some(Some(field)) = field else {
            return Vec::new();
        };
        self.form.reason = None;
        self.field = field;
        self.change = Some(Change::Undoing(self.form.step()));
        vec![with_args("history.undo", json!({ "expect": self.id }))]
    }
}

/// The title row: × at its end, then the command's title, which folds the panel to this row and
/// opens it again. Returns what × asked to run, and whether the title was tapped.
fn title_bar(
    ui: &mut egui::Ui,
    title: &str,
    folded: bool,
    rec: &mut Recorder,
) -> (Option<Step>, bool) {
    let mut tapped = (None, false);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            tapped.0 = close(ui, rec);
            let row = titled_row(ui, title, folded).0.on_hover_text(FOLD);
            rec.widget(Widget::plain(title, Shown::Label), row.rect);
            rec.hint(FOLD);
            rec.section(FOLD_ID, title, folded, false, row.rect);
            tapped.1 = row.clicked();
        });
    });
    tapped
}

/// ×, which runs `ui.adjust` with `open = false`; what it asked to run when tapped.
fn close(ui: &mut egui::Ui, rec: &mut Recorder) -> Option<Step> {
    let step = with_args("ui.adjust", json!({ "open": false }));
    let tint = theme(ui.ctx()).colors.text;
    let button = match crate::icons::image(ui, "x", tint) {
        Some(image) => egui::Button::image(image),
        None => egui::Button::new("×"),
    };
    let shown = ui.add(button).on_hover_text(CLOSE);
    crate::descriptions::note(ui, &shown, &step, None);
    let widget = Widget::runs("Close", Shown::Button, step.clone(), None);
    rec.widget(widget, shown.rect);
    rec.hint(CLOSE);
    shown.clicked().then_some(step)
}

impl Shell {
    /// `ui.adjust`: shows the last panel again while its step is the newest undo step, else
    /// says on the status line there is nothing to adjust; `open = false` closes it.
    pub fn adjust_action(&mut self, change: &UiAdjust) -> AdjustInfo {
        let newest = self.state.history.undo_id;
        let adjust = &mut self.adjust;
        match change.open {
            Some(true) if adjust.panel.as_ref().is_some_and(|p| Some(p.id) == newest) => {
                adjust.open = true;
            }
            Some(true) => self.status.problem = Some("Nothing to adjust".into()),
            Some(false) => adjust.open = false,
            None => {}
        }
        self.adjust.info()
    }
}
