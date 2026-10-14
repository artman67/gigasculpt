//! The parts list, and Isolate's button per part. What a tap on a part's row runs is the
//! controls file's `part-row` set (decision 0033, section 7), one group to each place on the
//! row: `Name`, the part's name, lit while the part is active; `Eye`, lit while the part's eye
//! is on, and `Shift+Eye` (any held keys before `+Eye`), the eye tapped with those keys held;
//! `Paint` and `Shift+Paint` likewise, after the eye, lit while the part's paint shows (decision
//! 0039, section 10); `Isolate`, the part's button in Isolate's options, lit while the part
//! shows. In a control's arguments `"@row"` is the row's part and `"flip"` the opposite of the
//! row's light.

use egui::scroll_area::{DragScroll, ScrollSource, State};
use egui::{RichText, ScrollArea};
use gs_protocol::Step;
use serde_json::{Value, json};

use super::buttons::{Face, Press, hover, pressed_as};
use super::{ListedPart, Now, label};
use crate::direction::Control;
use crate::record::{Recorder, with_args};

/// The set whose groups say what a tap on a part's row runs.
const ROW: &str = "part-row";

/// The parts, one to a row under their title when `titled`: each part's eye and paint switch,
/// then its name.
/// The eye is lit while the part's own visibility is on, which Isolate leaves alone; during
/// Isolate a change shows once Isolate ends (decision 0028). The active part's name is lit.
pub(super) fn parts(
    ui: &mut egui::Ui,
    now: &Now,
    titled: bool,
    rec: &mut Recorder,
) -> Option<Step> {
    if titled {
        label(ui, RichText::new("Parts").strong(), rec);
        ui.end_row();
    }
    if now.parts.is_empty() {
        let hint = "No parts yet: use Add or File > Import.";
        label(ui, RichText::new(hint).weak(), rec);
        ui.end_row();
        return None;
    }
    let (eye, paint) = (held(now, "Eye"), held(now, "Paint"));
    let name = first(now, "Name");
    part_list(ui, "parts", now, rec, |ui, part, rec| {
        let mut asked = None;
        if let Some(c) = eye {
            asked = tap(ui, c, &c.label, (part, part.visible), now, rec);
        }
        if let Some(c) = paint {
            asked = tap(ui, c, &c.label, (part, part.painted), now, rec).or(asked);
        }
        if let Some(c) = name {
            return tap(ui, c, &part.name, (part, part.active), now, rec).or(asked);
        }
        label(ui, RichText::new(&part.name), rec);
        asked
    })
}

/// The active part's name, which Parts' title row shows after its title, open or folded
/// (decision 0060 as amended for S1, section 1); `None` in an empty scene.
pub(super) fn active(now: &Now) -> Option<String> {
    now.parts.iter().find(|p| p.active).map(|p| p.name.clone())
}

/// Isolate's options (decision 0028): a button per part that shows it alone, lit while the part
/// shows, so pressing the part shown alone ends Isolate, as `part.isolate` does. The `isolate`
/// set's controls follow it.
pub(super) fn isolate(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    if now.parts.is_empty() {
        label(ui, RichText::new("No parts yet.").weak(), rec);
        ui.end_row();
        return None;
    }
    let c = first(now, "Isolate")?;
    part_list(ui, "isolate", now, rec, |ui, part, rec| {
        tap(ui, c, &part.name, (part, part.shown), now, rec)
    })
}

/// The parts, a row each drawn by `row`, in a list that shows the window's `part_rows` of them
/// at a time and scrolls by a drag, drawing only the rows in sight, so a scene of thousands of
/// parts keeps the controls under it in reach (decision 0028, section 10). When the active part
/// or the rows shown change, the list scrolls just far enough to show it. Ends the row it is
/// drawn in.
pub(crate) fn part_list(
    ui: &mut egui::Ui,
    salt: &str,
    now: &Now,
    rec: &mut Recorder,
    mut row: impl FnMut(&mut egui::Ui, &ListedPart, &mut Recorder) -> Option<Step>,
) -> Option<Step> {
    let parts = now.parts;
    let gap = ui.spacing().item_spacing.y;
    let tall = ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    let tall = tall.max(ui.spacing().interact_size.y);
    let rows = (now.layout.settings.part_rows.max(1) as usize).min(parts.len());
    let height = rows as f32 * (tall + gap) - gap;
    let active = parts.iter().position(|p| p.active);
    let mut asked = None;
    ui.vertical(|ui| {
        // The scroll area's own id: it hashes its salt into an `IdSalt` first.
        let id = ui.make_persistent_id(egui::IdSalt::new(salt));
        let source = ScrollSource {
            drag: DragScroll::Always,
            ..ScrollSource::default()
        };
        let mut area = ScrollArea::vertical()
            .id_salt(salt)
            .scroll_source(source)
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, true]);
        if let Some(to) = into_sight(
            ui,
            id,
            active.map(|i| (i, &parts[i].name)),
            tall + gap,
            height,
        ) {
            area = area.vertical_scroll_offset(to);
        }
        let outer = rec.listing();
        area.show_rows(ui, tall, parts.len(), |ui, range| {
            rec.list(Some(ui.clip_rect()));
            for part in &parts[range] {
                ui.horizontal(|ui| {
                    ui.set_min_height(tall);
                    asked = row(ui, part, rec).or(asked.take());
                });
            }
        });
        rec.list(outer);
    });
    ui.end_row();
    asked
}

/// The scroll offset that brings `active` (its row and name) into a list `id` whose rows are
/// `step` apart and which shows `height`, when the active part or the height is another than
/// when the list last asked; `None` to leave the list where it is.
fn into_sight(
    ui: &egui::Ui,
    id: egui::Id,
    active: Option<(usize, &String)>,
    step: f32,
    height: f32,
) -> Option<f32> {
    let memo = id.with("active");
    // The height by its bits, which compare exactly.
    let seen = (active.map(|(_, name)| name.clone()), height.to_bits());
    let last: Option<(Option<String>, u32)> = ui.data(|d| d.get_temp(memo));
    if last.as_ref() == Some(&seen) {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(memo, seen));
    let (i, _) = active?;
    let offset = State::load(ui.ctx(), id).map_or(0.0, |s| s.offset.y);
    let gap = ui.spacing().item_spacing.y;
    let top = i as f32 * step;
    let bottom = top + step - gap;
    if top < offset {
        Some(top)
    } else if bottom > offset + height {
        Some(bottom - height)
    } else {
        None
    }
}

/// The control of the `part-row` set's group `group` (`Eye`), or of its group for the held keys
/// (`Shift+Eye`) when the set has one for them.
fn held<'a>(now: &'a Now, group: &str) -> Option<&'a Control> {
    let held = now.held.as_ref().map(|h| format!("{}+{group}", h.keys));
    held.and_then(|g| first(now, &g))
        .or_else(|| first(now, group))
}

/// The first control of the `part-row` set's group `group`, if the controls file has one.
fn first<'a>(now: &'a Now, group: &str) -> Option<&'a Control> {
    let (_, picked) = now.directions.controls.piece(&format!("{ROW}/{group}"))?;
    picked.first().copied()
}

/// Draws `c` on `part`'s row as a button labelled `label`, lit while `lit`, saying `c`'s hint on
/// hover. A label too long for the row is cut, whole on hover, so the row fits a narrow tray.
/// Returns its command when tapped.
fn tap(
    ui: &mut egui::Ui,
    c: &Control,
    label: &str,
    (part, lit): (&ListedPart, bool),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut args = serde_json::to_value(&c.args).unwrap_or_default();
    fill(&mut args, &part.name, lit);
    let press = Press {
        label,
        step: with_args(c.command.as_deref()?, args),
        key: None,
        lit,
        shows: None,
    };
    let step = press.step.clone();
    let shown = pressed_as(ui, press, Face::Cut, now, rec);
    hover(shown, c.hint.as_deref(), rec)
        .clicked()
        .then_some(step)
}

/// Fills in a row's arguments: `"@row"` is part `name`, `"flip"` the opposite of `lit`.
fn fill(v: &mut Value, name: &str, lit: bool) {
    let filled = match v.as_str() {
        Some("@row") => json!(name),
        Some("flip") => json!(!lit),
        _ => {
            match v {
                Value::Array(items) => items.iter_mut().for_each(|v| fill(v, name, lit)),
                Value::Object(fields) => fields.values_mut().for_each(|v| fill(v, name, lit)),
                _ => {}
            }
            return;
        }
    };
    *v = filled;
}
