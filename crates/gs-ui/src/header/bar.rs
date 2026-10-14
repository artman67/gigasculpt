//! The bar itself: the menu titles at one end and the pen end's pieces at the other, on one row
//! when both fit and on two otherwise; the titles folded into one button when they do not fit a
//! row of their own, the pen end then beside it when that makes the bar less tall (decision 0060
//! as amended for H7, item 1).

use egui::{Align, Layout, Margin, Vec2, vec2};
use gs_engine::Hand;
use gs_protocol::Step;
use serde_json::json;

use super::{End, GAP, Menus, PAD, menu, pen, spec};
use crate::direction::{HeaderSpec, title};
use crate::pieces::{Dragging, Face, Now, Press, pressed_as};
use crate::record::{Recorder, with_args};
use crate::theme::theme;

/// The room an end is drawn in past its measured width, in points, so that rounding at another
/// place never wraps a row that measured as fitting.
const SLACK: f32 = 1.0;

/// One row of the bar: the end at its left and the end at its right, each with the width it
/// takes; a pen end narrower than its one row wraps in it.
type Row = [Option<(End, f32)>; 2];

/// Draws the header bar along the top of the room left for the 3D view. Call after the docked
/// panels and before the 3D view, so it is as wide as the view and the view's picture starts
/// under it. Shuts a menu, or a view button's options, the header does not have. Returns what
/// it asked to run.
pub fn bar(
    ui: &mut egui::Ui,
    now: &Now,
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let Some(spec) = spec(now) else {
        menus.shut();
        return Vec::new();
    };
    let has = match (menus.menu(), menus.options()) {
        (Some(menu), _) => spec.menus.iter().any(|m| m == menu),
        (_, Some(id)) => pen::opens(spec, now, id),
        _ => true,
    };
    if !has {
        menus.shut();
    }
    let mut asked = Vec::new();
    let shown = egui::Panel::top(egui::Id::new("view header"))
        .resizable(false)
        .show_separator_line(false)
        .frame(frame(ui))
        .show(ui, |ui| {
            rec.panel("header", "Header", ui.max_rect());
            ui.spacing_mut().item_spacing.y = 2.0 * f32::from(PAD);
            asked = rows(ui, spec, now, menus, dragging, rec);
        });
    rec.fit(shown.response.rect);
    asked
}

/// The bar's frame: the header's color, a tray's margin at the sides and [`PAD`] above and
/// below. The Reference View's bar takes it too.
pub(crate) fn frame(ui: &egui::Ui) -> egui::Frame {
    let t = theme(ui.ctx());
    let side = t.spacing.margin as i8;
    (egui::Frame::new().fill(t.colors.header)).inner_margin(Margin::symmetric(side, PAD))
}

/// The bar's rows (see [`plan`]), each end at its own end. The folded menus' list opens with a
/// menu opened while they are folded, and shuts while they are not. Returns what they asked to
/// run.
fn rows(
    ui: &mut egui::Ui,
    spec: &HeaderSpec,
    now: &Now,
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let (mut plan, tall) = plan(ui, (spec, now), menus, false);
    // Past two rows, the pen end's sets that an options button folds fold into it (decision
    // 0060 as amended for H6: Symmetry in a narrow view).
    let two = 2.0 * ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    if tall > two + SLACK {
        plan = self::plan(ui, (spec, now), menus, true).0;
    }
    let folded = plan
        .iter()
        .flatten()
        .flatten()
        .any(|(end, _)| *end == End::Fold);
    if folded {
        menus.list |= menus.menu().is_some();
    } else {
        (menus.list, menus.fold) = (false, None);
    }
    let mut asked = Vec::new();
    for [left, right] in plan {
        ui.horizontal_top(|ui| {
            let mut draw = |ui: &mut egui::Ui, (end, wide): (End, f32)| {
                put(ui, end, wide + SLACK, (spec, now), menus, dragging, rec)
            };
            if let Some(end) = left {
                asked.extend(draw(ui, end));
            }
            if let Some(end) = right {
                ui.add_space((ui.available_width() - end.1).max(0.0));
                asked.extend(draw(ui, end));
            }
        });
    }
    asked
}

/// Where the bar puts its ends, the menus' end left of the pen end but for the left hand: one
/// row while both fit with [`GAP`] between; else the menus' end over the pen end, or, folded, the
/// pen end wrapping beside the button when that makes the bar less tall (on a tie, under it).
/// The titles fold into the button when they do not fit a row, and the pen end's sets into
/// their options buttons while `fold`. Returns the rows and how tall they are, in points.
fn plan(ui: &mut egui::Ui, at: (&HeaderSpec, &Now), menus: &Menus, fold: bool) -> (Vec<Row>, f32) {
    let (spec, now) = at;
    let room = ui.available_width();
    let titles = (End::Menus, measure(ui, End::Menus, at, menus));
    let first = if titles.1 <= room {
        titles
    } else {
        (End::Fold, measure(ui, End::Fold, at, menus))
    };
    let end = End::Pen(fold);
    let pen = measure(ui, end, at, menus);
    let ends = |a, b| {
        if now.layout.hand == Hand::Left {
            [b, a]
        } else {
            [a, b]
        }
    };
    let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    if spec.show.is_empty() || first.1 + GAP + pen <= room {
        return (vec![ends(Some(first), Some((end, pen)))], row);
    }
    let under = wrapped(ui, end, room, at, menus);
    let beside = room - first.1 - GAP;
    if first.0 == End::Fold && beside > 0.0 {
        let wraps = wrapped(ui, end, beside, at, menus);
        if wraps.y < row + under.y {
            return (vec![ends(Some(first), Some((end, wraps.x)))], wraps.y);
        }
    }
    let rows = vec![ends(Some(first), None), ends(None, Some((end, under.x)))];
    (rows, row + under.y)
}

/// How wide one end of the bar is on one row, in points (see [`one_row`]).
fn measure(ui: &mut egui::Ui, end: End, at: (&HeaderSpec, &Now), menus: &Menus) -> f32 {
    let (mut menus, mut dragging) = (menus.clone(), None);
    one_row(ui, ("header width", end), |ui, rec| {
        side(ui, end, at, &mut menus, &mut dragging, rec);
    })
}

/// How big one end of the bar is wrapping in `width`: its widest row and its height, in points.
fn wrapped(
    ui: &mut egui::Ui,
    end: End,
    width: f32,
    at: (&HeaderSpec, &Now),
    menus: &Menus,
) -> Vec2 {
    let (mut menus, mut dragging) = (menus.clone(), None);
    sized(ui, ("header wraps", end, width.to_bits()), |ui, rec| {
        put(ui, end, width, at, &mut menus, &mut dragging, rec);
    })
}

/// How wide what `add` draws is on one row, in points (see [`sized`]).
pub(crate) fn one_row(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder),
) -> f32 {
    sized(ui, salt, add).x
}

/// How big what `add` draws is, in points: drawn left to right in an invisible pass (salted
/// with `salt`) that takes no input and records nothing.
fn sized(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder),
) -> Vec2 {
    let builder = egui::UiBuilder::new()
        .id_salt(salt)
        .sizing_pass()
        .invisible()
        .layout(Layout::left_to_right(Align::Center));
    let mut child = ui.new_child(builder);
    let mut rec = Recorder::new(ui.ctx().pixels_per_point());
    add(&mut child, &mut rec);
    child.min_rect().size()
}

/// Draws one end of the bar in a block `width` wide, wrapping in it where it does not fit a
/// row. Returns what it asked to run.
fn put(
    ui: &mut egui::Ui,
    end: End,
    width: f32,
    at: (&HeaderSpec, &Now),
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let layout = Layout::left_to_right(Align::Center).with_main_wrap(true);
    let size = vec2(width, ui.spacing().interact_size.y);
    let drawn = ui.allocate_ui_with_layout(size, layout, |ui| {
        // A label (Symmetry's line) moves to the next row whole, as a button does.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        side(ui, end, at, menus, dragging, rec)
    });
    drawn.inner
}

/// Draws one end of the bar: the menu titles, the button they fold into (`menu::fold`), or the
/// pen end's pieces (`pen::end`). Returns what it asked to run.
fn side(
    ui: &mut egui::Ui,
    end: End,
    (spec, now): (&HeaderSpec, &Now),
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    match end {
        End::Menus => titles(ui, (spec, now), menus, false, rec),
        End::Fold => {
            menu::fold(ui, spec, menus, rec);
            Vec::new()
        }
        End::Pen(fold) => pen::end(ui, (spec, now, fold), menus, dragging, rec),
    }
}

/// The menu titles, in the header's order, each lit while its menu is open: a tap opens its
/// menu, or shuts it when it is open (but one the pointer opened, which a tap keeps); while one
/// is open, the pointer over another title opens that one (not while a view button's options
/// are). `listed`, they are the folded menus' list's rows, each with ▸ at its right. Returns
/// what a tap asked for.
pub(super) fn titles(
    ui: &mut egui::Ui,
    (spec, now): (&HeaderSpec, &Now),
    menus: &mut Menus,
    listed: bool,
    rec: &mut Recorder,
) -> Vec<Step> {
    let mut asked = Vec::new();
    for name in &spec.menus {
        let open = menus.menu() == Some(name.as_str());
        let mut args = toml::Table::new();
        args.insert("open".into(), name.as_str().into());
        let to = if open && !menus.hovered {
            json!(false)
        } else {
            json!(name)
        };
        let step = with_args("ui.menu", json!({ "open": to }));
        let press = Press {
            label: title(&now.directions.controls, name),
            key: now.keymap.key_for_args("ui.menu", &args),
            step: step.clone(),
            lit: open,
            shows: None,
        };
        let shown = pressed_as(ui, press, Face::Label, now, rec);
        if listed {
            menu::arrow(ui, shown.rect, open);
        }
        if menus.menu().is_some() && !open && shown.hovered() {
            menus.show(name, true);
        }
        if menus.menu() == Some(name.as_str()) {
            menus.at = Some(shown.rect);
        }
        if shown.clicked() {
            asked.push(step);
        }
    }
    asked
}
