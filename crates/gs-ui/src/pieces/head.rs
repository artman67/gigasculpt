//! The brush tray's header (decision 0060, section 6): the brush's name, whose tap opens the
//! brush picker beside the tray (the brush's family first, then the others, each under its
//! name; a pick is one brush step and shuts it), then the `head` set: the brush's ⋯ with Save
//! brush…, Load brush…, Reset to defaults and Copy settings to…. Brushes have no pictures yet,
//! so the name stands alone. It never folds: a tray draws it with no title row. While the Gizmo
//! tool is on it is the tool's: its name, then the `gizmo-head` set, its ⋯.

use egui::{Popup, RichText};
use gs_protocol::Step;
use serde_json::json;

use super::{Fit, Now, control, controls, hover, label, options_mark, popover};
use crate::floats::scrolled;
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// The brush name's hint (decision 0060, section 11).
const HINT: &str =
    "The brush you are using. Tap for the brush picker. Backtick goes back to the last brush.";

/// The header's piece name.
pub const HEAD: &str = "head";
/// The set the header shows while the Gizmo tool is on: the tool's ⋯.
pub(super) const GIZMO: &str = "gizmo-head";

/// The brush's name, its picker while open, and the brush's ⋯; while the Gizmo tool is on, the
/// tool's name and its ⋯. Returns what was picked.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    if now.state.get("gizmo.tool") == Some(&json!(true)) {
        label(ui, RichText::new("Gizmo"), rec);
        return controls(ui, GIZMO, Fit::Section, false, now, rec);
    }
    let name = now.brush.brush.as_str();
    let shown = ui.button(name);
    rec.widget(Widget::plain(name, Role::Button), shown.rect);
    let shown = hover(shown, Some(HINT), rec);
    options_mark(ui, shown.rect, false);
    let margin = 2.0 * theme(ui.ctx()).spacing.margin;
    let below = ui.ctx().content_rect().bottom() - shown.rect.top() - margin;
    let (id, picked) = popover(ui, &shown, rec, |ui, rec| {
        scrolled(ui, below, rec, |ui, rec| picker(ui, now, rec))
    });
    let picked = picked.flatten();
    if picked.is_some() {
        Popup::close_id(ui.ctx(), id);
    }
    controls(ui, HEAD, Fit::Section, false, now, rec).or(picked)
}

/// The brushes, the pen's brush's family first, each family under its name. Returns what was
/// picked.
fn picker(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let brushes = &now.directions.brushes;
    let own = brushes.iter().find(|b| b.name == now.brush.brush);
    let own = own.map(|b| b.family.as_str());
    let mut picks = now.directions.brush_controls();
    picks.sort_by_key(|c| c.group.as_deref() != own);
    let mut family: Option<String> = None;
    let rows = ui.horizontal_wrapped(|ui| {
        let mut asked = None;
        for c in &picks {
            if c.group != family {
                if family.is_some() {
                    ui.end_row();
                }
                family.clone_from(&c.group);
                let title = family.as_deref().unwrap_or_default();
                label(ui, RichText::new(title).small().weak(), rec);
                ui.end_row();
            }
            asked = control(ui, c, false, now, rec).or(asked);
        }
        asked
    });
    rows.inner
}
