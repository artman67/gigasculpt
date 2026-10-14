//! Descriptions on hover (catalog `interface.gesture-map-widgets`, its first row, and the
//! address of `interface.tooltips`): while Ctrl is held over a button, a box beside the pointer
//! says what the button does, from its control's hint and its command's catalog summary, with
//! the command's address (what scripts, the keymap and the palette call it) and the key bound
//! to it. With Ctrl+Alt the box shows the command's whole docs instead. Nothing waits: the box
//! shows on the frame the key goes down and goes with it.
//!
//! A button notes itself (`pieces/buttons.rs`) while it is drawn; the box is drawn last
//! (`shell/regions.rs`), over everything, and is recorded as the panel `description`.

use egui::{Id, Order, Pos2, Ui, vec2};
use gs_engine::PanelInfo;
use gs_protocol::{Catalog, Step};

use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// The box's widest, in points.
const WIDTH: f32 = 320.0;

/// How far from the pointer the box's corner sits, in points, so it never covers the button.
const OFFSET: f32 = 16.0;

/// The button under the pointer while Ctrl is held: what it runs, by which key, and its
/// control's own words.
#[derive(Clone)]
struct Noted {
    step: Step,
    key: Option<String>,
    hint: Option<String>,
}

/// Where a frame keeps the button noted.
fn noted() -> Id {
    Id::new("described button")
}

/// Notes the button `shown`, which runs `step` by `key`, as the one to describe when the
/// pointer is on it with Ctrl held.
pub fn note(ui: &Ui, shown: &egui::Response, step: &Step, key: Option<&str>) {
    if shown.contains_pointer() && ui.input(|i| i.modifiers.ctrl) {
        let note = Noted {
            step: step.clone(),
            key: key.map(Into::into),
            hint: None,
        };
        ui.data_mut(|d| d.insert_temp(noted(), note));
    }
}

/// The control's own words for the button `shown`: in its description when it was noted,
/// else as egui's hover text, so only one box shows.
pub fn hint(ui: &Ui, shown: &egui::Response, hint: &str) {
    let note = ui.data(|d| d.get_temp::<Noted>(noted()));
    match note.filter(|_| shown.contains_pointer()) {
        Some(note) => {
            let hint = Some(hint.to_string());
            ui.data_mut(|d| d.insert_temp(noted(), Noted { hint, ..note }));
        }
        None => {
            shown.clone().on_hover_text(hint);
        }
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
    let lines = lines(note, catalog, ui.input(|i| i.modifiers.alt));
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
            for line in lines {
                let rect = ui.label(&line).rect;
                rec.widget(Widget::plain(line, Role::Label), rect);
            }
        });
    });
    rec.fit(shown.response.rect);
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

/// What the box says about `note`: its hint, its command's summary, its address and its key;
/// or, `longer`, the command's whole docs.
fn lines(note: Noted, catalog: &Catalog, longer: bool) -> Vec<String> {
    let Noted { step, key, hint } = note;
    let info = catalog.commands.iter().find(|c| c.name == step.command);
    if longer {
        return info.map(|i| i.docs.clone()).into_iter().collect();
    }
    let empty = step.args.as_object().is_none_or(serde_json::Map::is_empty);
    let address = if empty {
        step.command
    } else {
        format!("{} {}", step.command, step.args)
    };
    let summary = info.map(|i| i.summary.clone());
    let key = key.map(|k| format!("Key: {k}"));
    [hint, summary, Some(address), key]
        .into_iter()
        .flatten()
        .collect()
}
