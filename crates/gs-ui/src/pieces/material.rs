//! The Look options' Material section (decision 0043, section 8, steps (b) to (d)): the active
//! material's picture, which opens the Library's Materials tab, its title, its `···` (the controls
//! file's `material` set, decision 0060 as amended for R1), which lists the palette's Undo and
//! Redo, Reset, Copy, Rename and Delete, and what the user folder refused; the slot slider,
//! which picks a material by its place; then, for any material but a flat one (Flat Color),
//! which has no settings, its colors on egui's color buttons, the shipped environments that can
//! light it, a slider for each of its nine numeric keys and, for a standard material, its
//! cavity Intensity, Radius and Colorize. A slider or a color runs `material.set` once when the
//! pen lets go, so a drag or a pick is one palette step.

use egui::{Color32, RichText};
use gs_engine::MaterialInfo;
use gs_protocol::Step;
use serde_json::json;

use super::sliders::{index, material as slider};
use super::{Face, Now, Press, control, controls_of, hover, label, pressed_as};
use crate::library::thumbs::{Source, picture};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// Each slider's title, the key it sets and its rail. A typed value goes past the rail, as far
/// as `material.set` takes it.
const KEYS: [(&str, &str, [f32; 2]); 9] = [
    ("Metalness", "base_metalness", [0.0, 1.0]),
    ("Specular", "specular_weight", [0.0, 1.0]),
    ("Roughness", "specular_roughness", [0.0, 1.0]),
    ("IOR", "specular_ior", [1.0, 3.0]),
    ("Coat", "coat_weight", [0.0, 1.0]),
    ("Coat roughness", "coat_roughness", [0.0, 1.0]),
    ("Coat IOR", "coat_ior", [1.0, 3.0]),
    ("Subsurface", "subsurface_weight", [0.0, 1.0]),
    // The shipped looks' bake ball is in millimeters; skin scatters about 5.
    ("Scatter radius", "subsurface_radius", [0.0, 10.0]),
];

/// A standard material's cavity sliders, after the nine, on their whole ranges.
const CAVITY: [(&str, &str, [f32; 2]); 3] = [
    ("Cavity intensity", "cavity_intensity", [0.0, 2.0]),
    ("Cavity radius", "cavity_radius", [0.25, 4.0]),
    ("Cavity colorize", "cavity_colorize", [0.0, 1.0]),
];

/// Each color's title and key; the cavity's, last, only for a standard material.
const COLORS: [(&str, &str); 4] = [
    ("Base color", "base_color"),
    ("Specular color", "specular_color"),
    ("Coat color", "coat_color"),
    ("Cavity color", "cavity_color"),
];

/// The active material's picture, in points across; an environment's.
const LARGE: f32 = 72.0;
const SMALL: f32 = 36.0;

/// Draws the section, each part on a row of its own. Returns what it asked to run.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let looks = &now.looks.looks;
    let at = looks.iter().position(|m| m.name == now.looks.selected)?;
    let active = &looks[at];
    let mut asked = head(ui, active, now, rec);
    ui.end_row();
    refused(ui, now, rec);
    let pick = |i: usize| with_args("material.select", json!({ "name": format!("@{i}") }));
    let names = ("Material slot", "Slot");
    asked = index(ui, names, (looks.len(), at), pick, (now, rec)).or(asked);
    ui.end_row();
    if active.kind == "flat" {
        label(
            ui,
            RichText::new("A flat material has no settings").weak(),
            rec,
        );
        ui.end_row();
        return asked;
    }
    let material = format!("#{}", active.id);
    let values = serde_json::to_value(&active.values).ok()?;
    let standard = active.kind == "standard";
    let colors = if standard { &COLORS[..] } else { &COLORS[..3] };
    for &(title, key) in colors {
        let rgb = std::array::from_fn(|i| values[key][i].as_f64().unwrap_or_default() as f32);
        asked = color(ui, (title, key, rgb), &material, rec).or(asked);
    }
    ui.end_row();
    asked = environments(ui, active, &material, now, rec).or(asked);
    ui.end_row();
    let cavity: &[_] = if standard { &CAVITY } else { &[] };
    for &(title, key, ends) in KEYS.iter().chain(cavity) {
        let value = values[key].as_f64().unwrap_or_default() as f32;
        asked = slider(ui, (title, key, value, ends), &material, now, rec).or(asked);
        ui.end_row();
    }
    asked
}

/// The active material's picture, which opens the Library's Materials tab, its title, and the
/// `material` set by short labels: its `···`.
fn head(ui: &mut egui::Ui, active: &MaterialInfo, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let press = Press {
        label: "Materials",
        step: with_args("ui.library", json!({ "tab": "matcaps" })),
        key: None,
        lit: false,
        shows: None,
    };
    let step = press.step.clone();
    let shown = pressed_as(ui, press, Face::Square(LARGE), now, rec);
    let shown = hover(shown, Some("Opens the Library's Materials"), rec);
    let look = with_args("view.look", json!({ "matcap": active.name }));
    paint(ui, &shown, Source::of(&look, now.materials));
    label(ui, RichText::new(&active.title), rec);
    let actions = controls_of("material", now).into_iter();
    let picked = actions.fold(None, |asked, c| control(ui, &c, true, now, rec).or(asked));
    shown.clicked().then_some(step).or(picked)
}

/// How many files the user folder holds that were not read, each named with why on hover.
fn refused(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) {
    let refused = &now.looks.refused;
    if refused.is_empty() {
        return;
    }
    let n = refused.len();
    let text = format!("{n} file{} not read", if n == 1 { "" } else { "s" });
    let why: Vec<String> = (refused.iter())
        .map(|r| format!("{}: {}", r.file, r.error))
        .collect();
    let shown = ui.label(RichText::new(&text).weak());
    rec.widget(Widget::plain(text, Role::Label), shown.rect);
    hover(shown, Some(&why.join("\n")), rec);
    ui.end_row();
}

/// The active material's color `key`, `value` linear, on egui's color button, recorded as
/// `title`; while its picker is open, the picker too, as `{title} picker`. A pick runs
/// `material.set` once: when the pen lets go, or when the picker shuts while it is held.
fn color(
    ui: &mut egui::Ui,
    (title, key, value): (&str, &str, [f32; 3]),
    material: &str,
    rec: &mut Recorder,
) -> Option<Step> {
    let id = egui::Id::new(("material color", key));
    let mut rgb = ui.data(|d| d.get_temp::<[f32; 3]>(id)).unwrap_or(value);
    // The id egui's button gives its picker, taken before the button takes the next one.
    let popup = ui.auto_id_with("popup");
    let shown = ui.color_edit_button_rgb(&mut rgb);
    let step = |c: [f32; 3]| with_args("material.set", json!({ "material": material, key: c }));
    rec.widget(
        Widget::runs(title, Role::Button, step(rgb), None),
        shown.rect,
    );
    hover(shown, Some(title), rec);
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    if let Some(rect) = (ui.ctx().memory(|m| m.area_rect(popup))).filter(|_| open) {
        // Over the options, not in their scrolled list.
        let listing = rec.listing();
        rec.list(None);
        let picker = Widget::runs(format!("{title} picker"), Role::Slider, step(rgb), None);
        rec.widget(picker, rect);
        rec.list(listing);
    }
    // egui's picker goes through HSV and back: a change is more than its rounding.
    let moved = rgb.iter().zip(value).any(|(a, b)| (a - b).abs() > 1e-4);
    if moved && open && ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.insert_temp(id, rgb));
        return None;
    }
    ui.data_mut(|d| d.remove::<[f32; 3]>(id));
    moved.then(|| step(rgb))
}

/// The shipped environments as the Environment set names them, each a picture of it labelled
/// `Lit by` its title, the material's own lit. A tap lights the material with it.
fn environments(
    ui: &mut egui::Ui,
    active: &MaterialInfo,
    material: &str,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut asked = None;
    for c in controls_of("environment", now) {
        let Some(name) = c.args.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let title = format!("Lit by {}", c.label);
        let press = Press {
            label: &title,
            step: with_args(
                "material.set",
                json!({ "material": material, "environment": name }),
            ),
            key: None,
            lit: active.environment == name,
            shows: None,
        };
        let step = press.step.clone();
        let shown = pressed_as(ui, press, Face::Square(SMALL), now, rec);
        let shown = hover(shown, Some(&title), rec);
        let ball = with_args("view.environment", json!({ "name": name }));
        paint(ui, &shown, Source::of(&ball, now.materials));
        asked = shown.clicked().then_some(step).or(asked);
    }
    asked
}

/// Paints `source`'s picture on the square button `shown`, dimmed while it cannot run.
fn paint(ui: &egui::Ui, shown: &egui::Response, source: Option<Source>) {
    if !ui.is_rect_visible(shown.rect) {
        return;
    }
    let Some(texture) = source.and_then(|s| picture(ui.ctx(), &s)) else {
        return;
    };
    let t = theme(ui.ctx());
    let face = shown.rect.shrink(t.spacing.item[1]);
    let tint = if shown.enabled() {
        Color32::WHITE
    } else {
        t.colors.text_off
    };
    egui::Image::from_texture((texture, face.size()))
        .tint(tint)
        .paint_at(ui, face);
}
