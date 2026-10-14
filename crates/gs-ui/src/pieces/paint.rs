//! Brush tray > Paint (decision 0039, section 11, as amended 2026-10-12 in step P8a; decision
//! 0060 section 6, item 7): the palette's Main and Secondary swatches with the controls file's
//! `paint` set's first control, Swap, between them and its second, Paint's ⋯ (Fill part, Fill
//! visible), after them; while the edited color is open, the picker (`picker.rs`, as amended in
//! step P8b), the hex field and Red, Green and Blue; then the rest of the set in its order:
//! Color Strength while the brush paints, Gradient and Alternate on a row, and the brush's Color
//! switch after a small Brush for a brush outside the Paint family. Which swatch is edited is
//! the window's own: Main at first, then the one tapped last, ringed. A tap on the edited
//! swatch, or a hold on either, opens or shuts its color; a drag from a swatch onto the 3D view
//! picks a color into it. Every color change is one `color.set`, a View call and no undo step:
//! the picker's at most once a frame, a drag's when the pen lets go, the hex field's on Enter, a
//! slider's each time it is set.

use egui::{Color32, Id, Key, RichText, Sense, Stroke, StrokeKind};
use gs_core::brush::Family;
use gs_protocol::Step;
use serde_json::{Value, json};

use super::{
    Face, Now, Press, control, controls_of, held_past_click, hover, label, picker, pressed_as,
    running,
};
use crate::direction::Control;
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// Each swatch's title and its key in `color.set` and, after `color.`, in `session.state`.
const SWATCHES: [(&str, &str); 2] = [("Main", "main"), ("Secondary", "secondary")];

/// The sliders of the edited color's bytes.
const CHANNELS: [&str; 3] = ["Red", "Green", "Blue"];

/// The brush's Color Strength, null while it paints nothing.
const STRENGTH: &str = "brush.paint.color_strength";

/// The hex field's width in points: room for "#rrggbb".
const FIELD: f32 = 72.0;

/// Draws the section. Returns what it asked to run.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let set = controls_of("paint", now);
    let edited = edited(ui.ctx());
    let colors = SWATCHES.map(|(_, key)| {
        let value = now.state.get(&format!("color.{key}"));
        value
            .and_then(Value::as_str)
            .and_then(bytes)
            .unwrap_or_default()
    });
    let mut asked = swatch(ui, 0, (colors[0], edited), rec);
    if let Some(swap) = set.first() {
        asked = control(ui, swap, true, now, rec).or(asked);
    }
    asked = swatch(ui, 1, (colors[1], edited), rec).or(asked);
    if let Some(more) = set.get(1) {
        asked = control(ui, more, true, now, rec).or(asked);
    }
    ui.end_row();
    let (_, key) = SWATCHES[edited];
    if ui.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false) {
        asked = picker::rows(ui, key, colors[edited], rec).or(asked);
        asked = hex_field(ui, key, colors[edited], rec).or(asked);
        ui.end_row();
        asked = channels(ui, key, colors[edited], now, rec).or(asked);
    }
    rest(ui, set.get(2..).unwrap_or_default(), now, rec).or(asked)
}

/// Both swatches' colors, as a folded Paint's title says them: `#ff0000 · #0000ff`.
pub(super) fn summary(now: &Now) -> Option<String> {
    let [main, secondary] = SWATCHES.map(|(_, key)| now.state.get(&format!("color.{key}")));
    Some(format!("{} · {}", main?.as_str()?, secondary?.as_str()?))
}

/// Where egui keeps which swatch is edited: 0 Main, 1 Secondary.
fn edited_id() -> Id {
    Id::new("paint edited")
}

/// Which swatch is edited: 0 Main, 1 Secondary.
fn edited(ctx: &egui::Context) -> usize {
    ctx.data(|d| d.get_temp::<usize>(edited_id())).unwrap_or(0) % 2
}

/// The edited swatch's key in `color.set`.
pub(super) fn edited_key(ctx: &egui::Context) -> &'static str {
    SWATCHES[edited(ctx)].1
}

/// Where egui keeps whether the picker is open.
fn open_id() -> Id {
    Id::new("paint picker open")
}

/// Swatch `i` showing `value`, recorded under its title with its color on hover and selected
/// while it is the `edited` one, which has the theme's lit ring. A tap on the other swatch makes
/// it the edited one; a tap on the edited one opens or shuts its color; a hold on either makes it
/// the edited one and opens it. A drag from it onto the 3D view shows the color under the pen and
/// sets it on the release, with one `color.set` (`picker::dragged`).
fn swatch(
    ui: &mut egui::Ui,
    i: usize,
    (value, edited): ([u8; 3], usize),
    rec: &mut Recorder,
) -> Option<Step> {
    let (title, key) = SWATCHES[i];
    let size = ui.spacing().interact_size;
    let (rect, shown) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (color, picked) = picker::dragged(ui, &shown, key, value);
    let [r, g, b] = color;
    let colors = &theme(ui.ctx()).colors;
    (ui.painter()).rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
    let edge = Stroke::new(1.0, colors.edge);
    (ui.painter()).rect_stroke(rect, 2.0, edge, StrokeKind::Inside);
    let step = |c: [u8; 3]| with_args("color.set", json!({ key: hex(c) }));
    let widget = Widget::runs(title, Role::Button, step(color), None);
    rec.widget(widget.selected(i == edited), rect);
    if i == edited {
        let ring = Stroke::new(2.0, colors.lit);
        (ui.painter()).rect_stroke(rect.expand(2.0), 2.0, ring, StrokeKind::Outside);
    }
    let said = format!(
        "{title} color {}: a tap edits it; drag onto the model to pick a color, with Alt as shown",
        hex(color)
    );
    let shown = hover(shown, Some(&said), rec);
    let held = held_past_click(ui, &shown);
    if shown.clicked() || held {
        let open = ui.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false);
        let open = held || open != (i == edited);
        ui.data_mut(|d| {
            d.insert_temp(edited_id(), i);
            d.insert_temp(open_id(), open);
        });
    }
    picked.filter(|c| *c != value).map(step)
}

/// The edited color `key` as `#rrggbb` in a field. A tap empties it, showing the color faintly;
/// Enter sends what was typed, with or without its `#`, and Escape or a press elsewhere sends
/// nothing. What is not a color goes as typed, and the engine refuses it with its reason.
fn hex_field(ui: &mut egui::Ui, key: &str, value: [u8; 3], rec: &mut Recorder) -> Option<Step> {
    let id = Id::new("paint hex");
    let typed = id.with("typed");
    let now = hex(value);
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text = match ui.data(|d| d.get_temp::<String>(typed)) {
        Some(text) if focused => text,
        _ => now.clone(),
    };
    let field = egui::TextEdit::singleline(&mut text)
        .id(id)
        .hint_text(&now)
        .char_limit(7)
        .desired_width(FIELD);
    let field = ui.add(field);
    rec.widget(Widget::plain("Hex", Role::TextField), field.rect);
    let field = hover(field, Some("Hex: type a color as #rrggbb, then Enter"), rec);
    if field.has_focus() {
        let kept = if field.gained_focus() {
            String::new()
        } else {
            text
        };
        ui.data_mut(|d| d.insert_temp(typed, kept));
        return None;
    }
    ui.data_mut(|d| d.remove::<String>(typed));
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let text = text.trim().trim_start_matches('#');
    (entered && !text.is_empty())
        .then(|| with_args("color.set", json!({ key: format!("#{text}") })))
}

/// Red, Green and Blue, 0 to 255, for the edited color `key`, each a slider on a row of its own
/// that runs `color.set` once a value.
fn channels(
    ui: &mut egui::Ui,
    key: &str,
    rgb: [u8; 3],
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut asked = None;
    for (i, title) in CHANNELS.into_iter().enumerate() {
        let step = |v: f32| {
            let mut c = rgb;
            c[i] = v.round().clamp(0.0, 255.0) as u8;
            with_args("color.set", json!({ key: hex(c) }))
        };
        let slider = ("paint", title, f32::from(rgb[i]), [0.0, 255.0], false);
        asked = running(ui, slider, &step, now, rec).or(asked);
        ui.end_row();
    }
    asked
}

/// The set's controls after Swap and the ⋯, in their order, with short labels: a slider on a row
/// of its own, shown only while the value it shows is not null, and the buttons on one row; then
/// the brush's switch.
fn rest(ui: &mut egui::Ui, set: &[Control], now: &Now, rec: &mut Recorder) -> Option<Step> {
    let mut asked = None;
    for c in set {
        let value = now.state.get(c.shows.as_deref().unwrap_or_default());
        let slider = c.range.is_some();
        if !slider || value.is_some_and(|v| !v.is_null()) {
            asked = control(ui, c, true, now, rec).or(asked);
            if slider {
                ui.end_row();
            }
        }
    }
    ui.end_row();
    brush_switch(ui, now, rec).or(asked)
}

/// The brush's color switch, Color after a small Brush, on a row of its own, for a brush outside
/// the Paint family (theirs always paint). Lit while the brush paints color, it sends `brush.set`
/// with `settings.paint` a Color table, and while lit null, which puts back the preset's own. It
/// shows on a brush that cannot paint too, its hint saying so (decision 0062).
fn brush_switch(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    if now.preset.is_none_or(|p| p.family == Family::Paint) {
        return None;
    }
    let paints = now.state.get(STRENGTH).is_some_and(|v| !v.is_null());
    let paint = if paints {
        Value::Null
    } else {
        json!({ "channel": "color" })
    };
    let press = Press {
        label: "Color",
        step: with_args("brush.set", json!({ "settings": { "paint": paint } })),
        key: None,
        lit: paints,
        shows: None,
    };
    let step = press.step.clone();
    label(ui, RichText::new("Brush").small().weak(), rec);
    let shown = pressed_as(ui, press, Face::Label, now, rec);
    let said = "Color: the brush paints Main as it strokes, at its Color Strength. Drag brushes \
                such as Move and the gesture brushes cannot paint; on them it does nothing";
    let shown = hover(shown, Some(said), rec);
    ui.end_row();
    shown.clicked().then_some(step)
}

/// The bytes of a color written `#rrggbb`.
fn bytes(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#').filter(|d| d.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `rgb` written `#rrggbb`, as the engine keeps it.
pub(super) fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}
