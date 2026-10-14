//! The command palette: one key opens it, typing narrows every command in the catalog and every
//! control the layouts know, and each row shows its key and where it lives in the layout shown.
//! With nothing typed, what was picked last comes first. Picking a row runs it; a command that
//! needs arguments asks for them in a form first, each drawn by its kind (see [`form`]). A scene
//! file form's `path` starts in the folder that fits it (decision 0016, section 17).

mod form;

use std::collections::BTreeMap;

use egui::{Align2, Key, RichText};
use gs_engine::PaletteRow;
use gs_protocol::{Catalog, CommandInfo, Step};
use serde_json::Value;

use crate::direction::{PaletteSpec, Place};
use crate::fuzzy;
use crate::keymap::Keymap;
use crate::pieces::Now;
use crate::record::{Recorder, Role, Widget, step};
use form::Answer;
pub use form::Form;

/// Rows of each kind shown at most.
const ROWS: usize = 20;
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
    /// The scene's own file, as the engine last reported it (`file.path`).
    pub own: Option<String>,
    /// The folder of the newest file each kind of form ran with (see [`Palette::folder`]).
    folders: BTreeMap<&'static str, String>,
}

/// What the palette may list besides the catalog: every control, and where each lives.
pub struct Listed<'a> {
    pub keymap: &'a Keymap,
    /// Every control the layouts know: label and step.
    pub controls: Vec<(String, Step)>,
    /// Where controls of the layout shown live.
    pub places: Vec<(Step, String)>,
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
        }
        if let Some(query) = query {
            self.query = query.to_string();
            self.selected = 0;
        }
    }

    /// Opens it asking for the required arguments `step` lacks, and the optional ones named in
    /// `also` it lacks. An error if no command has that name.
    pub fn ask(&mut self, step: Step, catalog: &Catalog, also: &[String]) -> Result<(), String> {
        let info = (catalog.commands.iter()).find(|c| c.name == step.command);
        let info = info.ok_or_else(|| format!("no command called {}", step.command))?;
        let mut names = missing(info, &step.args);
        for name in also {
            if step.args.get(name.as_str()).is_none() && !names.contains(name) {
                names.push(name.clone());
            }
        }
        let start = |name: &str| match name {
            "path" => self.folder(&step.command).unwrap_or_default(),
            _ => String::new(),
        };
        let fields = (names.into_iter())
            .map(|name| {
                let text = start(&name);
                (name, text)
            })
            .collect();
        self.set(true, None);
        self.focus = true;
        self.form = Some(Form::new(step, info, fields));
        Ok(())
    }

    /// Opens it asking for `asked`, the argument an engine's refusal of `step` names: a boolean
    /// so asked is a question with two buttons (decision 0025, rule 13).
    pub fn refused(&mut self, step: Step, catalog: &Catalog, asked: String) -> Result<(), String> {
        self.ask(step, catalog, &[asked])?;
        if let Some(form) = &mut self.form {
            form.refused = true;
        }
        Ok(())
    }

    /// The folder a form's `path` starts in, ending in a separator so a name typed after it
    /// makes the path (decision 0016, section 17): for Open and Save, the scene's own file's,
    /// else the newest scene file's opened or saved through a form; for Import and Export,
    /// the newest file's that command ran with, else the scene's; for Save brush and Load brush,
    /// the newest brush file's. Kept for the launch only.
    fn folder(&self, command: &str) -> Option<String> {
        let own = self.own.as_deref().and_then(folder_of);
        let scene = || own.clone().or_else(|| self.folders.get(SCENE).cloned());
        match command {
            "file.open" | "file.save" => scene(),
            "file.import" | "file.export" => self.folders.get(command).cloned().or_else(scene),
            "brush.save" | "brush.load" => self.folders.get("brush").cloned(),
            _ => None,
        }
    }

    /// Notes the folder of the file `step` runs with, for the next form (see `folder`).
    fn note(&mut self, step: &Step) {
        let kind = match step.command.as_str() {
            "file.open" | "file.save" => SCENE,
            "file.import" => "file.import",
            "file.export" => "file.export",
            "brush.save" | "brush.load" => "brush",
            _ => return,
        };
        if let Some(dir) = step.args["path"].as_str().and_then(folder_of) {
            self.folders.insert(kind, dir);
        }
    }

    /// The commands matching the query, best first: name matches by score, then commands whose
    /// summary contains the query.
    pub fn matches<'c>(&self, catalog: &'c Catalog) -> Vec<&'c CommandInfo> {
        let needle = self.query.trim().to_lowercase();
        let mut scored: Vec<(i32, &CommandInfo)> = catalog
            .commands
            .iter()
            .filter_map(|info| {
                let by_summary = || {
                    let found = info.summary.to_lowercase().contains(&needle);
                    found.then_some(i32::MIN)
                };
                let score = fuzzy::score(&needle, &info.name).or_else(by_summary)?;
                Some((score, info))
            })
            .collect();
        let age = |info: &CommandInfo| self.age(&step(&info.name));
        scored.sort_by(|a, b| {
            (b.0.cmp(&a.0))
                .then_with(|| age(a.1).cmp(&age(b.1)))
                .then_with(|| a.1.name.cmp(&b.1.name))
        });
        scored
            .into_iter()
            .take(ROWS)
            .map(|(_, info)| info)
            .collect()
    }

    /// Every row, each with its key and where it lives: the commands whose name matches, then
    /// the matching controls, then the commands whose summary only mentions the query (so
    /// `clay` finds the Clay brush before a command that lights clay). With nothing typed,
    /// recent picks first.
    pub fn rows(&self, catalog: &Catalog, listed: &Listed) -> Vec<PaletteRow> {
        let places = |pick: &dyn Fn(&Step) -> bool| -> Vec<String> {
            let found = listed.places.iter().filter(|(s, _)| pick(s));
            found.map(|(_, place)| place.clone()).take(3).collect()
        };
        let command = |info: &CommandInfo| PaletteRow {
            label: info.name.clone(),
            command: step(&info.name),
            key: listed.keymap.key_for(&info.name).map(Into::into),
            places: places(&|s: &Step| s.command == info.name),
        };
        let needle = self.query.trim().to_lowercase();
        let (named, described): (Vec<_>, Vec<_>) = (self.matches(catalog).into_iter())
            .partition(|info| fuzzy::score(&needle, &info.name).is_some());
        let mut controls: Vec<(i32, &(String, Step))> = (listed.controls.iter())
            .filter_map(|c| Some((fuzzy::score(&needle, &c.0.to_lowercase())?, c)))
            .collect();
        // Recent picks first among equals, so they are not cut off before the sort below.
        controls.sort_by_key(|c| (std::cmp::Reverse(c.0), self.age(&c.1.1)));
        let controls = controls.into_iter().take(ROWS).map(|(_, (label, step))| {
            let args = serde_json::from_value(step.args.clone()).unwrap_or_default();
            PaletteRow {
                label: label.clone(),
                command: step.clone(),
                key: listed
                    .keymap
                    .key_for_args(&step.command, &args)
                    .map(Into::into),
                places: places(&|s: &Step| s == step),
            }
        });
        let named = named.into_iter().map(command);
        let described = described.into_iter().map(command);
        let mut rows: Vec<PaletteRow> = named.chain(controls).chain(described).collect();
        if needle.is_empty() {
            // Stable: recent picks first, newest first, the rest as they were.
            rows.sort_by_key(|r| self.age(&r.command));
        }
        rows
    }

    /// How many picks ago `step` was picked: 0 for the last; never picked, last of all.
    fn age(&self, step: &Step) -> usize {
        let found = self.recent.iter().position(|s| s == step);
        found.unwrap_or(usize::MAX)
    }

    /// Remembers a pick.
    fn remember(&mut self, step: &Step) {
        self.recent.retain(|s| s != step);
        self.recent.insert(0, step.clone());
        self.recent.truncate(RECENT);
    }

    /// Draws the palette if it is open, placed as `spec` says. Returns the command picked,
    /// which closes it; one that lacks arguments opens its form.
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
        let (up, down, enter, escape) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        if escape {
            self.set(false, None);
            return None;
        }
        // Moving past the last row is undone where the rows are drawn (`contents`).
        if down {
            self.selected += 1;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let window = egui::Window::new("Commands")
            .id(egui::Id::new("palette"))
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
        let mut chosen = None;
        window.show(ui.ctx(), |ui| {
            rec.panel("palette", "Command palette", ui.max_rect());
            chosen = match &mut self.form {
                Some(form) => form::draw(ui, form, &mut self.focus, enter, now, rec),
                None => self
                    .contents(ui, &rows, catalog, enter, rec)
                    .map(Answer::Run),
            };
        });
        match chosen? {
            Answer::Run(step) => self.pick(step, catalog),
            Answer::Cancel => {
                self.set(false, None);
                None
            }
        }
    }

    /// Runs `step`, or asks for the arguments it lacks.
    fn pick(&mut self, step: Step, catalog: &Catalog) -> Option<Step> {
        let info = catalog.commands.iter().find(|c| c.name == step.command);
        if self.form.is_none() && info.is_some_and(|i| !missing(i, &step.args).is_empty()) {
            self.ask(step, catalog, &[]).ok();
            return None;
        }
        self.remember(&step);
        self.note(&step);
        self.set(false, None);
        Some(step)
    }

    /// The search field, the rows and the selected command's docs. Returns the step of the row
    /// clicked, or of the selected one on Enter.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[PaletteRow],
        catalog: &Catalog,
        enter: bool,
        rec: &mut Recorder,
    ) -> Option<Step> {
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search commands and controls")
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
            field.request_focus();
        }
        if field.changed() {
            self.selected = 0;
        }
        rec.widget(
            Widget::plain("Search commands", Role::TextField),
            field.rect,
        );
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let mut clicked = None;
        for (i, row) in rows.iter().enumerate() {
            let text = match &row.key {
                Some(key) => format!("{}    [{key}]", row.label),
                None => row.label.clone(),
            };
            let highlighted = i == self.selected;
            let shown = ui.selectable_label(highlighted, RichText::new(text).strong());
            let info = catalog
                .commands
                .iter()
                .find(|c| c.name == row.command.command);
            let about = info.map_or("", |i| i.summary.as_str());
            let line = match row.places.as_slice() {
                [] => about.to_string(),
                places => format!("In {}.  {about}", places.join(", ")),
            };
            ui.label(RichText::new(line).weak().small());
            let widget = Widget::runs(
                &row.label,
                Role::ListItem,
                row.command.clone(),
                row.key.as_deref(),
            );
            rec.widget(widget.selected(highlighted), shown.rect);
            if shown.clicked() {
                clicked = Some(row.command.clone());
            }
        }
        match rows.get(self.selected) {
            Some(row) => {
                ui.separator();
                let info = catalog
                    .commands
                    .iter()
                    .find(|c| c.name == row.command.command);
                let docs = info.map_or("", |i| i.docs.as_str());
                let shown = ui.label(docs);
                rec.widget(Widget::plain(docs, Role::Label), shown.rect);
            }
            None => {
                ui.label("No command matches.");
            }
        }
        clicked.or_else(|| enter.then(|| rows.get(self.selected).map(|r| r.command.clone()))?)
    }
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
