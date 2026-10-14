//! A curve stroke's path in the window (decision 0055): the points the pen taps in with Alt,
//! the corners a quick second tap makes, Space moving the whole path, and the line the cursor
//! draws through them, with a gesture's shadow side (decision 0017 as amended 2026-10-12, item
//! 21).

use egui::{Color32, Pos2, Vec2};
use gs_core::brush::{BrushPreset, StrokeSpec};
use gs_core::stroke::curve_path;
use gs_core::{StrokeSample, Vec3};
use gs_engine::PenStroke;
use gs_platform::Input;

use super::Studio;
use crate::held::TAP;
use crate::record::{Recorder, Role, Widget};
use crate::viewport::Cursor;

/// How far the shadow side reaches from the path, in points.
const SHADOW: f32 = 32.0;

/// A stroke's path through points in window pixels, as the engine's stroke holds it (decision
/// 0055): the press and each point tapped in, then the pen.
pub(super) struct Path {
    /// The press and each point, each with whether it is sharp.
    pub(super) points: Vec<([f64; 2], bool)>,
    /// The latest sample sent.
    pen: [f64; 2],
    /// When Alt was last let go, in seconds by the window's clock, for a double tap.
    let_go: Option<f64>,
    /// For a gesture, whose cut has a side, whether its brush subtracts (Alt flips that side
    /// again); `None` for a stroke with no side (Clay Line).
    shadow: Option<bool>,
}

impl Path {
    /// A path pressed at `at`, stroked with `preset`.
    pub(super) fn new(at: [f64; 2], preset: &BrushPreset) -> Self {
        let gesture = matches!(preset.stroke, StrokeSpec::Gesture { .. });
        Self {
            points: vec![(at, true)],
            pen: at,
            let_go: None,
            shadow: gesture.then(|| preset.subtracts()),
        }
    }

    /// The pen moved to `at`: while Space is held (`moving`) the press and the points move
    /// with it.
    pub(super) fn step(&mut self, at: [f64; 2], moving: bool) {
        if moving {
            let [dx, dy] = [at[0] - self.pen[0], at[1] - self.pen[1]];
            for (p, _) in &mut self.points {
                *p = [p[0] + dx, p[1] + dy];
            }
        }
        self.pen = at;
    }

    /// Alt going down (`alt`) at `now` seconds taps a point in at the pen, or, within [`TAP`]
    /// of Alt being let go, makes the latest point a corner instead (decision 0055, section 2).
    /// What to send the engine: nothing when Alt is let go.
    pub(super) fn alt(&mut self, alt: bool, now: f64) -> Option<PenStroke> {
        if !alt {
            self.let_go = Some(now);
            return None;
        }
        let quick = self.let_go.take().is_some_and(|t| now - t <= TAP);
        // A point where the last one is (no sample between) draws nothing new: the path merges
        // coincident points, as the engine's does.
        match self.points.last_mut() {
            Some(last) if quick => last.1 = true,
            _ => self.points.push((self.pen, false)),
        }
        Some(if quick {
            PenStroke::Corner
        } else {
            PenStroke::Point
        })
    }
}

impl Studio {
    /// Space during a stroke that runs through points moves its path (decision 0055, section
    /// 6), and during Drag Rect its rectangle (decision 0062, row 782): from its press through
    /// its release, repeats too, it goes to the stroke and never to egui, so no Space binding
    /// (the palette's) sees it. Returns the rest of `inputs`.
    pub(super) fn stroke_keys(&mut self, inputs: &[Input]) -> Vec<Input> {
        let mut rest = Vec::with_capacity(inputs.len());
        for input in inputs {
            match (input, self.input.space) {
                (
                    Input::Key {
                        key, down, repeat, ..
                    },
                    Some(moving),
                ) if key == "Space" && (*down || moving) => {
                    if !*repeat && *down != moving {
                        self.input.space = Some(*down);
                        self.input.strokes.push(PenStroke::Space(*down));
                    }
                }
                _ => rest.push(input.clone()),
            }
        }
        rest
    }

    /// The cursor draws the stroke's path as it now stands, if it keeps one, and a gesture's
    /// shadow side as Alt now leaves it: the cut works the other way while Alt differs from
    /// the brush's subtracting.
    pub(super) fn show_path(&mut self) {
        let path = self.input.line.as_ref().and_then(|l| l.path.as_ref());
        self.shell.cursor.path = path.map_or_else(Vec::new, |p| {
            let mut points = p.points.clone();
            points.push((p.pen, true));
            points
        });
        let alt = self.input.alt;
        self.shell.cursor.shadow = path.and_then(|p| p.shadow).map(|sub| sub != alt);
    }
}

/// Draws a curve stroke's path (decision 0055, section 6) in the view `clip`, as an open line
/// with `stroke`: the same [`curve_path`] the engine's stroke runs along, through the cursor's
/// path (the press, each point and the pen, each with whether it is sharp) in window pixels,
/// over its shadow side if it has one.
pub(crate) fn draw_path(
    ui: &egui::Ui,
    clip: egui::Rect,
    ppp: f32,
    cursor: &Cursor,
    stroke: egui::Stroke,
    rec: &mut Recorder,
) {
    let points = &cursor.path;
    if points.len() < 2 {
        return;
    }
    let at = |[x, y]: [f64; 2]| StrokeSample {
        pos: Vec3::new(x as f32, y as f32, 0.0),
        pressure: 1.0,
        tilt: [0.0; 2],
        radius: 1.0,
    };
    let points: Vec<(StrokeSample, bool)> = points.iter().map(|&(p, s)| (at(p), s)).collect();
    let line: Vec<egui::Pos2> = (curve_path(&points).iter())
        .map(|s| egui::pos2(s.pos.x / ppp, s.pos.y / ppp))
        .collect();
    let bounds = egui::Rect::from_points(&line);
    let painter = ui.painter_at(clip);
    let band = cursor
        .shadow
        .and_then(|right| shadow(&line, right, stroke.color));
    if let Some(band) = band {
        rec.widget(
            Widget::plain("Shadow side", Role::Label),
            band.calc_bounds(),
        );
        painter.add(egui::Shape::mesh(band));
    }
    painter.add(egui::Shape::line(line, stroke));
    rec.widget(Widget::plain("Curve path", Role::Label), bounds);
}

/// The shadow side of `line` (decision 0017 as amended 2026-10-12, item 21): a band on its
/// left as it runs on the screen, or its right (`right`), from a third of `color` at the line
/// to nothing [`SHADOW`] out. `None` for a line with no length.
fn shadow(line: &[Pos2], right: bool, color: Color32) -> Option<egui::Mesh> {
    let mut points = line.to_vec();
    points.dedup();
    if points.len() < 2 {
        return None;
    }
    let side = if right { -1.0 } else { 1.0 };
    // Each piece's left on a screen whose y runs down.
    let normals: Vec<Vec2> = (points.windows(2))
        .map(|w| (w[1] - w[0]).normalized().rot90() * side)
        .collect();
    let (near, mut band) = (color.gamma_multiply(1.0 / 3.0), egui::Mesh::default());
    for (i, &p) in points.iter().enumerate() {
        let after = normals[i.min(normals.len() - 1)];
        let n = (normals[i.saturating_sub(1)] + after).normalized();
        let n = if n == Vec2::ZERO { after } else { n };
        band.colored_vertex(p, near);
        band.colored_vertex(p + n * SHADOW, Color32::TRANSPARENT);
        if let Some(k) = (2 * i as u32).checked_sub(2) {
            band.add_triangle(k, k + 1, k + 2);
            band.add_triangle(k + 1, k + 3, k + 2);
        }
    }
    Some(band)
}
