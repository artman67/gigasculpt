//! The keys cheat sheet: every key, with what it does, and the buttons that bind a key and put
//! keys back as built (decision 0025, section 5), Settings' Keys page (decision 0060, section 7).

use egui::RichText;
use gs_protocol::Step;
use serde_json::json;

use super::buttons::{Face, Press, control, pressed_as};
use super::{Now, all_controls, controls_of, label};
use crate::direction::Control;
use crate::gizmo::InUse;
use crate::held::Held;
use crate::keymap::{Binding, flipped, inline};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::values::Value;
use crate::{descriptions, text};

/// The `keys` set's Bind a key…, and Restore defaults while anything is changed; a line saying
/// Ctrl+Alt + click binds a key too; each change with its own Restore; then every key, under
/// their title when `titled`, with what it does beside it, or under it in a room too narrow for
/// that ([`stacked`]): what each held modifier does, then each binding with what it runs, by the
/// label of the control that runs the same, under its context's heading (`General keys`, then
/// View, Sculpt, Mask, Geometry and Your keys). Returns the command of the button pressed.
pub(super) fn keys(ui: &mut egui::Ui, now: &Now, titled: bool, rec: &mut Recorder) -> Option<Step> {
    if titled {
        label(ui, RichText::new("Keys").strong(), rec);
        ui.end_row();
    }
    let mods = [
        egui::Modifiers::SHIFT,
        egui::Modifiers::CTRL,
        egui::Modifiers::SHIFT | egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
    ];
    let gizmo = now.gizmo.map(|_| InUse::default());
    let of = |m| Held::of(m, gizmo, now.preset, &now.brush.slots);
    let held: Vec<(String, String)> = (mods.into_iter().filter_map(of))
        .map(|h| {
            let text = h.text();
            let (keys, action) = text.split_once(": ").unwrap_or_default();
            (format!("{keys} held"), action.to_string())
        })
        .collect();
    let contexts = now.keymap.by_context();
    let titled = |context: &str| format!("{context} keys");
    let every = (held.iter().map(|(key, _)| key.clone()))
        .chain(contexts.iter().map(|(context, _)| titled(context)))
        .chain(
            contexts
                .iter()
                .flat_map(|(_, b)| b.iter().map(|b| b.key.clone())),
        )
        .chain(now.keymap.changes.iter().map(|c| c.key.clone()));
    let stacked = stacked(ui, every);
    let asked = changes(ui, now, stacked, rec);
    let all = all_controls(now);
    let grid = egui::Grid::new("keys").num_columns(if stacked { 1 } else { 2 });
    grid.striped(true).show(ui, |ui| {
        for (key, action) in &held {
            line(ui, (key, action), stacked, rec, |_, _| {});
            ui.end_row();
        }
        for (context, bindings) in contexts {
            heading(ui, titled(context), stacked, rec);
            ui.end_row();
            for b in bindings {
                line(ui, (&b.key, &action(b, &all)), stacked, rec, |_, _| {});
                ui.end_row();
            }
        }
    });
    asked
}

/// Whether each key's action goes under it rather than beside it: when the room beside the
/// widest key or heading, which share the first column, is narrower than that, as in a tray a
/// narrow window shrinks (decision 0025, rule 12).
fn stacked(ui: &egui::Ui, firsts: impl Iterator<Item = String>) -> bool {
    let wide =
        |first: String| text::width(ui, RichText::new(first).strong(), egui::TextStyle::Body);
    let first = firsts.map(wide).fold(0.0, f32::max);
    ui.available_width() - first - ui.spacing().item_spacing.x < first
}

/// A context's heading over its keys, wrapped when `stacked`: a grid's cell keeps its text on
/// one line, and `Geometry keys` is wider than a tray a narrow window shrinks.
fn heading(ui: &mut egui::Ui, text: String, stacked: bool, rec: &mut Recorder) {
    let heading = egui::Label::new(RichText::new(&text).strong());
    let shown = ui.add(if stacked { heading.wrap() } else { heading });
    rec.widget(Widget::plain(text, Role::Label), shown.rect);
}

/// The `keys` set's buttons (Bind a key…) and the one that puts every key back, then each of
/// the user's changes with what it runs and a button that puts that key back, the action under
/// the key when `stacked`. Returns the command of the one pressed.
fn changes(ui: &mut egui::Ui, now: &Now, stacked: bool, rec: &mut Recorder) -> Option<Step> {
    let changed = &now.keymap.changes;
    let mut asked = None;
    ui.horizontal_wrapped(|ui| {
        for c in controls_of("keys", now)
            .iter()
            .filter(|c| c.shown(&now.state))
        {
            asked = control(ui, c, false, now, rec).or(asked.take());
        }
        if !changed.is_empty() {
            let hint = "Puts every key back as built and empties your keymap file.";
            let all = (json!({ "restore_defaults": true }), hint);
            asked = asked
                .take()
                .or(restore(ui, "Restore defaults", all, now, rec));
        }
    });
    ui.end_row();
    label(ui, RichText::new(crate::keys::BY_CLICK).weak().small(), rec);
    ui.end_row();
    if changed.is_empty() {
        return asked;
    }
    label(ui, RichText::new("Your changes").strong(), rec);
    ui.end_row();
    let all = all_controls(now);
    let grid = egui::Grid::new("key changes").num_columns(if stacked { 1 } else { 2 });
    grid.show(ui, |ui| {
        for c in changed {
            let does = if c.command.is_empty() {
                "unbound".into()
            } else {
                action(c, &all)
            };
            let name = format!("Restore {}", c.key);
            let hint = format!("Puts {} back as built.", c.key);
            let one = (json!({ "restore": c.key }), hint.as_str());
            line(ui, (&c.key, &does), stacked, rec, |ui, rec| {
                asked = asked.take().or(restore(ui, &name, one, now, rec));
            });
            ui.end_row();
        }
    });
    asked
}

/// A button labelled `label` that runs `ui.keymap` with `args`, saying `hint` on hover (decision
/// 0060, section 8's rule 8). Returns its command when pressed.
fn restore(
    ui: &mut egui::Ui,
    label: &str,
    (args, hint): (serde_json::Value, &str),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let step = with_args("ui.keymap", args);
    let p = Press {
        label,
        step: step.clone(),
        key: None,
        lit: false,
        shows: None,
    };
    let shown = pressed_as(ui, p, Face::Label, now, rec);
    rec.hint(hint);
    descriptions::hint(ui, &shown, hint);
    shown.clicked().then_some(step)
}

/// A key in bold, then what it does beside it, or under it when `stacked`, wrapped in the room
/// left to the panel's edge (decision 0025, rule 12), recorded as one label: `Ctrl+Z: Undo`;
/// then `then` (a change's Restore) after what it does. The wrapped lines start at the row's
/// top: a grid centres a cell on the row's height, and lines grown down from its middle run
/// into the next row.
fn line(
    ui: &mut egui::Ui,
    (key, action): (&str, &str),
    stacked: bool,
    rec: &mut Recorder,
    then: impl FnOnce(&mut egui::Ui, &mut Recorder),
) {
    let beside = (!stacked).then(|| ui.strong(key));
    let top = egui::UiBuilder::new().layout(egui::Layout::top_down(egui::Align::Min));
    ui.scope_builder(top, |ui| {
        let shown = beside.unwrap_or_else(|| ui.strong(key));
        ui.horizontal_wrapped(|ui| {
            // egui rounds the wrap width to a whole point and wraps half a point past it, and a
            // wrapping row allocates each line, trailing space and all: a point less keeps every
            // line inside the room (`F9: ui.adjust open = true` ran 0.44 past a tray's edge).
            ui.set_max_width((ui.available_width() - 1.0).max(0.0));
            let does = ui.add(egui::Label::new(action).wrap());
            let line = Widget::plain(format!("{key}: {action}"), Role::Label);
            rec.widget(line, shown.rect.union(does.rect));
            then(ui, rec);
        })
    });
}

/// What a binding does, in words: holding a key that sets a value while the pointer moves, a
/// key held down for its control, the label of the control that runs the same command (a
/// switch's flipped, as `Keymap::key_for_args` matches it), a built-in key's own title
/// ([`Binding::title`]: `[` reads Size down, not its arguments), or else [`titled`].
fn action(b: &Binding, all: &[Control]) -> String {
    let marked = (b.args.iter()).find(|(_, v)| matches!(v.as_str(), Some("drag" | "held")));
    let mut args = b.plain_args();
    if let Some((arg, mark)) = marked {
        if mark.as_str() == Some("drag") {
            let value = Value::from_arg(arg).map_or_else(|| arg.clone(), |v| v.title().into());
            return format!("hold and move: {value}");
        }
        args.insert(arg.clone(), true.into());
    }
    let same = |c: &&Control| {
        c.command.as_ref() == Some(&b.command) && (c.args == args || flipped(&c.args) == args)
    };
    match (all.iter().find(same), marked) {
        (Some(c), Some(_)) => format!("hold: {}", c.label),
        (Some(c), None) => c.label.clone(),
        (None, _) => b
            .title()
            .map_or_else(|| titled(&b.command, b.plain_args()), String::from),
    }
}

/// A binding no control runs, by a title the palette shows (decision 0062, row 818): a brush by
/// its name; a key that opens a command's form by that command's title and "…", as its Run
/// reads it; else its command's title. Any other arguments it gives follow in brackets.
fn titled(command: &str, mut args: toml::Table) -> String {
    if let (1, Some(brush)) = (args.len(), args.get("brush").and_then(toml::Value::as_str)) {
        return brush.to_string();
    }
    let form = (command == "ui.palette")
        .then(|| args.remove("form"))
        .flatten();
    let named = form
        .as_ref()
        .and_then(toml::Value::as_str)
        .unwrap_or(command);
    let specs = gs_engine::registry();
    let title = (specs.iter().find(|s| s.name == named)).map_or(named, |s| s.title);
    let title = if form.is_some() {
        format!("{title}…")
    } else {
        title.to_string()
    };
    match inline("", &args) {
        rest if rest.is_empty() => title,
        rest => format!("{title} ({})", rest.trim()),
    }
}
