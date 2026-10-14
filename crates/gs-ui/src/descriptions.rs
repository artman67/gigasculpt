//! Descriptions on hover (catalog `interface.gesture-map-widgets`, its first row, and the
//! address of `interface.tooltips`): while Ctrl is held over a button, a box beside the pointer
//! says what the button does, from its control's hint and its command's catalog summary, with
//! the command's address (what scripts, the keymap and the palette call it) and the key bound
//! to it. With Ctrl+Alt the box shows the command's whole docs instead. A button that picks a
//! brush has the box with no key held: the keys that pick the brush first, drawn as key caps,
//! then the brush's name, its control's hint and why it cannot run now; with Ctrl its keys lead
//! the description in place of the key line. Nothing waits: the box shows on the frame the
//! pointer or the key comes, and goes with them.
//!
//! A button notes itself (`pieces/buttons.rs`) while it is drawn; the box is drawn last
//! (`shell/regions.rs`), over everything, and is recorded as the panel `description`, a brush's
//! keys as one label: `B S M or Shift held`.

use egui::{Id, Order, Pos2, RichText, Stroke, Ui, vec2};
use gs_engine::PanelInfo;
use gs_protocol::{Catalog, Step};

use crate::keymap::{BrushKeys, NO_KEY};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// The box's widest, in points.
const WIDTH: f32 = 320.0;

/// How far from the pointer the box's corner sits, in points, so it never covers the button.
const OFFSET: f32 = 16.0;

/// A button that picks a brush: the brush's name, the keys that pick it, and why the button
/// cannot run now, if it cannot (said in the box, not in a second one).
#[derive(Clone)]
pub struct Brush {
    pub name: String,
    pub keys: BrushKeys,
    pub why: Option<String>,
}

/// The button under the pointer while Ctrl is held, or a brush's button under it: what it runs,
/// by which key, its control's own words, and the brush it picks.
#[derive(Clone)]
struct Noted {
    step: Step,
    key: Option<String>,
    hint: Option<String>,
    brush: Option<Brush>,
}

/// Where a frame keeps the button noted.
fn noted() -> Id {
    Id::new("described button")
}

/// Notes the button `shown`, which runs `step` by `key`, as the one to describe when the
/// pointer is on it with Ctrl held.
pub fn note(ui: &Ui, shown: &egui::Response, step: &Step, key: Option<&str>) {
    note_brush(ui, shown, step, key, None);
}

/// [`note`] for a button that picks `brush`, if it picks one, which is described with no key
/// held too.
pub fn note_brush(
    ui: &Ui,
    shown: &egui::Response,
    step: &Step,
    key: Option<&str>,
    brush: Option<Brush>,
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if shown.contains_pointer() && (ctrl || brush.is_some()) {
        let note = Noted {
            step: step.clone(),
            key: key.map(Into::into),
            hint: None,
            brush,
        };
        ui.data_mut(|d| d.insert_temp(noted(), note));
    }
}

/// The control's own words for the button `shown`: in its description when it was noted,
/// else as egui's hover text, so only one box shows.
pub fn hint(ui: &Ui, shown: &egui::Response, hint: &str) {
    said(ui.ctx(), shown.clone(), hint);
}

/// [`hint`] for a caller with no `Ui` at hand. Returns `shown`.
pub fn said(ctx: &egui::Context, shown: egui::Response, hint: &str) -> egui::Response {
    let note = ctx.data(|d| d.get_temp::<Noted>(noted()));
    match note.filter(|_| shown.contains_pointer()) {
        Some(note) => {
            let hint = Some(hint.to_string());
            ctx.data_mut(|d| d.insert_temp(noted(), Noted { hint, ..note }));
            shown
        }
        None => shown.on_hover_text(hint),
    }
}

/// Draws the description of the button noted this frame, if any, beside the pointer, with
/// `catalog` for its command's docs. Call after everything else is drawn.
pub fn show(ui: &Ui, catalog: &Catalog, rec: &mut Recorder) {
    let note = ui.data_mut(|d| {
        let note = d.get_temp::<Noted>(noted());
        d.remove::<Noted>(noted());
        note
    });
    let (Some(note), Some(at)) = (note, ui.input(|i| i.pointer.latest_pos())) else {
        return;
    };
    let keys = note.brush.as_ref().map(|b| b.keys.clone());
    let lines = lines(
        note,
        catalog,
        ui.input(|i| (i.modifiers.ctrl, i.modifiers.alt)),
    );
    let t = theme(ui.ctx());
    let room = ui.ctx().content_rect().shrink(t.spacing.margin);
    let area = egui::Area::new(Id::new("description"))
        .order(Order::Tooltip)
        .fixed_pos(at + vec2(OFFSET, OFFSET))
        .constrain_to(room)
        .interactable(false);
    let shown = area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(WIDTH);
            rec.panel("description", "Description", ui.max_rect());
            if let Some(keys) = keys {
                let rect = caps(ui, &keys);
                rec.widget(Widget::plain(keys.text(), Role::Label), rect);
            }
            for line in lines {
                let rect = ui.label(&line).rect;
                rec.widget(Widget::plain(line, Role::Label), rect);
            }
        });
    });
    rec.fit(shown.response.rect);
}

/// `keys` on one row: each key a cap, bold in the theme's text color on a control's fill with
/// an edge, a sequence's keys a cap each (`B` `S` `M` or `Shift` held), and the words between
/// them dim; [`NO_KEY`] dim when nothing picks it. Returns where the row is.
fn caps(ui: &mut Ui, keys: &BrushKeys) -> egui::Rect {
    let colors = &theme(ui.ctx()).colors;
    let word = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).color(colors.text_dim));
    };
    let cap = |ui: &mut Ui, text: &str| {
        let frame = egui::Frame::new()
            .fill(colors.control)
            .stroke(Stroke::new(1.0, colors.edge))
            .corner_radius(4.0)
            .inner_margin(vec2(6.0, 1.0));
        frame.show(ui, |ui| {
            ui.label(RichText::new(text).strong().color(colors.text))
        });
    };
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(key) = &keys.key {
            key.split(' ').for_each(|k| cap(ui, k));
        }
        if let Some(held) = &keys.held {
            if keys.key.is_some() {
                word(ui, "or");
            }
            cap(ui, held);
            word(ui, "held");
        }
        if keys.key.is_none() && keys.held.is_none() {
            word(ui, NO_KEY);
        }
    });
    row.response.rect
}

/// Whether `at` is inside `rect` (min x, min y, max x, max y), both in pixels.
pub fn inside(rect: [f32; 4], at: Pos2) -> bool {
    (rect[0]..=rect[2]).contains(&at.x) && (rect[1]..=rect[3]).contains(&at.y)
}

/// The panel a press at `at` (in pixels) lands on, of those recorded last frame: the one on top
/// there, looking past the box, which takes no press.
pub fn pressed_panel(panels: &[PanelInfo], at: Pos2) -> Option<&PanelInfo> {
    (panels.iter().rev()).find(|p| p.id != "description" && inside(p.rect, at))
}

/// What the box says about `note` under a brush's keys: with no Ctrl held (`ctrl`), the brush's
/// name, its hint and why it cannot run; else its hint, its command's summary, its address and
/// its key (but a brush's, whose keys lead) and why; or, with Alt too (`alt`), the command's
/// whole docs.
fn lines(note: Noted, catalog: &Catalog, (ctrl, alt): (bool, bool)) -> Vec<String> {
    let Noted {
        step,
        key,
        hint,
        brush,
    } = note;
    let why = brush.as_ref().and_then(|b| b.why.clone());
    if let Some(brush) = brush.as_ref().filter(|_| !ctrl) {
        return [Some(brush.name.clone()), hint, why]
            .into_iter()
            .flatten()
            .collect();
    }
    let info = catalog.commands.iter().find(|c| c.name == step.command);
    if alt {
        return info.map(|i| i.docs.clone()).into_iter().collect();
    }
    let empty = step.args.as_object().is_none_or(serde_json::Map::is_empty);
    let address = if empty {
        step.command
    } else {
        format!("{} {}", step.command, step.args)
    };
    let summary = info.map(|i| i.summary.clone());
    let key = key.filter(|_| brush.is_none()).map(|k| format!("Key: {k}"));
    [hint, summary, Some(address), key, why]
        .into_iter()
        .flatten()
        .collect()
}
