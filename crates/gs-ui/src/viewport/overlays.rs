//! What the 3D view draws over the model besides the cursor: the pivot dot while the view turns,
//! the brush's symmetry planes on the active part ([`ActivePart`]), and a select outline being
//! dragged with each place symmetry repeats it.

use egui::{Color32, Stroke};
use gs_core::camera::Projection;
use gs_core::stroke::rectangle;
use gs_core::transform::Similarity;
use gs_core::{Aabb, NodeId, Vec3};
use gs_engine::{BrushState, PartReport};

use super::Ring;
use super::cursor::MIRROR_FADE;
use crate::navigate::Context;
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// The active part as the 3D view draws symmetry on it: its bounding box in its own space and
/// its transform, part to scene, as the engine last reported them.
#[derive(Clone, Copy, Debug)]
pub struct ActivePart {
    pub bounds: Aabb,
    pub transform: Similarity,
}

impl ActivePart {
    /// The active part among `parts`, its box joined from its nodes' boxes; `None` without
    /// one, or while it has no points.
    pub fn of(parts: &[PartReport]) -> Option<Self> {
        let part = parts.iter().find(|p| p.active)?;
        let store = part.store;
        let bounds = (0..store.node_count() as NodeId)
            .map(|n| store.bounds(n))
            .fold(Aabb::EMPTY, Aabb::union);
        (bounds.lo.x <= bounds.hi.x).then_some(Self {
            bounds,
            transform: part.transform,
        })
    }
}

/// The point the view turns about, as a small dot, while it turns (Nomad shows it the same way).
pub(super) fn draw_pivot(ui: &egui::Ui, cx: &Context) {
    let size = cx.rect.size() * cx.ppp;
    let Some([x, y]) = cx.main.camera.project(cx.main.pivot, [size.x, size.y]) else {
        return;
    };
    let at = cx.rect.min + egui::vec2(x, y) / cx.ppp;
    if !cx.rect.contains(at) {
        return;
    }
    let painter = ui.painter_at(cx.rect);
    let colors = &theme(ui.ctx()).colors;
    painter.circle_filled(at, PIVOT_DOT, colors.pivot);
    painter.circle_stroke(at, PIVOT_DOT, Stroke::new(1.0, colors.edge));
}

/// The pivot dot's radius in points.
const PIVOT_DOT: f32 = 4.0;

/// The brush's symmetry on the active part (decision 0044, section 8): each Mirror or Parallel
/// plane on, as the rectangle where it crosses the part's box, through the part's origin in its
/// own space; the Radial axis as a line through the box. Each is a thin line in its axis'
/// color, a Parallel plane's dashed, drawn over the model in every look so a plane never hides
/// inside it. A plane the box does not reach is drawn across the box's other two sides all the
/// same, so it shows where it is. Nothing while the master is off, which leaves the brush's
/// symmetry empty.
pub(super) fn draw_planes(
    ui: &egui::Ui,
    cx: &Context,
    (brush, part): (&BrushState, ActivePart),
    rec: &mut Recorder,
) {
    let (s, t) = (&brush.symmetry, theme(ui.ctx()));
    let painter = ui.painter_at(cx.rect);
    let [lo, hi] = [part.bounds.lo.to_array(), part.bounds.hi.to_array()];
    let scene = |q: [f32; 3]| part.transform.point(Vec3::from_array(q));
    let (m, p) = (&s.mirror, &s.parallel);
    for (a, (mirror, parallel)) in [(m.x, p.x), (m.y, p.y), (m.z, p.z)].into_iter().enumerate() {
        if !mirror && !parallel {
            continue;
        }
        let (b, c) = ((a + 1) % 3, (a + 2) % 3);
        let corner = |[u, v]: [&[f32; 3]; 2]| {
            let mut q = [0.0; 3];
            (q[b], q[c]) = (u[b], v[c]);
            scene(q)
        };
        let corners = [[&lo, &lo], [&hi, &lo], [&hi, &hi], [&lo, &hi]].map(corner);
        let stroke = Stroke::new(PLANE_LINE, t.axis(a));
        let mut drawn = Vec::new();
        for i in 0..4 {
            let Some(edge) = on_view(cx, [corners[i], corners[(i + 1) % 4]]) else {
                continue;
            };
            if parallel {
                painter.extend(egui::Shape::dashed_line(&edge, stroke, DASH, DASH));
            } else {
                painter.line_segment(edge, stroke);
            }
            drawn.extend(edge);
        }
        let kind = if parallel { "Parallel" } else { "Mirror" };
        let label = format!("{kind} plane {}", AXES[a]);
        if !drawn.is_empty() {
            rec.widget(
                Widget::plain(label, Role::Label),
                egui::Rect::from_points(&drawn),
            );
        }
    }
    if let Some(r) = s.radial {
        let a = r.axis as usize;
        let mut ends = [[0.0; 3]; 2];
        (ends[0][a], ends[1][a]) = (lo[a], hi[a]);
        if let Some(axis) = on_view(cx, ends.map(scene)) {
            painter.line_segment(axis, Stroke::new(PLANE_LINE, t.axis(a)));
            let label = format!("Radial axis {}", AXES[a]);
            rec.widget(
                Widget::plain(label, Role::Label),
                egui::Rect::from_points(&axis),
            );
        }
    }
}

/// The scene segment `a` to `b` where the view draws it, in points: cut where it passes behind
/// a perspective eye, `None` if all of it does.
fn on_view(cx: &Context, [a, b]: [Vec3; 2]) -> Option<[egui::Pos2; 2]> {
    let camera = &cx.main.camera;
    let [a, b] = if camera.projection == Projection::Perspective {
        let near = camera.distance * NEAR;
        let (da, db) = (camera.depth(a), camera.depth(b));
        let cut = |p: Vec3, dp: f32, q: Vec3, dq: f32| {
            if dp < near {
                p + (q - p) * ((near - dp) / (dq - dp))
            } else {
                p
            }
        };
        if da < near && db < near {
            return None;
        }
        [cut(a, da, b, db), cut(b, db, a, da)]
    } else {
        [a, b]
    };
    let size = cx.rect.size() * cx.ppp;
    let at = |p: Vec3| {
        let [x, y] = camera.project(p.to_array(), [size.x, size.y])?;
        Some(cx.rect.min + egui::vec2(x, y) / cx.ppp)
    };
    Some([at(a)?, at(b)?])
}

/// While the pen drags a select outline, the outline, and on the active part each place the
/// brush's symmetry repeats it, where the hide applies too (gs-core `stroke/gesture.rs`, once
/// per pass): each sample carried to the plane through the camera's target that faces the
/// camera, as the engine places a gesture's points, through the pass in the part's own space
/// and back onto the screen. Thin closed lines in the ring's color, a copy fainter, as a mirror
/// ring is. From a turned camera a copy is that one slice of what the hide takes, which runs
/// through the model's depth.
pub(super) fn draw_outline(
    ui: &egui::Ui,
    cx: &Context,
    ring: &Ring,
    (brush, part): (&BrushState, Option<ActivePart>),
    rec: &mut Recorder,
) {
    let pts = &ring.cursor.outline;
    if pts.len() < 2 {
        return;
    }
    let corners: Vec<[f64; 2]>;
    let pts = match ring.cursor.rect {
        Some((square, center)) => {
            let f = |[x, y]: [f64; 2]| [x as f32, y as f32];
            let c = rectangle(f(pts[0]), f(pts[pts.len() - 1]), square, center);
            corners = c.map(|[x, y]| [f64::from(x), f64::from(y)]).to_vec();
            &corners
        }
        None => pts,
    };
    let painter = ui.painter_at(cx.rect);
    let tint = ring.tint(&theme(ui.ctx()).colors);
    let mut draw = |line: Vec<egui::Pos2>, color: Color32, label: &str| {
        let bounds = egui::Rect::from_points(&line);
        painter.add(egui::Shape::closed_line(
            line,
            Stroke::new(PLANE_LINE, color),
        ));
        rec.widget(Widget::plain(label, Role::Label), bounds);
    };
    let points = |[x, y]: [f64; 2]| egui::pos2(x as f32 / cx.ppp, y as f32 / cx.ppp);
    draw(
        pts.iter().map(|&p| points(p)).collect(),
        tint,
        "Gesture outline",
    );
    let Some(part) = part else {
        return;
    };
    let camera = &cx.main.camera;
    let size = cx.rect.size() * cx.ppp;
    let (size, corner) = ([size.x, size.y], cx.rect.min * cx.ppp);
    let (f, target) = (camera.view().dir, Vec3::from_array(camera.target));
    let placed: Vec<Vec3> = (pts.iter())
        .map(|&[x, y]| {
            let (o, d) = camera.ray(x as f32 - corner.x, y as f32 - corner.y, size);
            part.transform
                .part_point(o + d * ((target - o).dot(f) / d.dot(f)))
        })
        .collect();
    for xf in brush.symmetry.core().passes().into_iter().skip(1) {
        let seen = placed.iter().map(|&q| {
            let p = part.transform.point(xf.apply(q));
            let [x, y] = camera.project(p.to_array(), size)?;
            Some(cx.rect.min + egui::vec2(x, y) / cx.ppp)
        });
        if let Some(line) = seen.collect::<Option<Vec<_>>>() {
            draw(line, tint.gamma_multiply(MIRROR_FADE), "Mirrored outline");
        }
    }
}

/// The planes' and the Radial axis' line width, in points.
pub(super) const PLANE_LINE: f32 = 1.5;
/// A Parallel plane's dashes and the gaps between them, in points.
const DASH: f32 = 6.0;
/// How near the eye, in camera distances, a plane is cut where it passes behind it.
const NEAR: f32 = 1e-3;
/// The axes' names, X first.
const AXES: [&str; 3] = ["X", "Y", "Z"];
