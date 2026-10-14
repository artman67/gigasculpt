//! Open a recent scene's list (decision 0016, section 17, item 7): the button's options, a row
//! to each scene in the session's `file.recent`, newest first. A row shows the scene's name with
//! its folder's name dimmed and says its path on hover; a pick runs the `recent` row set's
//! control with `"@row"` as the scene's place in the list (`file.open` with `recent`). A file
//! moved or deleted since shows dimmed and cannot be picked.

use egui::RichText;
use gs_protocol::Step;
use serde_json::Value;

use crate::pieces::{Now, label};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// The row set a pick runs, which the button names as its options.
pub const SET: &str = "recent";

/// The list's rows, in the menu `ui`. Returns the step of the row picked.
pub fn rows(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let files = now.state.get("file.recent").and_then(Value::as_array);
    let empty = match files {
        None => Some("No recent scenes: autosave is off, and the list is kept with it."),
        Some(files) if files.is_empty() => Some("No recent scenes yet."),
        Some(_) => None,
    };
    if let Some(empty) = empty {
        label(ui, RichText::new(empty).weak(), rec);
        return None;
    }
    let (_, picked) = now.directions.controls.piece(SET)?;
    let c = picked.first()?;
    let (command, args) = (c.command.as_deref()?, serde_json::to_value(&c.args).ok()?);
    let mut asked = None;
    for file in files.into_iter().flatten() {
        let step = with_args(command, filled(&args, &file["recent"]));
        asked = row(ui, file, step, rec).or(asked);
    }
    asked
}

/// One scene's row: its name, its folder's name dimmed, its path on hover; dimmed, and not
/// picked by a tap, while its file is gone. Returns `step` when picked.
fn row(ui: &mut egui::Ui, file: &Value, step: Step, rec: &mut Recorder) -> Option<Step> {
    let text = |key: &str| file[key].as_str().unwrap_or_default();
    let (name, path) = (text("name"), text("path"));
    let gone = file["gone"].as_bool().unwrap_or(false);
    let colors = &theme(ui.ctx()).colors;
    // The theme fades nothing that is disabled: a gone file draws its own text_off.
    let shade = |t: RichText| if gone { t.color(colors.text_off) } else { t };
    let folder = RichText::new(text("folder")).color(colors.text_dim);
    let button = egui::Button::new(shade(RichText::new(name))).right_text(shade(folder));
    let shown = ui.add_enabled(!gone, button);
    let widget = Widget::runs(name, Role::Button, step.clone(), None);
    let (shown, widget) = if gone {
        let why = format!("{path} was moved or deleted");
        (shown.on_disabled_hover_text(&why), widget.disabled(why))
    } else {
        (shown.on_hover_text(path), widget)
    };
    rec.widget(widget, shown.rect);
    rec.hint(path);
    shown.clicked().then_some(step)
}

/// `args` with every `"@row"` in it as `place`.
fn filled(args: &Value, place: &Value) -> Value {
    match args {
        Value::String(s) if s == "@row" => place.clone(),
        Value::Array(items) => items.iter().map(|v| filled(v, place)).collect(),
        Value::Object(fields) => (fields.iter())
            .map(|(k, v)| (k.clone(), filled(v, place)))
            .collect(),
        other => other.clone(),
    }
}
