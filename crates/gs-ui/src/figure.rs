//! The corner orientation figure (decision 0017): the scene's three axes as `main` sees them,
//! X red, Y green, Z blue, the positive ends labelled. It sits in the 3D view's top corner on
//! the pen side, under the header, and moves with it for the left hand (decision 0025; decision
//! 0060 as amended for H5, item 4). Clicking an end looks from that side,
//! and clicking it again from the opposite side; a drag that starts on it orbits. An axis
//! tripod like Blender's navigation gizmo, not a copy of Autodesk's view cube, which a patent
//! covers (research 14, 3.2). A live figure of the parts is research 14's secondary views.
//!
//! Under it, two drag buttons for the pen alone (decision 0017 as amended, research 21, 1.2):
//! Zoom (drag up to come closer) and Pan. They are not controls of the controls file: a press
//! and drag on one is a gesture, the same rows a drag on the canvas runs (`navigate`).

use egui::{Pos2, Rect, Sense, Stroke, Vec2};
use gs_engine::{Hand, MainView};
use gs_protocol::Step;
use serde_json::json;

use crate::navigate::Row;
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// From the figure's centre to an axis end, in points.
const RADIUS: f32 = 44.0;
/// An end's disc, in points: 32 across, a pen target.
const END: f32 = 16.0;
/// From the view's top corner on the pen side to the figure's centre, in points.
const INSET: f32 = 68.0;
/// The drag buttons under the figure, top to bottom, and the row a drag on each runs.
pub const PADS: [(&str, Row); 2] = [("Zoom", Row::Dolly), ("Pan", Row::Pan)];
/// Between the figure and the first drag button, and between the buttons, in points.
const GAP: f32 = 8.0;
/// A drag button's width, in points: as high as an end's disc and wide enough for its word.
const PAD_WIDTH: f32 = 52.0;

/// Each axis end's label and the view a click asks for, in the order X, -X, Y, -Y, Z, -Z, so
/// end `i` lies on scene axis `i / 2` (colored by the theme) at its positive end when `i` is
/// even. Looking from an axis's positive end shows the right, top and front views.
const ENDS: [(&str, &str); 6] = [
    ("X", "right"),
    ("-X", "left"),
    ("Y", "top"),
    ("-Y", "bottom"),
    ("Z", "front"),
    ("-Z", "back"),
];

/// The figure's area in the 3D view `view`, in its top corner on the side of the `hand` that
/// holds the pen, if the view is large enough to spare it: at least three figures wide and
/// high.
pub fn area(view: Rect, hand: Hand) -> Option<Rect> {
    let side = 2.0 * (RADIUS + END);
    if view.width() < 3.0 * side || view.height() < 3.0 * side {
        return None;
    }
    let x = match hand {
        Hand::Right => view.max.x - INSET,
        Hand::Left => view.min.x + INSET,
    };
    let centre = Pos2::new(x, view.min.y + INSET);
    Some(Rect::from_center_size(centre, Vec2::splat(side)))
}

/// The drag buttons' areas under the figure in the 3D view `view`, with the row each runs, in
/// the order of [`PADS`]: pills as high as the figure's 32-point pen target, shown while it
/// shows.
pub fn pads(view: Rect, hand: Hand) -> Vec<(Row, Rect)> {
    let Some(figure) = area(view, hand) else {
        return Vec::new();
    };
    let x = figure.center().x;
    (PADS.iter().enumerate())
        .map(|(i, (_, row))| {
            let y = figure.max.y + GAP + END + i as f32 * (2.0 * END + GAP);
            (
                *row,
                Rect::from_center_size(Pos2::new(x, y), Vec2::new(PAD_WIDTH, 2.0 * END)),
            )
        })
        .collect()
}

/// Draws the drag buttons under the figure, as a panel of their own so a press on one never
/// sculpts. `held` is the row of a drag that started on a button, which lights it.
fn draw_pads(ui: &egui::Ui, (view, hand): (Rect, Hand), held: Option<Row>, rec: &mut Recorder) {
    let pads = pads(view, hand);
    let Some(all) = pads.iter().map(|p| p.1).reduce(Rect::union) else {
        return;
    };
    rec.panel("pads", "Zoom and Pan", all);
    let painter = ui.painter_at(all);
    let theme = theme(ui.ctx());
    for ((name, _), (row, rect)) in PADS.iter().zip(pads) {
        let lit = held == Some(row);
        let (fill, text) = if lit {
            (theme.colors.lit, theme.colors.on_lit)
        } else {
            (theme.colors.control, theme.colors.text)
        };
        painter.rect_filled(rect, END, fill);
        let edge = Stroke::new(1.0, theme.colors.edge);
        painter.rect_stroke(rect, END, edge, egui::StrokeKind::Inside);
        let font = egui::FontId::proportional(theme.sizes.small);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            *name,
            font,
            text,
        );
        // Reported as a slider running the command its drag runs, still.
        let step = match row {
            Row::Pan => with_args("view.pan", json!({ "dx": 0.0, "dy": 0.0 })),
            _ => with_args("view.dolly", json!({ "factor": 1.0 })),
        };
        let mut widget = Widget::runs(*name, Role::Slider, step, None);
        widget.selected = lit;
        rec.widget(widget, rect);
    }
}

/// Draws the figure in the corner of the 3D view `view` (in points) on the pen `hand`'s side,
/// as its own panel, with the drag buttons under it (`held` lights the one being dragged), and
/// returns the command a click on an end asks for.
pub fn figure(
    ui: &egui::Ui,
    (view, hand): (Rect, Hand),
    (main, held): (&MainView, Option<Row>),
    rec: &mut Recorder,
) -> Vec<Step> {
    let Some(rect) = area(view, hand) else {
        return Vec::new();
    };
    rec.panel("figure", "Orientation figure", rect);
    let (right, up, forward) = main.camera.axes();
    let (right, up, forward) = (right.to_array(), up.to_array(), forward.to_array());
    // How far along the screen's right, up and forward end `i` lies.
    let along = |v: [f32; 3], i: usize| {
        if i.is_multiple_of(2) {
            v[i / 2]
        } else {
            -v[i / 2]
        }
    };
    let centre = rect.center();
    let painter = ui.painter_at(rect);
    let theme = theme(ui.ctx());
    painter.circle_filled(centre, RADIUS + END, theme.colors.backdrop);
    // Far ends first, so the near ones draw over them.
    let mut ends: Vec<(usize, Pos2, f32)> = (0..ENDS.len())
        .map(|i| {
            let at = centre + Vec2::new(along(right, i), -along(up, i)) * RADIUS;
            (i, at, along(forward, i))
        })
        .collect();
    ends.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut asked = Vec::new();
    for (i, at, _) in ends {
        let (name, look_from) = ENDS[i];
        let step = with_args("view.align", json!({ "to": look_from, "toggle": true }));
        let color = theme.axis(i / 2);
        if i.is_multiple_of(2) {
            painter.line_segment([centre, at], Stroke::new(2.0, color));
            painter.circle_filled(at, END, color);
            let font = egui::FontId::proportional(theme.sizes.small);
            let (middle, on) = (egui::Align2::CENTER_CENTER, theme.colors.on_axis());
            painter.text(at, middle, name, font, on);
        } else {
            painter.circle_filled(at, END * 0.8, color.gamma_multiply(0.35));
            painter.circle_stroke(at, END * 0.8, Stroke::new(1.5, color));
        }
        let hit = Rect::from_center_size(at, Vec2::splat(2.0 * END));
        let id = ui.id().with(("figure end", name));
        if ui.interact(hit, id, Sense::click()).clicked() {
            asked.push(step.clone());
        }
        let label = format!("Look from {look_from}");
        rec.widget(Widget::runs(label, Role::Button, step, None), hit);
    }
    draw_pads(ui, (view, hand), held, rec);
    asked
}
