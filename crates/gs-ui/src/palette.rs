//! The command palette: one key opens it, typing narrows every command in the catalog and every
//! control the layouts know, and each row shows its key and where it lives in the layout shown.
//! With nothing typed, what was picked last comes first. Picking a row runs it; a command that
//! needs arguments asks for them in a form first, each drawn by its kind (see [`form`]). A scene
//! file form's `path` starts in the folder that fits it (decision 0016, section 17). In the
//! window a file action asks the system's file dialog instead ([`dialog`]), and the changes a
//! scene would lose are asked about first ([`unsaved`]). A form's life, from asked to sent and
//! kept to refused and reopened, is in [`sent`]. Bind a key… takes the keys pressed ([`bind`]).
//! The rows, ranked and drawn, are in [`rows`].

mod bind;
mod dialog;
mod form;
mod path;
mod rows;
mod sent;
mod unsaved;

use std::collections::BTreeMap;

use egui::{Align2, Key};
use gs_platform::{Answer as Picked, Dialog, Pick};
use gs_protocol::{Catalog, CommandInfo, Step};
use serde_json::Value;

use crate::direction::{PaletteSpec, Place};
use crate::pieces::Now;
use crate::record::Recorder;
use crate::shell::escape::{To, went_to};
use dialog::Wants;
pub use dialog::{Scene, opened};
use form::Answer;
pub use form::Form;
pub use rows::Listed;
pub(crate) use unsaved::guards;
pub use unsaved::{Then, Unsaved};

/// The palette's egui window, which Escape and Enter order by (`shell/escape.rs`).
pub(crate) const WINDOW: &str = "palette";
/// The egui area under the palette while it shows ([`backdrop`]).
const BACKDROP: &str = "palette backdrop";
/// Picks remembered.
const RECENT: usize = 8;
/// The folders' key for scene files, which Open and Save share.
const SCENE: &str = "scene";

/// The palette's state.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    /// The highlighted row.
    pub selected: usize,
    /// The arguments being asked for, if any.
    pub form: Option<Form>,
    /// Where the pointer was when it opened: a palette placed at the pen stays there.
    pub at: Option<egui::Pos2>,
    /// What was picked, newest first.
    recent: Vec<Step>,
    /// Give the first field the keyboard on the next frame.
    focus: bool,
    /// The scene as the engine last reported it: its own file, its active part, the brush and
    /// whether it has unsaved changes.
    pub scene: Scene,
    /// The folder of the newest file each kind of form ran with (see [`Palette::folder`]).
    folders: BTreeMap<&'static str, String>,
    /// File actions ask the system's file dialog rather than the form: the window's, until one
    /// fails to show.
    pub dialogs: bool,
    /// A dialog asked for, for the window to show ([`Palette::take_dialog`]).
    shown: Option<Dialog>,
    /// The step a dialog shown waits to fill, and what it fills.
    waiting: Option<(Step, Wants)>,
    /// The question before unsaved changes are lost, and what waits on it.
    pub unsaved: Unsaved,
    /// The form that sent the last step, as it was typed, until the step's answer (`sent`).
    kept: Option<sent::Kept>,
    /// Bind a key…, while it is open.
    binding: Option<bind::Bind>,
}

impl Palette {
    /// Opens or closes it; `query` replaces the search text.
    pub fn set(&mut self, open: bool, query: Option<&str>) {
        if open && !self.open {
            self.focus = true;
            self.selected = 0;
        }
        self.open = open;
        if !open {
            self.form = None;
            self.at = None;
            self.unsaved.asking = None;
            self.binding = None;
        }
        if let Some(query) = query {
            self.query = query.to_string();
            self.selected = 0;
        }
    }

    /// Escape, given by the window (`shell/escape.rs`) while the palette is in front, typed in
    /// or not: shuts it with its form or question, and what that question held up goes no
    /// further.
    pub fn escape(&mut self) {
        self.set(false, None);
        self.unsaved.stop();
    }

    /// Asks the system's file dialog for the file `step` lacks, as `wants` says; the window
    /// shows it and hands back its answer ([`Palette::answered`]). One at a time.
    fn show_dialog(&mut self, step: Step, wants: Wants) -> Result<(), String> {
        if self.waiting.is_some() {
            return Err("a file dialog is open: answer it first".into());
        }
        let folder = self.folder(&step.command);
        self.shown = Some(dialog::dialog(&step, wants, folder, &self.scene));
        self.waiting = Some((step, wants));
        self.set(false, None);
        Ok(())
    }

    /// The dialog asked for since the last call, for the window to show.
    pub fn take_dialog(&mut self) -> Option<Dialog> {
        self.shown.take()
    }

    /// Takes in a dialog's answer: the file chosen fills the step waiting for it, which runs
    /// (with `overwrite` when the Save dialog picked a file that exists, as it asked already);
    /// Cancel runs nothing. A dialog that could not show asks in the form instead, from then on,
    /// and gives why.
    pub fn answered(&mut self, answer: Picked, catalog: &Catalog) -> Result<Option<Step>, String> {
        let Some((step, wants)) = self.waiting.take() else {
            return Ok(None);
        };
        match answer {
            Picked::Chosen(file) => {
                let text = file.display().to_string();
                let path = dialog::extended(&step.command, &text);
                // The dialog asked about the file it picked, not the one its extension names.
                let asked = path == text && file.is_file();
                let mut step = dialog::given(&step, wants.arg, Value::from(path));
                let info = catalog.commands.iter().find(|c| c.name == step.command);
                let replaces =
                    info.is_some_and(|i| i.args_schema["properties"].get("overwrite").is_some());
                if wants.pick == Pick::Save && asked && replaces {
                    step = dialog::given(&step, "overwrite", Value::Bool(true));
                }
                let step = dialog::routed(step);
                self.remember(&step);
                self.note(&step);
                Ok(Some(step))
            }
            Picked::Cancelled => {
                self.unsaved.stop();
                Ok(None)
            }
            Picked::Failed(why) => {
                self.dialogs = false;
                self.ask(step, catalog, &[wants.arg.into()])?;
                Err(format!(
                    "the file dialog could not show ({why}): type the file's path here"
                ))
            }
        }
    }

    /// The folder a form's `path` starts in, ending in a separator so a name typed after it
    /// makes the path (decision 0016, section 17): for Open and Save, the scene's own file's,
    /// else the newest scene file's opened or saved through a form; for Import and Export,
    /// the newest file's that command ran with, else the scene's; for Save brush and Load brush,
    /// the newest brush file's; for Reference from a file, Import's. Kept for the launch only. A
    /// folder that is gone is passed over, so what is typed is the whole path.
    fn folder(&self, command: &str) -> Option<String> {
        let there = |dir: Option<String>| dir.filter(|d| std::path::Path::new(d).is_dir());
        let own = there(self.scene.own.as_deref().and_then(folder_of));
        let kept = |kind: &str| there(self.folders.get(kind).cloned());
        let scene = || own.clone().or_else(|| kept(SCENE));
        match command {
            "file.open" | "file.save" | "file.save_next" => scene(),
            "file.import" | "file.export" => kept(command).or_else(scene),
            "reference.open" => kept("file.import").or_else(scene),
            "brush.save" | "brush.load" => kept("brush"),
            _ => None,
        }
    }

    /// Notes the folder of the file `step` runs with, for the next form (see `folder`).
    fn note(&mut self, step: &Step) {
        let kind = match step.command.as_str() {
            "file.open" | "file.save" | "file.save_next" => SCENE,
            "file.import" | "reference.open" => "file.import",
            "file.export" => "file.export",
            "brush.save" | "brush.load" => "brush",
            _ => return,
        };
        let arg = dialog::wants(&step.command).map_or("path", |w| w.arg);
        if let Some(dir) = step.args[arg].as_str().and_then(folder_of) {
            self.folders.insert(kind, dir);
        }
    }

    /// Remembers a pick.
    fn remember(&mut self, step: &Step) {
        self.recent.retain(|s| s != step);
        self.recent.insert(0, step.clone());
        self.recent.truncate(RECENT);
    }

    /// Draws the palette if it is open, placed as `spec` says. Returns the command picked,
    /// which closes it; one that lacks arguments opens its form. A press elsewhere shuts it as
    /// Escape does, and does nothing else ([`backdrop`]).
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        catalog: &Catalog,
        listed: &Listed,
        spec: &PaletteSpec,
        now: &Now,
        rec: &mut Recorder,
    ) -> Option<Step> {
        if !self.open {
            return None;
        }
        let ctx = ui.ctx();
        if backdrop(ctx) {
            self.escape();
            return None;
        }
        let enter = self.keys(ctx);
        let window = egui::Window::new("Commands")
            .id(egui::Id::new(WINDOW))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_size([spec.width, 0.0]);
        let window = match (spec.place, self.at) {
            (Place::Cursor, Some(at)) => window.fixed_pos(at),
            (Place::Center, _) => window.anchor(Align2::CENTER_CENTER, [0.0, 0.0]),
            _ => window.anchor(Align2::CENTER_TOP, [0.0, 40.0]),
        };
        let rows = self.rows(catalog, listed);
        let (mut chosen, mut answered, mut bound) = (None, None, None);
        window.show(ui.ctx(), |ui| {
            rec.panel("palette", "Command palette", ui.max_rect());
            if self.unsaved.asking.is_some() {
                answered = self.question(ui, enter, catalog, (now, rec));
                return;
            }
            chosen = match &mut self.form {
                Some(form) => form::draw(ui, form, &mut self.focus, enter, now, rec),
                None if self.binding.is_some() => {
                    bound = self.bind_panel(ui, (&rows, catalog), enter, (now, rec));
                    None
                }
                None => self
                    .contents(ui, &rows, catalog, enter, rec)
                    .map(Answer::Run),
            };
        });
        self.landed(answered, bound, chosen, catalog)
    }

    /// Moves the highlight on the arrows the window gives the palette. Returns whether it gave
    /// the palette Enter.
    fn keys(&mut self, ctx: &egui::Context) -> bool {
        // Enter and the arrows are the palette's when the window gives them (`shell/escape.rs`):
        // not while the recovery dialog or another panel is in front of it.
        let up = went_to(ctx, Key::ArrowUp, To::Palette);
        let down = went_to(ctx, Key::ArrowDown, To::Palette);
        let enter = went_to(ctx, Key::Enter, To::Palette);
        // Moving past the last row is undone where the rows are drawn (`contents`).
        if down {
            self.selected += 1;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        enter
    }

    /// What a frame of the window gave: the question's answer, Bind's step, else the row
    /// picked or the form's, which runs, or Cancel, which shuts the palette.
    fn landed(
        &mut self,
        answered: Option<Step>,
        bound: Option<Answer>,
        chosen: Option<Answer>,
        catalog: &Catalog,
    ) -> Option<Step> {
        if answered.is_some() {
            return answered;
        }
        // Bind's step is sent as it is, not picked: it is no row to remember.
        if let Some(answer) = bound {
            self.set(false, None);
            return match answer {
                Answer::Run(step) => Some(step),
                Answer::Cancel => None,
            };
        }
        match chosen? {
            Answer::Run(step) => self.pick(step, catalog),
            Answer::Cancel => {
                self.set(false, None);
                self.unsaved.stop();
                None
            }
        }
    }

    /// Runs `step`, or asks for the arguments it lacks. Open given a mesh file imports it.
    fn pick(&mut self, step: Step, catalog: &Catalog) -> Option<Step> {
        // New, Open and Recover picked from the list ask about unsaved changes first; picked,
        // they are kept with the recent picks all the same.
        if self.form.is_none() && unsaved::guards(&step.command) {
            self.remember(&step);
            let now = self
                .first(Then::Run(step, Vec::new()), catalog)
                .ok()
                .flatten();
            if now.is_some() {
                self.set(false, None);
            }
            return now;
        }
        let info = catalog.commands.iter().find(|c| c.name == step.command);
        if self.form.is_none() && info.is_some_and(|i| !missing(i, &step.args).is_empty()) {
            self.ask(step, catalog, &[]).ok();
            return None;
        }
        let step = dialog::routed(step);
        self.remember(&step);
        self.note(&step);
        let form = self.form.take();
        self.keep(&step, form);
        self.set(false, None);
        Some(step)
    }
}

/// Draws an area over the whole window that takes every press, over the panels (the 3D view,
/// the trays, the header and the strips) and under every egui window and menu, the palette's
/// among them, and returns whether a press landed on it (decision 0025, section 2; review 0009,
/// C9). That press reaches no panel, so it runs no button there, and the 3D view starts no
/// stroke under the palette (`Shell::over_view`); a press on a window, the recovery dialog's,
/// stays that window's. egui's `Modal` blocks presses the same way; its modal layer alone only
/// holds the keyboard.
fn backdrop(ctx: &egui::Context) -> bool {
    let area = egui::Area::new(egui::Id::new(BACKDROP))
        .order(egui::Order::Background)
        .fixed_pos(egui::Pos2::ZERO);
    let layer = area.layer();
    area.show(ctx, |ui| {
        ui.allocate_rect(ui.ctx().content_rect(), egui::Sense::click_and_drag())
    });
    let pressed = ctx.input(|i| (i.pointer.press_origin()).filter(|_| i.pointer.any_pressed()));
    pressed.is_some_and(|at| ctx.layer_id_at(at) == Some(layer))
}

/// The folder `path` is in, ending in a separator; `None` for a bare name.
fn folder_of(path: &str) -> Option<String> {
    let dir = std::path::Path::new(path).parent()?;
    let dir = dir.to_str().filter(|d| !d.is_empty())?;
    Some(format!("{dir}{}", std::path::MAIN_SEPARATOR))
}

/// The required arguments of `info` that `args` lacks.
fn missing(info: &CommandInfo, args: &Value) -> Vec<String> {
    let required = info.args_schema["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    (required.iter())
        .filter_map(Value::as_str)
        .filter(|name| args.get(name).is_none())
        .map(String::from)
        .collect()
}
