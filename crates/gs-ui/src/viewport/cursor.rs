//! The pen's cursor over the 3D view: what the shell keeps of it between frames ([`Cursor`],
//! [`Detail`]) and its drawing ([`Ring`]): the pen's ring, the brush's rings, a quieter ring
//! where symmetry repeats the dab, and the smoothing string. Its lines of text are in `lines`.

mod lines;

use egui::{Color32, Stroke};
use gs_core::Vec3;
use gs_core::brush::{BrushPreset, Family};
use gs_engine::{Blocked, BrushState, SessionState};
use gs_protocol::PenSample;

use crate::gizmo::{GizmoPrefs, Lit};
use crate::held::Held;
use crate::navigate::Context;
use crate::record::{Recorder, Role, Widget};
use crate::theme::{Colors, theme};

pub use lines::symmetry_text;
use lines::{cursor_lines, draw_lines};

/// The pen's cursor, kept by the shell between frames: the pen writes it (`Studio::pen`) and
/// the 3D view draws it ([`Ring`]), so a cursor feature is a field here and its drawing below.
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    /// The pen, while it is in range.
    pub pen: Option<PenSample>,
    /// The pen is sculpting: it touched the 3D view and has not lifted. The held keys keep the
    /// brush slot they held at the press until it lifts.
    pub stroking: bool,
    /// Where the smoothing string pulls the dab while a stroke with smoothing runs (decision
    /// 0036 section 4), in window pixels, as the pen is.
    pub string: Option<[f64; 2]>,
    /// While the pen drags a select outline (a polygon or rectangle gesture), the samples it
    /// sent, in window pixels: the outline the engine closes when the pen lifts.
    pub outline: Vec<[f64; 2]>,
    /// While that outline is a rectangle (Select Rect), its Square and Center: it is drawn from
    /// the first sample to the last, as the engine makes it (gs-core `stroke::rectangle`).
    pub rect: Option<(bool, bool)>,
    /// Scene points: where the dab under the pen lands, then each place symmetry repeats it
    /// (gs-engine's `PenReads::dabs`). Empty while symmetry is off.
    pub dabs: Vec<Vec3>,
    /// The surface under the pen, for the brush's rings to lie on: a scene point and its unit
    /// normal (gs-engine's `PenReads::under`). `None` off the model, or with the rings set to
    /// stay flat on the screen (`ui.settings` `ring_on_surface`).
    pub surface: Option<(Vec3, Vec3)>,
    /// The value a slider key is setting, while it is held past the activation time or a
    /// number is typed (`Size 80 px`, `interface.on-canvas-adjust`).
    pub adjusting: Option<String>,
    /// What Live Detail does under the pen, while it is on and the brush changes shape.
    pub detail: Option<Detail>,
    /// Shift holds the stroke to a straight line (`stroke.freehand`), and the cursor says so.
    pub line: bool,
    /// A slider shows alone over the cursor (decision 0025 section 2, as amended 2026-10-12):
    /// the text beside it, which would lie over the slider, is left out.
    pub slider_alone: bool,
    /// While a stroke that runs through points is drawn (Clay Line, Clip Curve), its press,
    /// each point tapped in and the pen, in window pixels, each with whether it is sharp: the
    /// path drawn (decision 0055).
    pub path: Vec<([f64; 2], bool)>,
    /// While that path is a gesture's, the side its cut takes: `false` for the left of the way
    /// the path runs on the screen, `true` for the right (decision 0017, item 21).
    pub shadow: Option<bool>,
}

/// What Live Detail does under the pen (`geometry.live-detail-mode`), as `session.state` says
/// it (`live_detail.eligible`, `.blocked`): the cursor turns purple where a stroke refines,
/// orange where a smooth brush refines as it thins, and red where it cannot, with why beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Refines,
    Thins,
    Cannot(Blocked),
}

impl Detail {
    /// What Live Detail does on the active part under `preset`, the brush the pen strokes with
    /// (a held slot's): `None` while it is off, in an empty scene, and for a brush that masks,
    /// hides or selects, which changes no shape and keeps its own color.
    pub fn of(state: &SessionState, preset: Option<&BrushPreset>) -> Option<Self> {
        let family = preset.map(|p| p.family);
        if !state.brush.live_detail.on || state.active.is_none() || family == Some(Family::Mask) {
            return None;
        }
        Some(match (&state.live_detail_blocked, family) {
            (Some(refusal), _) => Self::Cannot(refusal.blocked),
            (None, Some(Family::Smooth)) => Self::Thins,
            (None, _) => Self::Refines,
        })
    }

    /// The cursor's color in `colors`: red where it cannot, as Alt's subtract.
    pub fn color(self, colors: &Colors) -> Color32 {
        match self {
            Self::Refines => colors.detail(false),
            Self::Thins => colors.detail(true),
            Self::Cannot(_) => colors.subtract,
        }
    }

    /// The line beside the cursor, a block's in gs-core's words for its kind
    /// ([`Blocked::label`]). `live_detail.why_not` says the same at length, with the remedy,
    /// which is too long for beside the pen.
    pub fn text(self) -> String {
        match self {
            Self::Refines => "Live Detail".into(),
            Self::Thins => "Live Detail: smooths and thins".into(),
            Self::Cannot(blocked) => format!("No Live Detail: {}", blocked.label()),
        }
    }

    /// The status line's part: the line beside the cursor while the brush's own automask keeps
    /// Live Detail off, which dims the switch (decision 0021, section 3). The other reasons
    /// show beside the pen only.
    pub fn status(self) -> Option<String> {
        matches!(self, Self::Cannot(b) if b.automask()).then(|| self.text())
    }

    /// The state value the Live Detail switch shows (its line in `controls/scene-tray.toml` is
    /// its one home), and why every toggle showing it is dimmed now: the brush's own automask
    /// keeps Live Detail off, as `live_detail.blocked` says, whether or not it is on. The reason
    /// is `live_detail.why_not`. Dimmed, the switch still turns Live Detail on and off, as
    /// Backslash does (decision 0021, section 3, amended 2026-10-12).
    pub fn switch(state: &SessionState) -> (&'static str, Option<String>) {
        let blocked = state.live_detail_blocked.as_ref();
        let why = blocked
            .filter(|r| r.blocked.automask())
            .map(|r| r.why.clone());
        ("live_detail.on", why)
    }
}

/// What the cursor ring shows: the cursor, the brush as the window shows it (only while the
/// pen is over the 3D view and nothing covers it), and what the held modifier keys do.
pub struct Ring<'a> {
    pub cursor: &'a Cursor,
    pub brush: Option<&'a BrushState>,
    pub held: Option<&'a Held>,
}

impl Ring<'_> {
    /// The rings' and lines' color in `colors`: what Live Detail does, while it is on and the
    /// brush changes shape; else what the held keys do; else the brush's own.
    pub(super) fn tint(&self, colors: &Colors) -> Color32 {
        match (self.cursor.detail, self.held) {
            (Some(detail), _) => detail.color(colors),
            (None, Some(held)) => held.color(colors),
            (None, None) => colors.brush,
        }
    }
}

/// The pen's cursor: a thin ring while it hovers, and a filled disc that grows with pressure
/// while it touches; around it the brush's circle at its size on screen (a size set as a length
/// shows no circle; while the tip touches, the size its pressure gives the dab) with the Softness
/// ring inside, both lying flat on the surface under the pen
/// ([`flat_ring`]), and beside it the brush's name, size and strength; its Softness and symmetry;
/// what the held modifier does; what Live Detail does; and the value a slider key is setting.
/// All of it is colored by Live Detail while it is on (`Detail::color`), else by the held
/// modifier (`Held::color`). Over a `lit` handle only the pen's ring shows, with what the
/// handle drags and of the lines only the held modifier's and the slider key's, clear of the
/// handle (decision 0041, amended 2026-10-13 for `DF7c-notes`, sections 1 and 2). With the
/// `gizmo` as the tool, its words and its Softness's ring ([`cursor_lines`]).
pub(super) fn draw_ring(
    ui: &egui::Ui,
    (cx, pen): (&Context, PenSample),
    (ring, lit, gizmo): (&Ring, Option<&Lit>, Option<GizmoPrefs>),
    rec: &mut Recorder,
) {
    let ppp = ui.ctx().pixels_per_point();
    let at = egui::pos2(pen.x as f32 / ppp, pen.y as f32 / ppp);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("cursor"),
    ));
    let t = theme(ui.ctx());
    let color = if pen.contact {
        t.colors.pen_down
    } else {
        t.colors.pen
    };
    painter.circle_stroke(at, PEN_RING, Stroke::new(1.5, color));
    if pen.contact {
        painter.circle_filled(at, 2.0 + 6.0 * pen.pressure, color.gamma_multiply(0.6));
    }
    let Some(brush) = ring.brush.filter(|b| !b.brush.is_empty()) else {
        return;
    };
    let tint = ring.tint(&t.colors);
    let font = egui::FontId::proportional(t.sizes.body);
    let lines = cursor_lines(brush, ring, (lit, gizmo));
    draw_lines(&painter, (at, lit, cx.rect), lines, (font, tint), rec);
    if let (Some(px), None) = (brush.size_px, lit) {
        // While the tip touches, the size the dab takes at the pen's pressure: through the size
        // curve in effect, the pen's Size sensitivity in it (`brush.set`'s `size_curve`).
        let curve = brush.size_curve.as_ref().filter(|_| pen.contact);
        let size = px / ppp * curve.map_or(1.0, |c| c.eval(pen.pressure));
        // The Softness ring: where the falloff curve's bent distance reaches one half
        // (gs-core's falloff.rs bends d to d^(2^(-f/50))), so it moves in as Softness rises.
        let falloff = gizmo.map_or(brush.falloff, |g| g.falloff);
        let inner = size * libm::powf(0.5, libm::exp2f(falloff / 50.0));
        for (radius, name, width) in [(size, "Size ring", 1.5), (inner, "Softness ring", 1.0)] {
            let stroke = Stroke::new(width, tint);
            let flat = ring.cursor.surface.and_then(|s| flat_ring(cx, s, radius));
            let bounds = if let Some(outline) = flat {
                let bounds = egui::Rect::from_points(&outline);
                painter.add(egui::Shape::closed_line(outline, stroke));
                bounds
            } else {
                painter.circle_stroke(at, radius, stroke);
                egui::Rect::from_center_size(at, egui::Vec2::splat(2.0 * radius))
            };
            rec.widget(Widget::plain(name, Role::Label), bounds);
        }
    }
}

/// A ring lying flat on the surface at `(point, normal)`: a circle on it about `point`, as
/// large as a circle `radius` points across the screen there, seen from the camera, so a
/// surface turning away shows it narrower. `None` if any of it falls behind the eye.
fn flat_ring(cx: &Context, (point, normal): (Vec3, Vec3), radius: f32) -> Option<Vec<egui::Pos2>> {
    let camera = &cx.main.camera;
    let size = cx.rect.size() * cx.ppp;
    let size = [size.x, size.y];
    let across = radius * cx.ppp * camera.pixel_size(camera.depth(point), size);
    let side = if normal.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = normal.cross(side).normalized();
    let v = normal.cross(u);
    (0..RING_STEPS)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / RING_STEPS as f32;
            let p = point + (u * libm::cosf(t) + v * libm::sinf(t)) * across;
            let [x, y] = camera.project(p.to_array(), size)?;
            Some(cx.rect.min + egui::vec2(x, y) / cx.ppp)
        })
        .collect()
}

/// Straight pieces in a ring lying on the surface.
const RING_STEPS: usize = 64;

/// Where symmetry repeats the dab under the pen: a quieter ring at each repeat, thinner and
/// fainter than the Size ring, as large as the dab is on screen at that depth. A size set as a
/// length shows no Size ring, so its repeats are marked with a ring the pen's size.
pub(super) fn draw_mirrored(ui: &egui::Ui, cx: &Context, ring: &Ring, rec: &mut Recorder) {
    let (Some(brush), [dab, repeats @ ..]) = (
        ring.brush.filter(|b| !b.brush.is_empty()),
        &ring.cursor.dabs[..],
    ) else {
        return;
    };
    let camera = &cx.main.camera;
    let size = cx.rect.size() * cx.ppp;
    let size = [size.x, size.y];
    let scale = |p: Vec3| camera.pixel_size(camera.depth(p), size);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("cursor"),
    ));
    let tint = ring.tint(&theme(ui.ctx()).colors);
    let stroke = Stroke::new(1.0, tint.gamma_multiply(MIRROR_FADE));
    for &p in repeats {
        let Some([x, y]) = camera.project(p.to_array(), size) else {
            continue;
        };
        let at = cx.rect.min + egui::vec2(x, y) / cx.ppp;
        let radius = (brush.size_px).map_or(PEN_RING, |px| px / cx.ppp * scale(*dab) / scale(p));
        painter.circle_stroke(at, radius, stroke);
        let bounds = egui::Rect::from_center_size(at, egui::Vec2::splat(2.0 * radius));
        rec.widget(Widget::plain("Mirror ring", Role::Label), bounds);
    }
}

/// While a stroke with smoothing runs, the string from the pen to the dab it pulls (`dab`, in
/// window pixels), with a dot at the dab, in the pen's color.
pub(super) fn draw_string(ui: &egui::Ui, pen: PenSample, dab: [f64; 2], rec: &mut Recorder) {
    let ppp = ui.ctx().pixels_per_point();
    let [at, dab] = [[pen.x, pen.y], dab].map(|[x, y]| egui::pos2(x as f32 / ppp, y as f32 / ppp));
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("cursor"),
    ));
    let color = theme(ui.ctx()).colors.pen_down;
    painter.line_segment([at, dab], Stroke::new(1.5, color));
    painter.circle_filled(dab, STRING_DOT, color);
    let bounds = egui::Rect::from_two_pos(at, dab).expand(STRING_DOT);
    rec.widget(Widget::plain("Smoothing string", Role::Label), bounds);
}

/// The ring marking the pen itself, in points: small, so the brush's own rings stay clear.
const PEN_RING: f32 = 10.0;
/// The dot at the dab the smoothing string pulls, its radius in points.
const STRING_DOT: f32 = 3.0;
/// How much of the brush's color a mirror ring keeps: half, so the pen's own ring leads.
pub(super) const MIRROR_FADE: f32 = 0.5;
