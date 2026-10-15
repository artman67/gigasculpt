//! Bind a key as one gesture (review 0009, C11; decision 0025, section 5, amended 2026-10-14).
//! Settings > Keys' Bind a key… opens the palette's list under "Pick what the key runs"; once a
//! row is picked, the palette says "Press the keys for" it, takes the next key pressed with the
//! modifiers held then, shows it, and Bind binds it through `ui.keymap`, as a Ctrl+Alt press on
//! a control does (`keys/catch.rs`). A taken chord is `ui.keymap`'s refusal, which asks Replace.
//! Enter is the chord until one is pressed, then Bind's; Escape shuts it all. It takes no key
//! while anything shows in front of the palette. Typing the key's name stays `ui.keymap`'s, for
//! agents.

use egui::{Event, Key, RichText};
use gs_engine::PaletteRow;
use gs_protocol::{Catalog, Step};
use serde_json::{Map, Value, json};

use super::form::{Answer, plain, spec};
use super::{Palette, missing};
use crate::keymap::{Chord, binding};
use crate::pieces::{Face, Now, Press, label, pressed_as};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::shell::escape::{To, in_front};

/// Bind a key…, open: what the key will run once picked, and the keys pressed.
#[derive(Clone, Debug, Default)]
pub struct Bind {
    /// What the key runs: its title, and its step.
    runs: Option<(String, Step)>,
    /// The keys pressed, as the keymap writes them.
    keys: Option<String>,
}

impl Palette {
    /// Opens Bind a key… for the command `args` name (with their `args`), else asking which
    /// first.
    pub(crate) fn bind_key(&mut self, args: &Map<String, Value>) {
        let runs = args.get("command").and_then(Value::as_str).map(|command| {
            let given = args.get("args").cloned().unwrap_or_else(|| json!({}));
            let title = spec(command).map_or(command, |s| s.title);
            (title.to_string(), with_args(command, given))
        });
        self.set(true, Some(""));
        self.form = None;
        self.focus = true;
        self.binding = Some(Bind { runs, keys: None });
    }

    /// Draws Bind a key…: the list under "Pick what the key runs" until a row is picked, then
    /// the keys pressed with Bind and Cancel. Gives Bind's `ui.keymap` step, or Cancel.
    pub(super) fn bind_panel(
        &mut self,
        ui: &mut egui::Ui,
        (rows, catalog): (&[PaletteRow], &Catalog),
        enter: bool,
        (now, rec): (&Now, &mut Recorder),
    ) -> Option<Answer> {
        let runs = self.binding.as_ref().map(|b| b.runs.clone())?;
        let Some((title, runs)) = runs else {
            label(ui, RichText::new("Pick what the key runs").strong(), rec);
            let picked = self.contents(ui, rows, catalog, enter, rec)?;
            let title = titled(&picked, rows);
            let bind = self.binding.as_mut()?;
            bind.runs = Some((title, asking(picked, catalog)));
            return None;
        };
        let bind = self.binding.as_mut()?;
        // A key is the panel's only while the palette is in front (`shell/escape.rs`): not with
        // a menu, a popup, the quick menu, the wheel or another window over it.
        let pressed = ui.input(|i| i.events.iter().rev().find_map(chord));
        let pressed = pressed.filter(|_| in_front(ui.ctx()) == Some(To::Palette));
        let bound = bind.keys.is_some() && pressed.as_deref() == Some("Enter");
        if !bound && pressed.is_some() {
            bind.keys = pressed;
        }
        let line = format!("Press the keys for {title}");
        label(ui, RichText::new(line).strong(), rec);
        let keys = bind.keys.clone();
        let shown = keys.as_deref().unwrap_or("Waiting for a key…");
        label(ui, RichText::new(shown).monospace(), rec);
        label(ui, RichText::new(crate::keys::BY_CLICK).weak().small(), rec);
        let mut answer = None;
        ui.horizontal(|ui| {
            if let Some(step) = keys.map(|k| binding(&k, &runs)) {
                let bind = Press {
                    key: Some("Enter"),
                    ..plain("Bind", step.clone())
                };
                let clicked = pressed_as(ui, bind, Face::Label, now, rec).clicked();
                if clicked || bound {
                    answer = Some(Answer::Run(step));
                }
            } else {
                let why = "Press the keys first";
                let gray = ui.add_enabled(false, egui::Button::new("Bind"));
                let gray = gray.on_disabled_hover_text(why);
                rec.widget(Widget::plain("Bind", Role::Button).disabled(why), gray.rect);
            }
            let cancel = plain("Cancel", with_args("ui.palette", json!({ "open": false })));
            if pressed_as(ui, cancel, Face::Label, now, rec).clicked() {
                answer = Some(Answer::Cancel);
            }
        });
        answer
    }
}

/// A key pressed, with the modifiers held then, as the keymap writes it; not Escape, which
/// shuts the palette.
fn chord(e: &Event) -> Option<String> {
    match e {
        Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if *key != Key::Escape => Some(Chord::pressed(*key, *modifiers).to_string()),
        _ => None,
    }
}

/// What `picked` is called: the label of a control among `rows` that sends it, else its
/// command's title, else the command's name.
fn titled(picked: &Step, rows: &[PaletteRow]) -> String {
    let control = rows
        .iter()
        .find(|r| r.command == *picked && r.label != picked.command);
    let title = || spec(&picked.command).map_or(picked.command.as_str(), |s| s.title);
    control.map_or_else(|| title().to_string(), |r| r.label.clone())
}

/// What a key bound to `picked` runs: the step itself, or its form when it lacks arguments or
/// asks about unsaved changes first, as Ctrl+O opens Open's and Ctrl+N asks before New.
fn asking(picked: Step, catalog: &Catalog) -> Step {
    let info = catalog.commands.iter().find(|c| c.name == picked.command);
    let lacks = info.is_some_and(|i| !missing(i, &picked.args).is_empty());
    if !lacks && !super::guards(&picked.command) {
        return picked;
    }
    let mut args = json!({ "form": picked.command });
    if picked.args.as_object().is_some_and(|a| !a.is_empty()) {
        args["args"] = picked.args;
    }
    with_args("ui.palette", args)
}
