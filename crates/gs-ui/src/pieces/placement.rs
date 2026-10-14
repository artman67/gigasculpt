//! The brush tray's Depth section (decision 0053, section 8, as amended 2026-10-12; decision
//! 0060 section 6, item 6): Brush Placement, a view of where the brush sits against the surface,
//! then the `depth` set's controls that show in their order, a line's after its label, with the
//! Depth Curve in the curve editor under Outer Depth while the Depth Mask is on, and Infinite
//! Depth's X, Y and Z beside its switch while it is on. X, Y and Z each send the whole
//! `depth.axes` triple with their own axis turned, since `brush.set` replaces an array whole, so
//! they are drawn here rather than listed in the set.

use egui::{Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use gs_protocol::Step;
use serde_json::json;

use super::sliders::held;
use super::{Face, Now, Press, control, controls_of, hover, label, pressed_as};
use crate::curve::{Points, Setting, setting};
use crate::direction::Control;
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// Brush Placement's height in points; the brush's circle is a quarter of it in radius.
const HEIGHT: f32 = 72.0;
const IMBED: &str = "brush.depth.imbed";
const MASK: &str = "brush.depth.mask";
const INNER: &str = "brush.depth.inner";
const OUTER: &str = "brush.depth.outer";
const INFINITE: &str = "brush.depth.infinite";
const AXES: &str = "brush.depth.axes";

/// Draws the section. Returns what it asked to run.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let all = controls_of("depth", now);
    // Brush Placement's room, painted once the sliders under it have said what the pen holds.
    let (view, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    ui.end_row();
    let asked = controls(ui, &all, now, rec);
    placement(ui, view, &all, now, rec);
    asked
}

/// The `depth` set's controls that show, in their order, with the Depth Curve and X, Y and Z
/// among them. A row holds one control, or a line's after its label (Mask's switch, Gravity's
/// directions), each by its short label.
fn controls(ui: &mut egui::Ui, all: &[Control], now: &Now, rec: &mut Recorder) -> Option<Step> {
    let shown: Vec<&Control> = all.iter().filter(|c| c.shown(&now.state)).collect();
    let mut asked = None;
    for (i, c) in shown.iter().enumerate() {
        let line = c.line.as_deref();
        let last = i.checked_sub(1).and_then(|j| shown.get(j));
        if let Some(name) = line.filter(|l| !l.is_empty() && last.is_none_or(|b| b.line != c.line))
        {
            label(ui, RichText::new(name), rec);
        }
        asked = control(ui, c, line.is_some(), now, rec).or(asked);
        let shows = c.shows.as_deref();
        if shows == Some(INFINITE) && on(now, INFINITE) {
            asked = axes(ui, now, rec).or(asked);
        }
        if line.is_none() || shown.get(i + 1).is_none_or(|n| n.line != c.line) {
            ui.end_row();
        }
        if shows == Some(OUTER) && on(now, MASK) {
            asked = curve(ui, now, rec).or(asked);
        }
    }
    asked
}

/// Whether the state value `shows` is on.
fn on(now: &Now, shows: &str) -> bool {
    now.state.get(shows) == Some(&json!(true))
}

/// Brush Placement in `rect`, a view only: the surface a line across its middle, the brush's
/// circle raised by Imbed radii, with the Depth Mask on a dot on its axis at Outer Depth and one
/// at Inner Depth, and with Infinite Depth on a dashed line down each side from the view's top
/// to its foot. While the pen holds Imbed's, Inner Depth's or Outer Depth's slider, the view
/// follows the pen.
fn placement(ui: &egui::Ui, rect: Rect, all: &[Control], now: &Now, rec: &mut Recorder) {
    let t = theme(ui.ctx());
    let c = &t.colors;
    let r = HEIGHT / 4.0;
    // A value in radii's points: the one the pen holds on its slider, else the brush's.
    let at = |shows: &str| {
        let slider = all.iter().find(|c| c.shows.as_deref() == Some(shows));
        let held = slider.and_then(|s| held(ui, &s.label));
        let value = now.state.get(shows).and_then(serde_json::Value::as_f64);
        held.or(value.map(|v| v as f32)).unwrap_or_default() * r
    };
    let mark = |name: &str, rect: Rect, rec: &mut Recorder| {
        rec.widget(Widget::plain(name, Role::Label), rect);
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.spacing.radius, c.control);
    let mid = rect.center().y;
    painter.hline(rect.x_range(), mid, Stroke::new(1.0, c.text_dim));
    let center = pos2(rect.center().x, mid - at(IMBED));
    painter.circle_stroke(center, r, Stroke::new(2.0, c.lit));
    mark("Brush Placement", rect, rec);
    let circle = Rect::from_center_size(center, vec2(2.0 * r, 2.0 * r));
    mark("Brush Placement circle", circle, rec);
    if on(now, MASK) {
        for (name, shows) in [("Outer Depth mark", OUTER), ("Inner Depth mark", INNER)] {
            let dot = pos2(center.x, center.y - at(shows));
            painter.circle_filled(dot, 3.0, c.lit);
            mark(name, Rect::from_center_size(dot, vec2(6.0, 6.0)), rec);
        }
    }
    if on(now, INFINITE) {
        for x in [center.x - r, center.x + r] {
            let side = [pos2(x, rect.top()), pos2(x, rect.bottom())];
            painter.extend(Shape::dashed_line(&side, Stroke::new(1.0, c.lit), 4.0, 3.0));
        }
        let reach = Rect::from_x_y_ranges(center.x - r..=center.x + r, rect.y_range());
        mark("Infinite Depth reach", reach, rec);
    }
}

/// Infinite Depth's X, Y and Z: each lit while its model axis is on, and each sending the whole
/// `depth.axes` triple with its own axis turned.
fn axes(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let lit: [bool; 3] = (now.state.get(AXES).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut asked = None;
    for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        let mut turned = lit;
        turned[i] = !turned[i];
        let depth = json!({ "axes": turned });
        let step = with_args("brush.set", json!({ "settings": { "depth": depth } }));
        let press = Press {
            label: axis,
            step: step.clone(),
            key: None,
            lit: lit[i],
            shows: None,
        };
        let hint = format!("Infinite Depth along the model's {axis} axis; with none on, the view");
        if hover(
            pressed_as(ui, press, Face::Label, now, rec),
            Some(&hint),
            rec,
        )
        .clicked()
        {
            asked = Some(step);
        }
    }
    asked
}

/// The Depth Curve in the curve editor (decision 0048): its graph, a line saying which way it
/// runs, then Invert, Copy and Paste; each edit is one `brush.set` of `depth.curve`.
fn curve(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let step = |p: &Points| {
        let depth = json!({ "curve": p.value() });
        with_args("brush.set", json!({ "settings": { "depth": depth } }))
    };
    let depth = Setting {
        names: ["Depth Curve", "Depth Curve point"],
        curve: &now.preset?.depth.curve,
        step: &step,
        legend: "The surface at the left, the band's edges at the right",
        snap: false,
        under: None,
    };
    setting(ui, &depth, now, rec)
}
