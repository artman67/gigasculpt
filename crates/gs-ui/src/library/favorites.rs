//! The favorites on the shelf (decision 0025, section 13, `favorites` in a layout file): each
//! pinned tile as the same control the tile is, in the order pinned. A favorite whose item no
//! Library tab lists any more is drawn in `text_off` under its arguments; its hover says so and
//! a tap unpins it. With no favorites the widget draws nothing, not even its title.

use egui::{Color32, RichText};
use gs_protocol::Step;
use serde_json::json;

use super::{named, step, tiles};
use crate::pieces::{Now, control, label};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// Draws the favorites, after their title when `headed`, with short labels when `short`.
/// Returns what was tapped.
pub fn favorites(
    ui: &mut egui::Ui,
    headed: bool,
    short: bool,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    if now.layout.favorites.is_empty() {
        return None;
    }
    if headed {
        label(ui, RichText::new("Favorites").strong(), rec);
    }
    let listed = now.direction.map(|d| tiles(d, now)).unwrap_or_default();
    let mut asked = None;
    for favorite in &now.layout.favorites {
        let tile = listed.iter().find(|c| c.step().as_ref() == Some(favorite));
        asked = match tile {
            Some(c) => control(ui, c, short, now, rec),
            None => missing(ui, favorite, rec),
        }
        .or(asked);
    }
    asked
}

/// A favorite no tab lists: its arguments in `text_off`, and a tap unpins it.
fn missing(ui: &mut egui::Ui, favorite: &Step, rec: &mut Recorder) -> Option<Step> {
    let values = favorite.args.as_object().into_iter().flatten();
    let values: Vec<String> = values
        .map(|(_, v)| v.as_str().map_or(v.to_string(), str::to_string))
        .collect();
    let text = if values.is_empty() {
        favorite.command.clone()
    } else {
        values.join(", ")
    };
    let why = format!(
        "No Library tab lists {} any more. Tap to unpin it.",
        named(favorite)
    );
    let unpin = step(json!({ "unpin": favorite }));
    let off = RichText::new(&text).color(theme(ui.ctx()).colors.text_off);
    let button = egui::Button::new(off).fill(Color32::TRANSPARENT);
    let shown = ui.add(button).on_hover_text(&why);
    let widget = Widget::runs(&text, Role::Button, unpin.clone(), None);
    rec.widget(widget, shown.rect);
    shown.clicked().then_some(unpin)
}
