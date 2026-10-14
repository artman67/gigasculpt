//! The gizmo drawn over the 3D view, from gs-core's `handles`, and the open deformer's handles
//! over it, from gs-core's `deformer_handles` (decision 0041, amended 2026-10-13 for `DF7c`,
//! section 5).

use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};
use gs_core::camera::Camera;
use gs_core::gizmo::{DeformerHandle, DeformerHandles, Kind, handles};

use super::{GizmoPrefs, PenPress, Tool};
use crate::theme::{Colors, theme};

/// The handle lit in the 3D view (decision 0041, amended 2026-10-13 for `DF7c-notes`): its
/// outline in points, which the cursor's lines keep clear of, and for one of the open
/// deformer's what it drags, in lines for the cursor (none for the gizmo's own).
pub struct Lit {
    pub outline: Vec<Pos2>,
    pub words: Vec<String>,
}

/// Draws the gizmo over the 3D view `rect` (in points) as `camera` sees it with `prefs`, in the
/// view's own layer under the cursor. Each handle is its axis's color (a plane square its
/// normal's, the centre and the view ring the dim text's), faded as gs-core says; the handle
/// held, or else the one a press at the hovering pen `hover` (the view's pixels) would grab, is
/// the brush's color and thicker. While the tool is on, the open deformer's box and handles go
/// over it the same way, its handles grabbed first, and the pen rectangle over all. Returns the
/// handle lit.
pub fn draw(
    ui: &egui::Ui,
    tool: &Tool,
    (rect, camera): (egui::Rect, &Camera),
    prefs: GizmoPrefs,
    hover: Option<[f32; 2]>,
) -> Option<Lit> {
    let style = &prefs.style(ui.ctx().pixels_per_point());
    let ppp = style.px_per_pt;
    let size = rect.size() * ppp;
    let size = [size.x, size.y];
    let painter = ui.painter_at(rect);
    let colors = &theme(ui.ctx()).colors;
    let at = |p: &[f32; 2]| rect.min + vec2(p[0], p[1]) / ppp;
    let deforming = tool.deforming.as_ref().filter(|_| tool.on);
    let shown = deforming.map(|d| d.shown(camera, size, style));
    let hot =
        (shown.as_ref()).and_then(|s| tool.hot_deformer(s, style, hover).map(DeformerHandle::name));
    let mut lit = None;
    if let Some((_, pose)) = tool.drawn() {
        let drawn = handles(&pose, camera, size, style);
        let hot = tool.hot(&drawn, style, hover.filter(|_| hot.is_none()));
        for h in &drawn.handles {
            let points: Vec<Pos2> = h.points.iter().map(at).collect();
            let on = hot == Some(h.id);
            let base = match h.id.axis() {
                _ if on => colors.brush,
                Some(a) => axis(colors, a),
                None => colors.text_dim,
            };
            if on {
                lit = Some(Lit {
                    outline: points.clone(),
                    words: Vec::new(),
                });
            }
            let color = base.gamma_multiply(h.fade);
            let width = if on { 3.5 } else { 2.0 };
            painter.extend(outline(h.id.kind(), points, color, width));
        }
    }
    if let (Some(shown), Some(d)) = (shown, deforming) {
        let on = (shown.handles.iter()).find(|h| hot.as_deref() == Some(h.name().as_str()));
        if let Some(h) = on {
            let outline = h.points.iter().map(at).collect();
            let words = d.words(ui.ctx(), h, prefs.snap);
            lit = Some(Lit { outline, words });
        }
        painter.extend(deformer(&shown, (hot.as_deref(), &d.selected), colors, at));
    }
    if let PenPress::Rect(r) = &tool.press {
        let stroke = Stroke::new(1.0, colors.brush);
        let corners = Rect::from_two_pos(at(&r.start), at(&r.end));
        painter.rect_stroke(corners, 0.0, stroke, StrokeKind::Inside);
    }
    lit
}

/// A handle's shapes: an arrow is a line with a head at its tip; a scale box and the grip are
/// filled; a plane square and the centre are a faint fill inside a line; a ring is a line.
fn outline(kind: &str, p: Vec<Pos2>, color: Color32, width: f32) -> Vec<Shape> {
    let stroke = Stroke::new(width, color);
    match kind {
        "arrow" if p.len() == 2 => {
            let (start, tip) = (p[0], p[1]);
            let along = (tip - start).normalized();
            let mut shapes = vec![Shape::line_segment([start, tip], stroke)];
            if along.is_finite() && (tip - start).length() > ARROW_HEAD[0] {
                let back = tip - along * ARROW_HEAD[0];
                let side = along.rot90() * ARROW_HEAD[1];
                let head = vec![tip, back + side, back - side];
                shapes.push(Shape::convex_polygon(head, color, Stroke::NONE));
            }
            shapes
        }
        "box" | "grip" => vec![Shape::convex_polygon(p, color, Stroke::NONE)],
        "plane" | "centre" => {
            let fill = color.gamma_multiply(0.35);
            vec![Shape::convex_polygon(p, fill, stroke)]
        }
        _ => vec![Shape::line(p, stroke)],
    }
}

/// The open deformer's shapes (section 5): its boxes' edges and the cage's or curve's lines in
/// the dim text color, a repeat's at half strength; a cone filled in its face's axis color and
/// the ring a line in its axis's, faded as gs-core says; a dot in the dim text color, or the
/// brush's while `selected`. The `hot` handle is the brush's color, a cone or dot outlined and
/// the ring thicker.
fn deformer(
    shown: &DeformerHandles,
    (hot, selected): (Option<&str>, &BTreeSet<String>),
    colors: &Colors,
    at: impl Fn(&[f32; 2]) -> Pos2,
) -> Vec<Shape> {
    let lines = shown.lines.iter().map(|l| {
        let color = colors.text_dim;
        let color = if l.repeat {
            color.gamma_multiply(0.5)
        } else {
            color
        };
        Shape::line(l.points.iter().map(&at).collect(), Stroke::new(1.0, color))
    });
    let handles = shown.handles.iter().map(|h| {
        let name = h.name();
        let lit = hot == Some(name.as_str());
        let points: Vec<Pos2> = h.points.iter().map(&at).collect();
        let base = match (h.kind, h.axis) {
            _ if lit => colors.brush,
            (Kind::Dot, _) if selected.contains(&name) => colors.brush,
            (Kind::Cone | Kind::Ring, Some(a)) => axis(colors, a),
            _ => colors.text_dim,
        };
        let color = base.gamma_multiply(h.fade);
        match h.kind {
            Kind::Ring => Shape::line(points, Stroke::new(if lit { 3.5 } else { 2.0 }, color)),
            _ if lit => Shape::convex_polygon(points, color, Stroke::new(1.5, colors.text)),
            _ => Shape::convex_polygon(points, color, Stroke::NONE),
        }
    });
    lines.chain(handles).collect()
}

/// Axis `a`'s color, 0 for X.
fn axis(colors: &Colors, a: usize) -> Color32 {
    [colors.axis_x, colors.axis_y, colors.axis_z][a.min(2)]
}

/// An arrow's head: its length and half its width, in points.
const ARROW_HEAD: [f32; 2] = [10.0, 5.0];
