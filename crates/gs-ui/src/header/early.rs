//! The press a menu takes first: while one is open, or a view button's options, Escape and a
//! press outside it shut it.

use gs_engine::PanelInfo;

use super::Menus;
use crate::descriptions;

/// While a menu, or a view button's options, is open, takes Escape and a press outside it out
/// of `events` before anything sees them, and shuts it: that press does nothing else, so it
/// starts no stroke and presses no button (decision 0060, section 2, and as amended for H5). A
/// press on a menu title, or on the button the menus fold into, is left to it, which opens its
/// menu or list or shuts what is open; the folded menus' list counts as inside (decision 0060 as
/// amended for H7). A press on what the open panel draws past its edge (a color picker) is
/// inside it. Escape while the pen, pressed inside, has not let go is left to what it pressed
/// (a dropdown panel's slider, whose pull it cancels) and shuts nothing (D1). `panels` are as
/// recorded last frame, in pixels; `ppp` pixels to a point.
pub fn early(menus: &mut Menus, events: &mut Vec<egui::Event>, panels: &[PanelInfo], ppp: f32) {
    if !menus.shows() {
        return;
    }
    let is_title =
        |w: &gs_engine::WidgetInfo| (w.command.as_ref()).is_some_and(|s| s.command == "ui.menu");
    let on_menu = |at: egui::Pos2| {
        if menus.fold.is_some_and(|fold| fold.contains(at)) {
            return true;
        }
        let at = egui::pos2(at.x * ppp, at.y * ppp);
        let within = |p: &PanelInfo| {
            let drawn = (p.widgets.iter()).any(|w| descriptions::inside(w.rect, at));
            descriptions::inside(p.rect, at) || drawn
        };
        panels.iter().any(|p| match p.id.as_str() {
            "menu" | "submenu" | "menus" => within(p),
            "header" => (p.widgets.iter()).any(|w| is_title(w) && descriptions::inside(w.rect, at)),
            id => menus.options() == Some(id) && within(p),
        })
    };
    let (before, mut held) = (events.len(), menus.held);
    events.retain(|e| match e {
        egui::Event::Key {
            key: egui::Key::Escape,
            pressed: true,
            ..
        } => held.is_some(),
        egui::Event::PointerButton { pressed, pos, .. } => {
            held = Some(*pos).filter(|at| *pressed && on_menu(*at));
            held.is_some() || !*pressed
        }
        _ => true,
    });
    menus.held = held;
    if events.len() < before {
        menus.shut();
    }
}
