//! How every slider is drawn ([`rail`]): its rail with its title and value fitted to its row,
//! and the ways it sets its value (a drag, a typed value, the fine handle, the wheel).

use egui::{CursorIcon, PointerButton, Sense, TextEdit, TextWrapMode};

use super::{Moved, Slider};
use crate::shell::escape::{To, given};

/// The typing field's width, in points: room for "10000".
const FIELD: f32 = 64.0;

/// How far a rail narrows for a label too wide for the room beside it, in points: a digit or
/// two. A label further over goes above its rail (decision 0025, rule 12).
const NARROWS: f32 = 24.0;

/// A wheel notch moves a slider this share of its rail, and a tenth of it with Shift held.
const NOTCH: f32 = 0.01;

/// The fine handle moves a value this share of what the rail does for the same drag.
const FINE: f32 = 0.1;

/// The center snap's dead zone on each side of 0: this share of the rail.
const CENTER: f32 = 0.02;

/// Where egui keeps the knob of the last slider drawn ([`knob`]).
const KNOB: &str = "slider knob";

/// Where the last slider drawn put its knob (its rail's handle), in points, taken so a later
/// slider is not mistaken for it: the quick menu opens a slider alone with its knob under the
/// pointer (decision 0025 section 2, as amended 2026-10-12).
pub(crate) fn knob(ctx: &egui::Context) -> Option<egui::Pos2> {
    ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(egui::Id::new(KNOB)))
}

/// Where a drag started: the value then, and whether it goes to the slider's stops.
#[derive(Clone, Copy)]
struct Grab {
    from: f32,
    stops: bool,
}

/// A slider's row as drawn: its rail with the value a drag let go at, a value typed, and the
/// typing field while it is open.
type Row = (
    (egui::Response, Option<f32>),
    Option<f32>,
    Option<egui::Rect>,
);

/// A slider's open fine handle: where the slider and the handle were drawn, so a press off
/// them shuts it, and in which pass, so one left open by a slider no longer drawn is shut when
/// the slider draws again.
#[derive(Clone, Copy)]
struct Fine {
    area: egui::Rect,
    pass: u64,
}

/// Draws a slider: its rail and its title and value ([`row`]), which a tap turns into a field to
/// type a value in, and under them its fine handle while it is open. Returns what was done with
/// it.
pub(in crate::pieces) fn rail(ui: &mut egui::Ui, s: &Slider) -> Moved {
    let mut value = s.value;
    let fine_id = s.id.with("fine");
    let fine_open = fine_open(ui, fine_id);
    let column = ui.vertical(|ui| {
        let row = row(ui, s, &mut value);
        // As long as the rail, which narrows for a wide value, so it moves a tenth as far.
        ui.spacing_mut().slider_width = row.inner.0.0.rect.width();
        let fine = fine_open.then(|| track(ui, s, FINE, &mut value));
        (row, fine)
    });
    let (row, fine) = column.inner;
    let ((rail, let_go), typed, field) = row.inner;
    let (fine, fine_let_go) = fine.unzip();
    let set = let_go.or(fine_let_go.flatten()).or(typed);
    let held = rail.dragged() || fine.as_ref().is_some_and(egui::Response::dragged);
    let area = column.response.rect;
    if fine.is_some() {
        let pass = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|d| d.insert_temp(fine_id, Fine { area, pass }));
        ui.data_mut(|d| d.insert_temp(egui::Id::new(FINE_SHOWN), pass));
    }
    if !s.settings.show_values {
        ui.data_mut(|d| {
            d.insert_temp(s.id.with("row"), row.response.rect);
            d.insert_temp(s.id.with("held"), held);
        });
    }
    let reset = double_click(ui, s, area);
    let set = set.or_else(|| wheel(ui, row.response.contains_pointer(), s));
    Moved {
        held: held.then_some(value),
        set,
        reset,
        rect: row.response.rect,
        fine: fine.map(|f| f.rect),
        field,
    }
}

/// A slider's row, fitted to the room it has (decision 0025, rule 12): its rail with its label
/// beside it ([`beside`]), or else its label above a rail as wide as the rail or the row, the
/// label wrapping only where it is wider than the row by itself, so the row never widens its
/// panel. Returns the row, with the rail (and a value its drag let go at), a value typed, and
/// the typing field while it is open.
fn row(ui: &mut egui::Ui, s: &Slider, value: &mut f32) -> egui::InnerResponse<Row> {
    if let Some(width) = beside(ui, s) {
        ui.spacing_mut().slider_width = width;
        return ui.horizontal(|ui| {
            let rail = track(ui, s, 1.0, value);
            let (set, field) = label(ui, s, TextWrapMode::Extend);
            (rail, set, field)
        });
    }
    ui.vertical(|ui| {
        let width = ui.spacing().slider_width.min(ui.available_width());
        let (set, field) = ui.horizontal(|ui| label(ui, s, TextWrapMode::Wrap)).inner;
        ui.spacing_mut().slider_width = width;
        (track(ui, s, 1.0, value), set, field)
    })
}

/// The rail's width with the label beside it, when the row has room for both (decision 0025,
/// rule 12): the theme's, narrowed by up to [`NARROWS`] for the widest label the slider shows,
/// with its value at either end of its rail or as it is, so a drag never changes the row. None
/// when the row has not.
fn beside(ui: &egui::Ui, s: &Slider) -> Option<f32> {
    let [low, high] = &s.ends;
    let labels = [
        s.label.clone(),
        format!("{}  {low}", s.title),
        format!("{}  {high}", s.title),
    ];
    let wide = |text: String| crate::text::width(ui, text, egui::TextStyle::Body);
    let label = labels.map(wide).into_iter().fold(0.0, f32::max);
    let room = ui.available_width() - label - ui.spacing().item_spacing.x;
    let width = ui.spacing().slider_width;
    (room >= width - NARROWS).then_some(room.min(width))
}

/// A slider's title and value (only its title with show values off, but under the pen or while
/// dragged), laid out with `wrap`, which a tap turns into a field to type a value in. Returns a
/// value typed, and the field while it is open.
fn label(ui: &mut egui::Ui, s: &Slider, wrap: TextWrapMode) -> (Option<f32>, Option<egui::Rect>) {
    if let Some(text) = ui.data(|d| d.get_temp::<String>(s.id)) {
        let (field, set) = type_in(ui, s, text);
        return (set, Some(field));
    }
    let under = |r: egui::Rect| ui.input(|i| i.pointer.hover_pos().is_some_and(|p| r.contains(p)));
    let over = (ui.data(|d| d.get_temp::<egui::Rect>(s.id.with("row")))).is_some_and(under);
    // Whether a drag held it at the last frame: the label above a rail is drawn before it.
    let held = ui.data(|d| d.get_temp::<bool>(s.id.with("held")));
    let text = if s.settings.show_values || over || held == Some(true) {
        s.label.as_str()
    } else {
        s.title
    };
    let label = egui::Label::new(text)
        .wrap_mode(wrap)
        .selectable(false)
        .sense(Sense::click());
    let label = ui.add(label).on_hover_cursor(CursorIcon::Text);
    if label.clicked() {
        ui.data_mut(|d| d.insert_temp(s.id, String::new()));
        ui.memory_mut(|m| m.request_focus(s.id.with("field")));
    }
    (None, None)
}

/// Draws a rail, which a drag moves the value along from where it is (a press does not move
/// it): the rail's width takes it `rate` of the way from one end to the other. Returns it, and
/// the value to set once a drag that moved the value lets go (a tap sets nothing).
fn track(
    ui: &mut egui::Ui,
    s: &Slider,
    rate: f32,
    value: &mut f32,
) -> (egui::Response, Option<f32>) {
    let height = (ui.text_style_height(&egui::TextStyle::Body)).max(ui.spacing().interact_size.y);
    let size = egui::vec2(ui.spacing().slider_width, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::drag());
    let range = rect.x_range().shrink(height / 2.5);
    let id = response.id.with("grab");
    let shift = ui.input(|i| i.modifiers.shift);
    if response.drag_started() {
        let stops = shift && !s.stops.is_empty();
        ui.data_mut(|d| {
            d.insert_temp(
                id,
                Grab {
                    from: s.value,
                    stops,
                },
            )
        });
    }
    let grab = ui.data(|d| d.get_temp::<Grab>(id));
    let held = grab.filter(|_| response.dragged());
    let pointer = ui.input(|i| i.pointer.interact_pos().zip(i.pointer.press_origin()));
    if let (Some(grab), Some((to, from))) = (held, pointer) {
        let moved = (to.x - from.x) / range.span() * rate;
        *value = dragged(s, grab, moved, (shift, rate));
    }
    let from = held.map_or(*value, |g| g.from);
    let knob = paint(ui, &response, range, place(s, *value, from, rate));
    if rate >= 1.0 {
        ui.data_mut(|d| d.insert_temp(egui::Id::new(KNOB), knob));
    }
    let moved = grab.filter(|g| response.drag_stopped() && value.total_cmp(&g.from).is_ne());
    (response, moved.map(|_| *value))
}

/// Where a drag takes a slider: `moved` along its rail (a share of it, on the rail's own scale)
/// from where the drag started; from past an end (a typed value) it moves back toward the rail
/// only. With `shift`, to the nearest stop if the drag started with Shift on a slider with
/// stops, else rounded to whole numbers in the units shown. Without, the center snap holds it at
/// 0 near there, over a fiftieth of the rail (of a fine handle's, at its `rate`).
fn dragged(s: &Slider, grab: Grab, moved: f32, (shift, rate): (bool, f32)) -> f32 {
    let [low, high] = s.rail;
    let span = along(s.log, high) - along(s.log, low);
    let bounds = |v: f32| v.clamp(low.min(grab.from), high.max(grab.from));
    let raw = bounds(at(s.log, along(s.log, grab.from) + moved * span));
    if shift && grab.stops {
        let off = |v: &&f32| (**v - raw).abs();
        let nearest = s.stops.iter().min_by(|a, b| off(a).total_cmp(&off(b)));
        return nearest.copied().unwrap_or(raw);
    }
    if shift {
        let step = whole(s);
        return bounds((raw * s.per / step).round() * step / s.per);
    }
    let zone = CENTER * (high - low) * rate;
    let through = low < 0.0 && high > 0.0;
    if s.settings.center_snap && through && raw.abs() < zone {
        0.0
    } else {
        raw
    }
}

/// What Shift rounds a slider's value to, in the units it shows: 1, or a tenth for each tenth a
/// rail spanning less than ten falls short (orbit speed's 0.1 to 2 rounds to tenths).
fn whole(s: &Slider) -> f32 {
    let span = (s.rail[1] - s.rail[0]) * s.per;
    let mut step = 1.0;
    while span < step * 10.0 && step > 1e-6 {
        step /= 10.0;
    }
    step
}

/// Where along its rail, from 0 to 1, a slider's handle sits at `value`. A fine handle's rail
/// covers `rate` of the slider's around where its drag started (`from`), so it rests in the
/// middle.
fn place(s: &Slider, value: f32, from: f32, rate: f32) -> f32 {
    let [low, high] = s.rail;
    let span = along(s.log, high) - along(s.log, low);
    if rate < 1.0 {
        0.5 + (along(s.log, value) - along(s.log, from)) / (span * rate)
    } else {
        (along(s.log, value.clamp(low, high)) - along(s.log, low)) / span
    }
}

/// A value on a rail's own scale, and back: logarithmic or not.
fn along(log: bool, v: f32) -> f32 {
    if log { libm::logf(v) } else { v }
}

fn at(log: bool, t: f32) -> f32 {
    if log { libm::expf(t) } else { t }
}

/// Paints a rail and its handle at `place` (0 to 1 along `range`), as egui paints its sliders.
/// Returns the handle's center.
fn paint(ui: &egui::Ui, response: &egui::Response, range: egui::Rangef, place: f32) -> egui::Pos2 {
    let rect = response.rect;
    let half = (ui.spacing().slider_rail_height / 2.0).max(0.0);
    let rail = egui::Rect::from_x_y_ranges(
        rect.x_range(),
        egui::Rangef::new(rect.center().y - half, rect.center().y + half),
    );
    let widgets = &ui.visuals().widgets;
    let corner = widgets.inactive.corner_radius;
    ui.painter()
        .rect_filled(rail, corner, widgets.inactive.bg_fill);
    let center = egui::pos2(
        range.min + place.clamp(0.0, 1.0) * range.span(),
        rail.center().y,
    );
    if ui.visuals().slider_trailing_fill {
        let mut fill = rail;
        fill.max.x = center.x + f32::from(corner.nw);
        let color = ui.visuals().selection.bg_fill;
        ui.painter().rect_filled(fill, corner, color);
    }
    let look = ui.style().interact(response);
    let radius = rect.height() / 2.5 + look.expansion;
    ui.painter()
        .circle(center, radius, look.bg_fill, look.fg_stroke);
    center
}

/// Whether a slider's fine handle is open (since a double-click on the slider). A press off the
/// slider and the handle, Escape given to it or to the typing field it opened with
/// (`shell/escape.rs`), or a frame without the slider shuts it.
fn fine_open(ui: &egui::Ui, id: egui::Id) -> bool {
    let Some(fine) = ui.data(|d| d.get_temp::<Fine>(id)) else {
        return false;
    };
    let off = |p: egui::Pos2| !fine.area.contains(p);
    let pressed =
        ui.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(off));
    let shut = pressed || given(ui.ctx(), To::Fine) || given(ui.ctx(), To::Field);
    if shut || fine.pass + 1 < ui.ctx().cumulative_pass_nr() {
        ui.data_mut(|d| d.remove::<Fine>(id));
        return false;
    }
    true
}

/// The pass any slider's fine handle last showed in.
const FINE_SHOWN: &str = "fine handle shown";

/// Whether a slider's fine handle showed in this pass or the last, so Escape shuts it before
/// it does anything else (decision 0041, amended 2026-10-13 for `DF7c-notes`, section 3; the
/// `Fine` row of `shell/escape.rs`).
pub(crate) fn fine_shown(ctx: &egui::Context) -> bool {
    let pass = ctx.data(|d| d.get_temp::<u64>(egui::Id::new(FINE_SHOWN)));
    pass.is_some_and(|p| p + 1 >= ctx.cumulative_pass_nr())
}

/// A double-click on a slider (`area`, its fine handle with it) opens its typing field and its
/// fine handle. With Ctrl held it shuts them instead and answers true: put the default back.
fn double_click(ui: &egui::Ui, s: &Slider, area: egui::Rect) -> bool {
    // Only where the slider shows: a tray cut short leaves it running on under the 3D view.
    let shown = area.intersect(ui.clip_rect());
    let (double, ctrl) = ui.input(|i| {
        let on = i.pointer.interact_pos().is_some_and(|p| shown.contains(p));
        let double = on && i.pointer.button_double_clicked(PointerButton::Primary);
        (double, i.modifiers.ctrl)
    });
    if !double {
        return false;
    }
    let field = s.id.with("field");
    if ctrl {
        ui.data_mut(|d| {
            d.remove::<String>(s.id);
            d.remove::<Fine>(s.id.with("fine"));
        });
        ui.memory_mut(|m| m.surrender_focus(field));
        return true;
    }
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| {
        d.insert_temp(s.id.with("fine"), Fine { area, pass });
        if d.get_temp::<String>(s.id).is_none() {
            d.insert_temp(s.id, String::new());
        }
    });
    ui.memory_mut(|m| m.request_focus(field));
    false
}

/// The field a value is typed in, open with `text` typed so far: what it sets, once, when Enter
/// or a press elsewhere closes it, with where it is drawn. Escape closes it setting nothing, and
/// only it, so the options or menu the slider is in stay open (the `Field` row of
/// `shell/escape.rs`). An empty field shows the value faintly; a typed value is in the units the
/// slider shows, with them or without.
fn type_in(ui: &mut egui::Ui, s: &Slider, mut text: String) -> (egui::Rect, Option<f32>) {
    ui.label(s.title);
    let field = TextEdit::singleline(&mut text)
        .id(s.id.with("field"))
        .hint_text(&s.number)
        .desired_width(FIELD);
    let field = ui.add(field);
    // A press elsewhere closes it at once, before a menu the press shuts stops drawing it.
    if field.has_focus() && ui.input(|i| i.pointer.any_pressed()) && !field.hovered() {
        field.surrender_focus();
    }
    if field.has_focus() {
        ui.data_mut(|d| d.insert_temp(s.id, text));
        return (field.rect, None);
    }
    // Closed, or left without focus where it was not drawn (its menu shut).
    ui.data_mut(|d| d.remove::<String>(s.id));
    if !field.lost_focus() || given(ui.ctx(), To::Field) {
        return (field.rect, None);
    }
    let number = text
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c == '%');
    let typed = number
        .trim_end()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite());
    (field.rect, typed.map(|v| v / s.per))
}

/// The wheel turned over a slider (`over`): it steps a hundredth of the rail a notch, a
/// thousandth with Shift held, from where the slider is, and stops at the rail's ends; from past
/// an end (a typed value) it steps back toward it only. The slider takes the wheel, so a tray or
/// menu under it does not scroll.
fn wheel(ui: &egui::Ui, over: bool, s: &Slider) -> Option<f32> {
    if !over {
        return None;
    }
    let notches: f32 = ui.input(|i| {
        (i.raw.events.iter())
            .filter_map(|e| match e {
                egui::Event::MouseWheel {
                    delta, modifiers, ..
                } => Some(if modifiers.shift {
                    delta.y / 10.0
                } else {
                    delta.y
                }),
                _ => None,
            })
            .sum()
    });
    ui.ctx()
        .input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    if notches == 0.0 {
        return None;
    }
    let [low, high] = s.rail;
    let span = along(s.log, high) - along(s.log, low);
    let to = at(s.log, along(s.log, s.value) + notches * NOTCH * span);
    let to = if s.value > high {
        to.min(s.value)
    } else if s.value < low {
        to.max(s.value)
    } else {
        to.clamp(low, high)
    };
    ((to - s.value).abs() > 0.0).then_some(to)
}
