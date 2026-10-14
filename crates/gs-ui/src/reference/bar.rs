//! The Reference View's bar over its pictures, in the header's style (decision 0060 as amended
//! for H5): on the header's color, Follow and its three, Align, then Frame, Wire, Look, Next and
//! Close, each an icon alone where it has one, all for the active reference. It wraps when
//! narrow; the floating window is never narrower than its one row ([`width`]).

use egui::{Popup, ScrollArea};
use gs_protocol::Step;
use serde_json::json;

use super::PICKED;
use crate::header::{PAD, buttons, frame, one_row, tip};
use crate::icons;
use crate::pieces::{Face, Now, faced};
use crate::record::{Recorder, Role, Widget, with_args};

/// What the Look button is for.
const LOOK: &str = "The MatCap the reference draws with: its own, or the same as the view's.";

/// Draws the bar across `ui`. Returns what it asked to run.
pub(super) fn bar(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Vec<Step> {
    let mut asked = Vec::new();
    frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0 * f32::from(PAD);
        ui.horizontal_wrapped(|ui| asked = row(ui, now, rec));
    });
    asked
}

/// How wide the bar is on one row, in points, its margins included.
pub(super) fn width(ui: &mut egui::Ui, now: &Now) -> f32 {
    let wide = one_row(ui, "reference bar width", |ui, rec| {
        row(ui, now, rec);
    });
    wide + frame(ui).total_margin().sum().x
}

/// The bar's buttons in order: the `reference` set's, Look, then the `reference-views` set's.
fn row(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Vec<Step> {
    let mut asked = buttons(ui, "reference", now, None, rec);
    asked.extend(look(ui, now, rec));
    asked.extend(buttons(ui, "reference-views", now, None, rec));
    asked
}

/// The Look button: a tap opens a menu of the MatCaps for the reference, "Same as the view"
/// first, each row recorded with the `view.look` it runs while the menu is open. Returns the
/// pick.
fn look(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let icon = icons::of_options("look");
    let face = icon.map_or(Face::Label, Face::Alone);
    let button = faced(ui, (face, "Look"), (None, None), (true, false));
    rec.widget(Widget::plain("Look", Role::Button), button.rect);
    tip(ui, &button, ("Look", icon.is_some()), Some(LOOK), rec);
    let current = now.state.get("reference.look").and_then(|v| v.as_str());
    let mut rows = vec![("main".to_string(), "Same as the view".to_string())];
    rows.extend((now.looks.looks.iter()).map(|l| (l.name.clone(), l.title.clone())));
    let most = ui.spacing().combo_height;
    let menu = Popup::menu(&button).show(|ui| {
        let scroll = ScrollArea::vertical().max_height(most);
        let mut picked = None;
        scroll.show(ui, |ui| {
            for (name, title) in rows {
                let on = current == Some(name.as_str());
                let row = ui.selectable_label(on, &title);
                let step = with_args("view.look", json!({ "view": PICKED, "matcap": name }));
                let widget = Widget::runs(title, Role::Button, step.clone(), None).selected(on);
                rec.widget(widget, row.rect);
                if row.clicked() {
                    picked = Some(step);
                }
            }
        });
        picked
    });
    menu.and_then(|m| m.inner)
}
