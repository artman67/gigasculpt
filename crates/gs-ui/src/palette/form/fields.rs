//! The form's fields: each argument drawn by its kind (decision 0025, rule 13), a row of words,
//! a switch, a slider, a pick of parts, a color, a field or three to type in.

use egui::RichText;
use serde_json::{Value, json};

use super::kind::{Kind, Word};
use super::words::{friendly, word_label};
use super::{Arg, Form, plain, pointer};
use crate::pieces::{Face, Now, Press, label, part_list, pressed_as, running};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// Draws the argument `name` by its kind (decision 0025, rule 13), under its name and hint
/// where its control does not carry its name. Returns whether a slider took a typed value.
pub(super) fn argument(
    ui: &mut egui::Ui,
    form: &mut Form,
    name: &str,
    focus: &mut bool,
    now: &Now,
    rec: &mut Recorder,
) -> bool {
    let Some(arg) = form.args.get(name).cloned() else {
        return false;
    };
    let title = friendly(name);
    let caption = |ui: &mut egui::Ui, rec: &mut Recorder| {
        ui.label(RichText::new(&title).strong());
        said(ui, &arg.hint, rec);
    };
    match &arg.kind {
        Kind::Switch => switch(ui, form, name, &arg, rec),
        Kind::Slider(ends, whole) => {
            return slider(ui, form, name, (&arg, *ends, *whole), now, rec);
        }
        Kind::Words(words, several) => {
            caption(ui, rec);
            row(ui, form, name, (words, *several), now, rec);
        }
        Kind::WordsOrNumber(words) => {
            caption(ui, rec);
            row(ui, form, name, (words, false), now, rec);
            typed(ui, form, name, focus, rec);
        }
        Kind::Part(several) => {
            caption(ui, rec);
            part(ui, form, name, (&arg, *several), now, rec);
        }
        Kind::Color => {
            caption(ui, rec);
            color(ui, form, name, rec);
        }
        Kind::Text | Kind::Typed | Kind::Numbers => {
            caption(ui, rec);
            typed(ui, form, name, focus, rec);
        }
        Kind::Three(_) => {
            caption(ui, rec);
            three(ui, form, name, focus, rec);
        }
        Kind::Given => {
            caption(ui, rec);
            let given = RichText::new("Given by a script or the window.")
                .small()
                .weak();
            label(ui, given, rec);
        }
    }
    false
}

/// An argument's hint as the line under its name, recorded as a label; nothing when it has none.
fn said(ui: &mut egui::Ui, hint: &str, rec: &mut Recorder) {
    if !hint.is_empty() {
        label(ui, RichText::new(hint).small().weak(), rec);
    }
}

/// `words` as a row of lit buttons, the one `name` shows lit (each it shows when `several`,
/// a tap adding or taking it away). A tap chooses, running nothing; each is recorded with the
/// step Run would send with it chosen, and says its word's doc on hover, else the argument's
/// hint.
fn row(
    ui: &mut egui::Ui,
    form: &mut Form,
    name: &str,
    (words, several): (&[Word], bool),
    now: &Now,
    rec: &mut Recorder,
) {
    let shows = form.shows(name);
    let hint = form.args.get(name).map(|a| a.hint.clone());
    let hint = hint.filter(|h| !h.is_empty());
    let axis = form.shows("axis");
    let axis = axis
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or("x")
        .to_uppercase();
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for word in words {
            let value = Value::from(word.value.as_str());
            let lit = match &shows {
                Some(Value::Array(all)) => all.contains(&value),
                shown => shown.as_ref() == Some(&value),
            };
            let chosen = if several {
                toggled(shows.as_ref(), &value)
            } else {
                value
            };
            let label = word_label(name, &word.value, &axis);
            let press = Press {
                lit,
                ..plain(&label, form.with(name, chosen.clone()))
            };
            let mut shown = pressed_as(ui, press, Face::Label, now, rec);
            if let Some(doc) = word.doc.as_ref().or(hint.as_ref()) {
                rec.hint(doc);
                shown = shown.on_hover_text(doc);
            }
            if shown.clicked() {
                picked = Some(chosen);
            }
        }
    });
    if let Some(value) = picked {
        form.chosen.insert(name.into(), value);
    }
}

/// A boolean: a switch, lit as the trays' switches while on, recorded as a toggle with the step
/// Run would send with it flipped.
fn switch(ui: &mut egui::Ui, form: &mut Form, name: &str, arg: &Arg, rec: &mut Recorder) {
    let on = form.shows(name) == Some(Value::Bool(true));
    let title = friendly(name);
    let colors = &theme(ui.ctx()).colors;
    let text = RichText::new(&title);
    let button = if on {
        egui::Button::new(text.color(colors.on_lit)).fill(colors.lit)
    } else {
        egui::Button::new(text)
    };
    let shown = ui.add(button.selected(on));
    let step = form.with(name, Value::Bool(!on));
    rec.widget(
        Widget::runs(&title, Role::Toggle, step, None).selected(on),
        shown.rect,
    );
    if !arg.hint.is_empty() {
        rec.hint(&arg.hint);
    }
    said(ui, &arg.hint, rec);
    if shown.clicked() {
        form.chosen.insert(name.into(), Value::Bool(!on));
    }
}

/// A number between `ends`: the window's slider (`running`), at what it shows or its low end,
/// reading "auto" there while it is an optional number nothing gives, which the engine settles
/// (decision 0062). A whole one reads with no decimals. A typed value is held to the ends, and
/// rounded when `whole`. Returns whether it took a value.
fn slider(
    ui: &mut egui::Ui,
    form: &mut Form,
    name: &str,
    (arg, ends, whole): (&Arg, [f64; 2], bool),
    now: &Now,
    rec: &mut Recorder,
) -> bool {
    let title = friendly(name);
    let shown = form.shows(name).and_then(|v| v.as_f64());
    let at = shown.unwrap_or(ends[0]);
    let held = |v: f32| {
        let v = f64::from(v).clamp(ends[0], ends[1]);
        if whole {
            json!(v.round() as i64)
        } else {
            json!(v)
        }
    };
    let step = |v: f32| form.with(name, held(v));
    let rail = [ends[0] as f32, ends[1] as f32];
    let word = (arg.optional && shown.is_none()).then_some((rail[0], "auto"));
    let width = form.rail(ui);
    let set = ui.scope(|ui| {
        ui.spacing_mut().slider_width = width;
        let reads = ("form", &title[..], at as f32, rail, (whole, word));
        running(ui, reads, &step, now, rec)
    });
    if !arg.hint.is_empty() {
        rec.hint(&arg.hint);
    }
    said(ui, &arg.hint, rec);
    let value = (set.inner).and_then(|s| s.args.pointer(&pointer(name)).cloned());
    let took = value.is_some();
    if let Some(value) = value {
        form.chosen.insert(name.into(), value);
    }
    took
}

/// The scene's parts by name in the parts list's rows: the one `name` shows lit (each it shows
/// when `several`), else the active part where the engine takes it when left out. A tap picks.
fn part(
    ui: &mut egui::Ui,
    form: &mut Form,
    name: &str,
    (arg, several): (&Arg, bool),
    now: &Now,
    rec: &mut Recorder,
) {
    if now.parts.is_empty() {
        label(ui, RichText::new("No parts yet.").weak(), rec);
        return;
    }
    let shows = form.shows(name);
    let mut picked = None;
    part_list(ui, &format!("form {name}"), now, rec, |ui, listed, rec| {
        let value = Value::from(listed.name.as_str());
        let lit = match &shows {
            Some(Value::Array(all)) => all.contains(&value),
            Some(shown) => *shown == value,
            None => arg.active && listed.active,
        };
        let chosen = if several {
            toggled(shows.as_ref(), &value)
        } else {
            value
        };
        let press = Press {
            lit,
            ..plain(&listed.name, form.with(name, chosen.clone()))
        };
        if pressed_as(ui, press, Face::Label, now, rec).clicked() {
            picked = Some(chosen);
        }
        None
    });
    if let Some(value) = picked {
        form.chosen.insert(name.into(), value);
    }
}

/// A color: egui's color button, sent as `"#rrggbb"` once changed.
fn color(ui: &mut egui::Ui, form: &mut Form, name: &str, rec: &mut Recorder) {
    let shows = form.shows(name);
    let rgb = shows.as_ref().and_then(Value::as_str).and_then(hex);
    let mut rgb = rgb.unwrap_or([128, 128, 128]);
    let shown = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
    let value = Value::from(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
    let step = form.with(name, value.clone());
    rec.widget(
        Widget::runs(friendly(name), Role::Button, step, None),
        shown.rect,
    );
    if shown.changed() {
        form.chosen.insert(name.into(), value);
    }
}

/// A value typed: a path, a name, a number with no ends, a list of numbers. The form's first
/// such field takes the keyboard when it opens, what it starts with selected, but for a path's
/// folder, which a name is typed after; typing drops a word chosen beside it.
fn typed(ui: &mut egui::Ui, form: &mut Form, name: &str, focus: &mut bool, rec: &mut Recorder) {
    let hint = (form.args.get(name)).map_or_else(String::new, |a| a.hint.clone());
    let mut all = form.fields.iter_mut().chain(form.more.iter_mut());
    let Some((_, text)) = all.find(|(n, _)| n == name) else {
        return;
    };
    let select = !crate::palette::path::named(name);
    if field(ui, text, (friendly(name), &hint), (focus, select), rec) {
        form.chosen.remove(name);
    }
}

/// Exactly three numbers, a point or a direction: X, Y and Z fields side by side (R, G and B for
/// a color, `x-rgb`), each recorded as the argument's name and its axis (`Center Y`, `Color R`).
/// Each shows its part of what is typed, else
/// of what the argument shows; a change of one types the three as a list, so the step sends
/// the whole point.
fn three(ui: &mut egui::Ui, form: &mut Form, name: &str, focus: &mut bool, rec: &mut Recorder) {
    let arg = form.args.get(name);
    let hint = arg.map_or_else(String::new, |a| a.hint.clone());
    let axes = if arg.is_some_and(|a| matches!(a.kind, Kind::Three(true))) {
        ["R", "G", "B"]
    } else {
        ["X", "Y", "Z"]
    };
    let (shows, title) = (form.shows(name), friendly(name));
    let mut all = form.fields.iter_mut().chain(form.more.iter_mut());
    let Some((_, text)) = all.find(|(n, _)| n == name) else {
        return;
    };
    let mut parts = parts(text, shows.as_ref());
    let mut changed = false;
    ui.columns(3, |columns| {
        let each = columns.iter_mut().zip(&mut parts).zip(axes);
        for ((column, part), axis) in each {
            column.horizontal(|ui| {
                ui.label(axis);
                let named = (format!("{title} {axis}"), hint.as_str());
                changed |= field(ui, part, named, (&mut *focus, false), rec);
            });
        }
    });
    if changed {
        let typed = parts.iter().any(|p| !p.trim().is_empty());
        *text = if typed {
            parts.join(", ")
        } else {
            String::new()
        };
        form.chosen.remove(name);
    }
}

/// The three numbers of a point as `text` types them (`1, 2, 3`, in brackets or not), else as
/// `shows` has them, else none.
fn parts(text: &str, shows: Option<&Value>) -> [String; 3] {
    let text = text.trim();
    let all: Vec<String> = if text.is_empty() {
        let shown = shows.and_then(Value::as_array).into_iter().flatten();
        shown.map(Value::to_string).collect()
    } else {
        let bare = text.trim_start_matches('[').trim_end_matches(']');
        bare.split(',').map(|p| p.trim().to_string()).collect()
    };
    std::array::from_fn(|i| all.get(i).cloned().unwrap_or_default())
}

/// A line to type into, recorded as a text field labelled `label` that says `hint` on hover,
/// taking the keyboard when `focus` is set, with what it holds then selected when `select`, to
/// type over (a part's name, review 0009, R15). Returns whether its text changed.
fn field(
    ui: &mut egui::Ui,
    text: &mut String,
    (label, hint): (String, &str),
    (focus, select): (&mut bool, bool),
    rec: &mut Recorder,
) -> bool {
    let shown = egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .show(ui);
    let field = shown.response;
    if std::mem::take(focus) {
        field.request_focus();
        if select {
            let mut state = shown.state;
            let all = egui::text::CCursorRange::select_all(&shown.galley);
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), field.id);
        }
    }
    rec.widget(Widget::plain(label, Role::TextField), field.rect);
    if !hint.is_empty() {
        rec.hint(hint);
    }
    field.changed()
}

/// `shows`, a list, with `value` added, or taken away if it has it.
fn toggled(shows: Option<&Value>, value: &Value) -> Value {
    let mut all = shows.and_then(Value::as_array).cloned().unwrap_or_default();
    match all.iter().position(|v| v == value) {
        Some(i) => {
            all.remove(i);
        }
        None => all.push(value.clone()),
    }
    Value::Array(all)
}

/// The bytes of a `"#rrggbb"` color.
fn hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}
