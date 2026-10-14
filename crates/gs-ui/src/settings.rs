//! The settings window (decision 0025, sections 2 and 3; decision 0052): the pieces the layout
//! file's `settings` lists, page by page, in a window at the 3D view's top corner on the pen
//! side. `ui.settings` opens it (the header's Window > Settings) and its Close button shuts it
//! with `ui.settings` too, so an agent can do both. It stays open while the pen works the view,
//! so a navigation speed can be tried as it is set. Its controls run the commands that keep the
//! settings (`ui.navigation`, `ui.layout`, `pen.set`, `view.settings`), so the palette finds
//! each of them as well. Its search lists the settings (`listed`) whose words it has, each
//! opening its page, and Reset puts defaults back once its question is answered. It stays
//! inside the 3D view: in a window too short for its pieces they scroll (`floats`), and its
//! foot stays put.

pub mod listed;

use egui::{Align2, RichText, TextStyle, vec2};
use gs_engine::{Hand, ResetScope};
use gs_protocol::Step;
use serde_json::json;

use crate::floats::scrolling;
use crate::pieces::{Dragging, Now, label, stacked};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;
use listed::Setting;

/// The window's width, in points.
const WIDTH: f32 = 400.0;

/// The settings window between frames, as `ui.settings` leaves it (decision 0052).
#[derive(Debug, Default)]
pub struct SettingsWindow {
    pub open: bool,
    /// The page shown, by its title; `None`: the first.
    pub page: Option<String>,
    /// The search text; empty for none.
    pub search: String,
    /// The Reset whose question waits for an answer.
    pub asking: Option<ResetScope>,
}

/// Draws the settings window while it is open, inside the 3D view's rectangle `view` (points),
/// and Reset's question while one waits. Call after the 3D view, so they lie on top. Returns
/// what they asked to run.
pub fn window(
    ui: &egui::Ui,
    (state, view): (&SettingsWindow, egui::Rect),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let mut asked = Vec::new();
    if state.open && now.direction.is_some() {
        asked = shown(ui, (state, view), now, dragging, rec);
    }
    if let Some(scope) = &state.asking {
        asked.extend(question(ui, scope, now, rec));
    }
    asked
}

/// The open window: the search, the page tabs, the page or what the search found, and its foot.
fn shown(
    ui: &egui::Ui,
    (state, view): (&SettingsWindow, egui::Rect),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let t = theme(ui.ctx());
    let inset = t.spacing.margin;
    // The pen side, where the scene tray and its Settings button are.
    let (corner, pivot) = match now.layout.hand {
        Hand::Right => (view.right_top() + vec2(-inset, inset), Align2::RIGHT_TOP),
        Hand::Left => (view.left_top() + vec2(inset, inset), Align2::LEFT_TOP),
    };
    let listed = listed::settings(now);
    let paged = listed::paged(now);
    let pages = listed::pages(&paged);
    let page = (state.page.as_ref())
        .filter(|p| pages.contains(p))
        .or(pages.first())
        .cloned()
        .unwrap_or_default();
    let pieces: Vec<String> = (paged.iter())
        .filter(|(_, p)| *p == page)
        .map(|(piece, _)| (*piece).to_string())
        .collect();
    let found = (!state.search.trim().is_empty()).then(|| listed::found(&listed, &state.search));
    // egui lays a window's contents out in the room of its earlier size, which starts small;
    // given the view's height, the sets have their room from the first frame. The window is
    // still drawn as tall as its contents.
    let tall = (view.height() - 2.0 * inset).max(0.0);
    let mut asked = Vec::new();
    let shown = egui::Window::new("Settings")
        .id(egui::Id::new("settings"))
        .collapsible(false)
        .resizable(false)
        .min_height(tall)
        .max_height(tall)
        .pivot(pivot)
        .fixed_pos(corner)
        .show(ui.ctx(), |ui| {
            // egui's first frame of a window only measures it, unseen and away from its place;
            // drawn again at once where the measure puts it, it shows there from its first frame.
            if ui.is_sizing_pass() {
                ui.ctx().request_discard("the settings window opened");
            }
            // Its rows wrap at this width; egui would otherwise lay each out as one long row.
            ui.set_width(WIDTH);
            rec.panel("settings", "Settings", ui.max_rect());
            asked.extend(search(ui, &state.search, rec));
            asked.extend(tabs(ui, &pages, &page, rec));
            let room = view.bottom() - inset - ui.cursor().top() - foot(ui);
            // Always in a scroll area (its width is the window's, so it narrows nothing): a page
            // too tall for its room is measured scrolled from the first frame, and never moves
            // the window by being drawn whole once.
            let (steps, _) = scrolling(ui, room.max(0.0), rec, |ui, rec| match &found {
                Some(found) => results(ui, found, rec),
                None => stacked(ui, &pieces, now, dragging, rec),
            });
            asked.extend(steps);
            let buttons = [
                ("Reset page", json!({ "reset": { "page": page } })),
                ("Reset all", json!({ "reset": "all" })),
                ("Close", json!({ "open": false })),
            ];
            ui.horizontal(|ui| {
                for (l, args) in buttons {
                    asked.extend(button(ui, l, args, rec));
                }
            });
        });
    if let Some(shown) = shown {
        rec.fit(shown.response.rect);
    }
    asked
}

/// A button running `ui.settings` with `args`, recorded as `label`; what it asked for when
/// pressed.
fn button(
    ui: &mut egui::Ui,
    label: &str,
    args: serde_json::Value,
    rec: &mut Recorder,
) -> Option<Step> {
    let step = with_args("ui.settings", args);
    let pressed = ui.button(label);
    rec.widget(
        Widget::runs(label, Role::Button, step.clone(), None),
        pressed.rect,
    );
    pressed.clicked().then_some(step)
}

/// The search field: typing in it runs `ui.settings` with the new text.
fn search(ui: &mut egui::Ui, text: &str, rec: &mut Recorder) -> Option<Step> {
    let mut typed = text.to_string();
    let field = egui::TextEdit::singleline(&mut typed)
        .id(egui::Id::new("settings-search"))
        .hint_text("Search settings")
        .desired_width(f32::INFINITY);
    let shown = ui.add(field);
    rec.widget(Widget::plain("Search", Role::TextField), shown.rect);
    (typed != text).then(|| with_args("ui.settings", json!({ "search": typed })))
}

/// A tab per page, lit for `page`; a tap shows its page and clears the search.
fn tabs(ui: &mut egui::Ui, pages: &[String], page: &str, rec: &mut Recorder) -> Vec<Step> {
    let mut asked = Vec::new();
    ui.horizontal_wrapped(|ui| {
        for p in pages {
            let step = with_args("ui.settings", json!({ "page": p, "search": "" }));
            let pressed = ui.selectable_label(p == page, p);
            let widget = Widget::runs(p, Role::Button, step.clone(), None).selected(p == page);
            rec.widget(widget, pressed.rect);
            asked.extend(pressed.clicked().then_some(step));
        }
    });
    asked
}

/// What the search found: a row per setting, which opens its page, with its page's name and
/// a Reset of its own.
fn results(ui: &mut egui::Ui, found: &[&Setting], rec: &mut Recorder) -> Vec<Step> {
    if found.is_empty() {
        label(
            ui,
            RichText::new("No setting has all of those words.").weak(),
            rec,
        );
    }
    let mut asked = Vec::new();
    for s in found {
        ui.horizontal(|ui| {
            let open = with_args("ui.settings", json!({ "page": s.page, "search": "" }));
            let row = ui.button(&s.label);
            rec.widget(
                Widget::runs(&s.label, Role::ListItem, open.clone(), None),
                row.rect,
            );
            asked.extend(row.clicked().then_some(open));
            label(ui, RichText::new(&s.page).small().weak(), rec);
            let reset = with_args("ui.settings", json!({ "reset": { "setting": s.name } }));
            let pressed = ui.small_button("Reset");
            let widget = Widget::runs(
                format!("Reset {}", s.label),
                Role::Button,
                reset.clone(),
                None,
            );
            rec.widget(widget, pressed.rect);
            asked.extend(pressed.clicked().then_some(reset));
        });
    }
    asked
}

/// Reset's question, in the middle of the window over everything else: Reset runs it, Cancel
/// lets it go (decision 0052, section 8).
fn question(ui: &egui::Ui, scope: &ResetScope, now: &Now, rec: &mut Recorder) -> Vec<Step> {
    let text = listed::question(scope, &listed::settings(now));
    let mut asked = Vec::new();
    let shown = egui::Window::new("Reset")
        .id(egui::Id::new("reset"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            rec.panel("reset", "Reset", ui.max_rect());
            label(ui, RichText::new(text), rec);
            ui.horizontal(|ui| {
                for (l, confirm) in [("Reset", true), ("Cancel", false)] {
                    asked.extend(button(ui, l, json!({ "confirm": confirm }), rec));
                }
            });
        });
    if let Some(shown) = shown {
        rec.fit(shown.response.rect);
    }
    asked
}

/// What the window needs below its pieces, in points: the foot's row of buttons and the window
/// frame's bottom margin.
fn foot(ui: &egui::Ui) -> f32 {
    let spacing = ui.spacing();
    let text = ui.text_style_height(&TextStyle::Button);
    let button = (text + 2.0 * spacing.button_padding.y).max(spacing.interact_size.y);
    let frame = egui::Frame::window(ui.style());
    spacing.item_spacing.y + button + frame.total_margin().bottom
}
