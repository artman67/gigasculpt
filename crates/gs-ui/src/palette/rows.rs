//! The palette's rows: every command and control matching what is typed, ranked as a person
//! looks for them, drawn under the search field with the selected row's details.

use std::collections::{BTreeMap, BTreeSet};

use egui::RichText;
use gs_engine::PaletteRow;
use gs_protocol::{Catalog, CommandInfo, Step};

use super::Palette;
use crate::fuzzy;
use crate::keymap::Keymap;
use crate::record::{Recorder, Role, Widget, step};

/// Rows of each kind shown at most.
const ROWS: usize = 20;

/// Where a row stands among the palette's rows, first first (review 0009, C12): what a person
/// presses leads, and a command's name, the id agents type, leads only for a query with a dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// A command by its name, for a query with a dot.
    Id,
    /// A control, by its label.
    Control,
    /// A command by its title.
    Titled,
    /// A command by its name.
    Named,
    /// A command whose summary alone mentions the query.
    Summary,
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
    /// The commands matching `needle`, the best [`ROWS`] by rank, then score, each once with the
    /// label it shows (review 0009, C12): by title, but those a control already lists (one of
    /// the same label, or one running the command with no arguments); else by name, the ids
    /// agents type; else by summary alone.
    fn commands<'c>(
        &self,
        catalog: &'c Catalog,
        listed: &Listed,
        needle: &str,
    ) -> Vec<(Rank, String, &'c CommandInfo)> {
        let titles: BTreeMap<&str, &str> = (gs_engine::registry().into_iter())
            .map(|s| (s.name, s.title))
            .collect();
        let bare = |s: &Step| s.args.as_object().is_none_or(serde_json::Map::is_empty);
        let run: BTreeSet<&str> = (listed.controls.iter())
            .filter(|(_, s)| bare(s))
            .map(|(_, s)| s.command.as_str())
            .collect();
        let labels: BTreeSet<String> = (listed.controls.iter())
            .map(|(label, _)| label.to_lowercase())
            .collect();
        let named = if needle.contains('.') {
            Rank::Id
        } else {
            Rank::Named
        };
        let mut found = Vec::new();
        for info in &catalog.commands {
            let title = (titles.get(info.name.as_str()).copied()).filter(|t| {
                !t.is_empty()
                    && !run.contains(info.name.as_str())
                    && !labels.contains(&t.to_lowercase())
            });
            let by_title = title.and_then(|t| Some((Rank::Titled, fuzzy::score(needle, t)?, t)));
            let by_name = || Some((named, fuzzy::score(needle, &info.name)?, info.name.as_str()));
            let by_summary = || {
                let found = info.summary.to_lowercase().contains(needle);
                found.then_some((Rank::Summary, 0, info.name.as_str()))
            };
            let ranked = by_title.or_else(by_name).or_else(by_summary);
            found.extend(ranked.map(|(rank, score, label)| (rank, score, label.to_string(), info)));
        }
        let age = |info: &CommandInfo| self.age(&step(&info.name));
        // With nothing typed, recent picks first, so none is cut off.
        let recent = needle.is_empty();
        found.sort_by(|a, b| {
            let ranked = || (a.0.cmp(&b.0)).then_with(|| b.1.cmp(&a.1));
            let aged = || age(a.3).cmp(&age(b.3));
            let first = if recent {
                aged().then_with(ranked)
            } else {
                ranked().then_with(aged)
            };
            first.then_with(|| a.3.name.cmp(&b.3.name))
        });
        let best = found.into_iter().take(ROWS);
        best.map(|(rank, _, label, info)| (rank, label, info))
            .collect()
    }

    /// Every row, each with its key and where it lives, as a person looks for them (review
    /// 0009, C12): the matching controls by their labels, then the commands by their titles,
    /// then by their names (the ids agents type, which lead for a query with a dot), then those
    /// whose summary alone mentions the query (so `clay` finds the Clay brush before a command
    /// that lights clay). With nothing typed, recent picks first.
    pub fn rows(&self, catalog: &Catalog, listed: &Listed) -> Vec<PaletteRow> {
        let places = |pick: &dyn Fn(&Step) -> bool| -> Vec<String> {
            let found = listed.places.iter().filter(|(s, _)| pick(s));
            found.map(|(_, place)| place.clone()).take(3).collect()
        };
        let command = |(rank, label, info): (Rank, String, &CommandInfo)| {
            let row = PaletteRow {
                label,
                command: step(&info.name),
                key: listed.keymap.key_for(&info.name).map(Into::into),
                places: places(&|s: &Step| s.command == info.name),
            };
            (rank, row)
        };
        let needle = self.query.trim().to_lowercase();
        let mut controls: Vec<(i32, &(String, Step))> = (listed.controls.iter())
            .filter_map(|c| Some((fuzzy::score(&needle, &c.0.to_lowercase())?, c)))
            .collect();
        // Recent picks first among equals, so they are not cut off before the sort below.
        controls.sort_by_key(|c| (std::cmp::Reverse(c.0), self.age(&c.1.1)));
        let controls = controls.into_iter().take(ROWS).map(|(_, (label, step))| {
            let args = serde_json::from_value(step.args.clone()).unwrap_or_default();
            let row = PaletteRow {
                label: label.clone(),
                command: step.clone(),
                key: listed
                    .keymap
                    .key_for_args(&step.command, &args)
                    .map(Into::into),
                places: places(&|s: &Step| s == step),
            };
            (Rank::Control, row)
        });
        let commands = self.commands(catalog, listed, &needle).into_iter();
        let mut rows: Vec<(Rank, PaletteRow)> = controls.chain(commands.map(command)).collect();
        rows.sort_by_key(|(rank, _)| *rank);
        let mut rows: Vec<PaletteRow> = rows.into_iter().map(|(_, row)| row).collect();
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

    /// The search field, the rows and the selected command's docs. Returns the step of the row
    /// clicked, or of the selected one on Enter.
    pub(super) fn contents(
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
            Some(row) => details(ui, &row.command.command, catalog, rec),
            None => {
                ui.label("No command matches.");
            }
        }
        clicked.or_else(|| enter.then(|| rows.get(self.selected).map(|r| r.command.clone()))?)
    }
}

/// Details, folded until opened: the selected row's command name and its docs, which are
/// written for agents (review 0009, C12). egui's memory keeps whether it is open.
fn details(ui: &mut egui::Ui, command: &str, catalog: &Catalog, rec: &mut Recorder) {
    ui.separator();
    let id = egui::Id::new(("palette", "details"));
    let was = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let header = ui.selectable_label(was, RichText::new("Details").weak());
    rec.section("details", "Details", !was, false, header.rect);
    let open = was != header.clicked();
    ui.ctx().data_mut(|d| d.insert_temp(id, open));
    if !open {
        return;
    }
    let info = catalog.commands.iter().find(|c| c.name == command);
    let docs = info.map_or("", |i| i.docs.as_str());
    for said in [RichText::new(command).monospace(), RichText::new(docs)] {
        let text = said.text().to_string();
        let shown = ui.label(said);
        rec.widget(Widget::plain(text, Role::Label), shown.rect);
    }
}
