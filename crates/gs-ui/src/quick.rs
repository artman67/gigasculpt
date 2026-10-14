//! The quick menu (decision 0025, section 2; catalog `interface.quick-menu`): a few pieces,
//! each under its title, in a small window whose corner is at the pointer. `ui.quick` opens it
//! (Shift+Space, or a barrel click set to `quick`) and closes it. It stays open while its
//! controls are used, so several can be set in a row, and closes on a press outside it or
//! Escape. It reaches controls without being their home (rule 1): each piece is also in a
//! tray. It stays inside the 3D view, and in a view too short for it its pieces scroll
//! (`floats`).
//!
//! It has up to five menus. At first there is one, the layout file's `quick` list; `ui.quick`
//! `add` and `remove` change what a menu holds and `menu` with `add` starts another, all kept
//! in the layout state. With more than one, a button for each heads the menu, and holding the
//! key that opens it while the pointer swipes across shows the next menu or the one before.
//!
//! While it shows, the wheel sizes the brush (`quick/wheel.rs`).
//!
//! Its window can hold one slider alone instead (`ui.quick` `slider`, as a short tap of a
//! slider key shows it; decision 0025, amended 2026-10-12): no menu buttons and no swipe, the
//! wheel steps that slider, and the window opens with the slider's knob under the pointer, at
//! the value it holds, so a drag moves it from there at once. Near the screen's edge the knob
//! stays under the pointer and the window runs past the edge.

mod wheel;

pub use wheel::Turns;

use egui::{Align2, Pos2};
use gs_engine::UiQuick;
use gs_protocol::Step;
use serde_json::json;

use crate::direction::{Control, Controls};
use crate::floats::scrolled;
use crate::keymap::Chord;
use crate::layout::Layout;
use crate::pieces::{Dragging, Now, Press, control, knob, run_button, section, stacked};
use crate::record::{Recorder, with_args};
use crate::theme::theme;
use crate::values::Value;

/// The menu's width, in points: room for a slider and its value.
const WIDTH: f32 = 300.0;

/// How far, in points, the pointer swipes across with the menu's key held to show the next
/// menu (to the right) or the one before (to the left).
const SWIPE: f32 = 60.0;

/// The buttons that show each menu.
const TABS: [&str; 5] = ["Menu 1", "Menu 2", "Menu 3", "Menu 4", "Menu 5"];

/// How far up and left of the pointer the menu's corner sits, in points: enough that the
/// pointer is inside it once egui rounds the corner to a pixel, so the pen is over the window
/// and the cursor's ring and text, which would cover its first row, step aside
/// (`Shell::over_view`). A slider alone is placed by its knob instead.
const INSIDE: f32 = 1.0;

/// The most quick menus there are.
pub const MOST: usize = TABS.len();

/// The quick menu while it shows: where, the slider it holds alone if it does, and the part of
/// a wheel notch not stepped yet.
#[derive(Debug)]
pub struct Open {
    /// Where it opened, the pointer then, in points: the menu's corner is by it, a slider
    /// alone's knob under it.
    pub at: Pos2,
    /// The slider shown alone, by its label; `None`, the menu.
    pub slider: Option<String>,
    /// Part of a notch turned, waiting for the rest.
    turned: f32,
    /// A slider alone's corner, in points, once its first frame has found its knob: where the
    /// knob lies under `at`.
    placed: Option<Pos2>,
}

impl Open {
    /// The window opening at `at`, holding the menu or the slider `slider`.
    pub fn new(at: Pos2, slider: Option<String>) -> Self {
        Self {
            at,
            slider,
            turned: 0.0,
            placed: None,
        }
    }
}

/// The brush value whose slider is labelled `label`: `Size`, `Strength` or `Softness`.
pub fn brush_value(label: &str) -> Option<Value> {
    let all = [Value::Size, Value::Strength, Value::Falloff];
    all.into_iter().find(|v| v.title() == label)
}

/// The slider in the controls files labelled `label`.
pub fn slider_control<'c>(controls: &'c Controls, label: &str) -> Option<&'c Control> {
    let all = controls.sets.iter().flat_map(|s| &s.controls);
    all.filter(|c| c.dragged().is_some())
        .find(|c| c.label == label)
}

/// Whether a slider is labelled `label` (see [`brush_value`] and [`slider_control`]).
pub fn is_slider(controls: &Controls, label: &str) -> bool {
    brush_value(label).is_some() || slider_control(controls, label).is_some()
}

/// What each quick menu holds: the user's menus in `layout`, or one, the layout file's list
/// `quick`.
pub fn menus(layout: &Layout, quick: &[String]) -> Vec<Vec<String>> {
    if layout.quick.is_empty() {
        vec![quick.to_vec()]
    } else {
        layout.quick.clone()
    }
}

/// The menu shown, from 0: the one picked, or the last of `count` when fewer remain.
pub fn shown(layout: &Layout, count: usize) -> usize {
    layout.quick_menu.min(count.saturating_sub(1))
}

/// Makes `change` to the menus in `layout` (see [`menus`]): the menu shown, and a piece added
/// to or taken out of it, or every menu as shipped. `knows` says whether a piece names
/// something to show.
pub fn change(
    layout: &mut Layout,
    quick: &[String],
    change: &UiQuick,
    knows: impl Fn(&str) -> bool,
) -> Result<(), String> {
    if change.reset {
        (layout.quick, layout.quick_menu) = (Vec::new(), 0);
    }
    let mut menus = menus(layout, quick);
    let count = menus.len();
    let mut at = shown(layout, count);
    if let Some(n) = change.menu {
        if n > count + 1 || (n > count && change.add.is_none()) {
            let next = count + 1;
            return Err(format!(
                "there are {count} quick menus; `menu` {next} with `add` starts another"
            ));
        }
        at = n - 1;
    }
    if let Some(step) = change.step {
        at = if step > 0 { at + 1 } else { at + count - 1 } % count;
    }
    if let Some(piece) = &change.add {
        if !knows(piece) {
            return Err(format!(
                "nothing called `{piece}` to add; give a set, `set/Group`, `brushes/Family` or \
                 a shared widget, as a layout file names them"
            ));
        }
        if at == count {
            menus.push(Vec::new());
        }
        if !menus[at].contains(piece) {
            menus[at].push(piece.clone());
        }
        layout.quick.clone_from(&menus);
    }
    if let Some(piece) = &change.remove {
        let menu = &mut menus[at];
        if !menu.contains(piece) {
            let holds = menu.join(", ");
            return Err(format!(
                "menu {} holds no `{piece}`; it holds {holds}",
                at + 1
            ));
        }
        menu.retain(|p| p != piece);
        if menu.is_empty() && menus.len() > 1 {
            menus.remove(at);
            at = at.min(menus.len() - 1);
        }
        layout.quick = menus;
    }
    layout.quick_menu = at;
    Ok(())
}

/// The quick menu's egui area.
pub(crate) const AREA: &str = "quick menu";

/// Whether `ui` draws inside the quick menu, which shows the open deformer's Accept, Cancel and
/// Reset at its head, so the Gizmo's set inside it leaves its own out.
pub(crate) fn inside(ui: &egui::Ui) -> bool {
    ui.layer_id().id == egui::Id::new(AREA)
}

/// Draws the quick menu, or the slider it holds alone, while it is open at `at`, the menu inside
/// the 3D view's rectangle `view` (points). Call after the 3D view, as for everything at the
/// pen, so it lies on top. Shuts it (`at` becomes `None`) on a press outside it; Escape shuts it
/// too, given by the window (`shell/escape.rs`). Returns what its controls asked to run.
pub fn menu(
    ui: &egui::Ui,
    (at, view): (&mut Option<Open>, egui::Rect),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let (Some(open), Some(direction)) = (at.as_ref(), now.direction) else {
        return Vec::new();
    };
    let (corner, alone, placed) = (open.at, open.slider.clone(), open.placed);
    let menus = menus(now.layout, &direction.quick);
    let shown = shown(now.layout, menus.len());
    if alone.is_none() && menus[shown].is_empty() {
        *at = None;
        return Vec::new();
    }
    let t = theme(ui.ctx());
    let mut asked = Vec::new();
    let room = view.shrink(t.spacing.margin);
    let area = egui::Area::new(egui::Id::new(AREA))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_TOP);
    // The menu by the pointer, moved inside the view as far as it needs (up from the pointer
    // near the view's foot), never over the status line, the shelf or a tray. A slider alone
    // where its knob lies under the pointer, wherever that puts the window; until its knob is
    // found, where it opened.
    let area = if alone.is_some() {
        area.fixed_pos(placed.unwrap_or(corner)).constrain(false)
    } else {
        let corner = corner - egui::vec2(INSIDE, INSIDE);
        area.fixed_pos(corner).constrain_to(room)
    };
    knob(ui.ctx());
    let shown_area = area.show(ui.ctx(), |ui| {
        let frame = egui::Frame::popup(ui.style()).inner_margin(t.spacing.margin);
        let margins = frame.total_margin().sum().y;
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            rec.panel("quick", "Quick menu", ui.max_rect());
            if let Some(label) = &alone {
                let one = ui.horizontal_wrapped(|ui| slider(ui, label, now, dragging, rec));
                asked.extend(one.inner);
                return;
            }
            asked.extend(crate::gizmo::deformer::quick_buttons(ui, now, rec));
            if menus.len() > 1 {
                asked.extend(tabs(ui, menus.len(), shown, now, rec));
            }
            // The pieces' room: the view's, less the rows above them.
            let above = ui.cursor().top() - ui.max_rect().top();
            let most = room.height() - margins - above;
            asked.extend(scrolled(ui, most, rec, |ui, rec| {
                stacked(ui, &menus[shown], now, dragging, rec)
            }));
        });
    });
    let rect = shown_area.response.rect;
    let found = knob(ui.ctx()).filter(|_| alone.is_some() && placed.is_none());
    if let (Some(knob), Some(open)) = (found, at.as_mut()) {
        open.placed = Some(corner - (knob - rect.min));
        // egui draws the frame again at once, the window in its place.
        ui.ctx()
            .request_discard("a slider alone opens with its knob under the pointer");
        ui.ctx().request_repaint();
    }
    rec.fit(rect);
    if alone.is_none() {
        asked.extend(swipe(ui, now));
    }
    let shut = ui.input(|i| {
        let origin = i.pointer.press_origin();
        i.pointer.any_pressed() && origin.is_some_and(|p| !rect.contains(p))
    });
    if shut {
        *at = None;
    }
    asked
}

/// The slider labelled `label` alone (see [`is_slider`]). Returns what it asked to run.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    if let Some(v) = brush_value(label) {
        return section(ui, v.name(), now, dragging, rec);
    }
    let c = slider_control(&now.directions.controls, label)?;
    control(ui, c, false, now, rec)
}

/// A button for each of `count` menus, the one `shown` lit; each shows its menu.
fn tabs(ui: &mut egui::Ui, count: usize, shown: usize, now: &Now, rec: &mut Recorder) -> Vec<Step> {
    let row = ui.horizontal(|ui| {
        (0..count)
            .filter_map(|i| {
                let press = Press {
                    label: TABS[i.min(TABS.len() - 1)],
                    step: with_args("ui.quick", json!({ "menu": i + 1 })),
                    key: None,
                    lit: i == shown,
                    shows: None,
                };
                run_button(ui, press, now, rec)
            })
            .collect()
    });
    row.inner
}

/// While the key that opens the menu is held, a swipe of the pointer across it shows the next
/// menu or the one before (`ui.quick` `step`). Where a swipe starts is egui's to keep, as a
/// scroll's is: it changes nothing an agent acts on.
fn swipe(ui: &egui::Ui, now: &Now) -> Option<Step> {
    let open = toml::Table::from_iter([("open".into(), toml::Value::Boolean(true))]);
    let bound = now.keymap.key_for_args("ui.quick", &open);
    let key = bound.and_then(|k| Chord::parse(k).ok())?.key.as_key()?;
    let id = egui::Id::new("quick swipe");
    let x = ui.input(|i| i.key_down(key).then(|| i.pointer.latest_pos()).flatten());
    let Some(x) = x.map(|p| p.x) else {
        ui.data_mut(|d| d.remove::<f32>(id));
        return None;
    };
    let start = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || x));
    let moved = x - start;
    if moved.abs() < SWIPE {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(id, x));
    let step = if moved > 0.0 { 1 } else { -1 };
    Some(with_args("ui.quick", json!({ "step": step })))
}
