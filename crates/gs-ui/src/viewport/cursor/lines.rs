//! The cursor's lines of text: what they say (the brush, its Softness and symmetry, the held
//! keys, Live Detail, a slider key's value, a lit handle's words) and where they go, inside the
//! 3D view and clear of a lit handle.

use std::sync::Arc;

use egui::{Color32, Galley};
use gs_engine::BrushState;
use gs_protocol::Length;

use super::{Detail, PEN_RING, Ring};
use crate::gizmo::{GizmoPrefs, Lit};
use crate::held::Held;
use crate::record::{Recorder, Role, Widget};
use crate::values::Value;

/// The lines beside the cursor: the brush, its size and strength; its Softness and symmetry;
/// what the held modifier does; Shift's straight line; what Live Detail does; and the value a
/// slider key is setting. Over a `lit` handle what it drags, then the held modifier's and the
/// slider key's. With the `gizmo` as the tool, its Reach (the brush's Size, which a soft drag
/// reaches to) and its own Softness in place of the brush's lines, since a gizmo drag uses
/// nothing else of the brush (decision 0062, row 816). None while a slider shows alone over
/// the cursor.
pub(super) fn cursor_lines(
    brush: &BrushState,
    ring: &Ring,
    (lit, gizmo): (Option<&Lit>, Option<GizmoPrefs>),
) -> Vec<String> {
    if ring.cursor.slider_alone {
        return Vec::new();
    }
    let held = ring.held.map(Held::text);
    let adjusting = ring.cursor.adjusting.clone();
    if let Some(lit) = lit {
        return (lit.words.iter().cloned())
            .chain(held)
            .chain(adjusting)
            .collect();
    }
    let falloff = Value::Falloff;
    if let Some(g) = gizmo {
        let reach = format!("Reach {}", size_text(brush));
        let soft = format!("{} {}", falloff.title(), falloff.text(g.falloff));
        let lines = [Some(reach), Some(soft), held, adjusting];
        return lines.into_iter().flatten().collect();
    }
    let second = format!(
        "{} {}  {}",
        falloff.title(),
        falloff.text(brush.falloff),
        symmetry_text(brush)
    );
    let line = ring.cursor.line.then(|| "Shift: Line".to_string());
    let detail = ring.cursor.detail.map(Detail::text);
    let lines = [
        Some(brush_text(brush)),
        Some(second),
        held,
        line,
        detail,
        adjusting,
    ];
    lines.into_iter().flatten().collect()
}

/// Draws `lines` one under another beside the pen at `at`, each recorded, inside the 3D view
/// `view` (never over the status line, a tray or the header): the first just right of the
/// cursor ring, level with the pen, where the eye already is, unless that leaves the view or
/// covers a `lit` handle; then where [`clear_of`] puts them. Over a lit handle they wrap at
/// [`WORDS_WRAP`] in even lines ([`balanced`]).
pub(super) fn draw_lines(
    painter: &egui::Painter,
    (at, lit, view): (egui::Pos2, Option<&Lit>, egui::Rect),
    lines: Vec<String>,
    (font, tint): (egui::FontId, Color32),
    rec: &mut Recorder,
) {
    let wrap = lit.map_or(f32::INFINITY, |_| WORDS_WRAP);
    let laid: Vec<_> = (lines.into_iter())
        .map(|l| (balanced(painter, &l, (&font, tint), wrap), l))
        .collect();
    let Some(high) = laid.first().map(|(g, _)| g.size().y) else {
        return;
    };
    let size = (laid.iter()).fold(egui::Vec2::ZERO, |s, (g, _)| {
        egui::vec2(s.x.max(g.size().x), s.y + g.size().y)
    });
    let outline = lit.map_or(&[][..], |l| &l.outline);
    let mut corner = clear_of((view, outline), (at, high), size);
    for (galley, text) in laid {
        let rect = egui::Rect::from_min_size(corner, galley.size());
        corner.y += galley.size().y;
        painter.galley(rect.min, galley, tint);
        rec.widget(Widget::plain(text, Role::Label), rect);
    }
}

/// `text` wrapped at `wrap` in lines as even as that many lines allow, so no line is left
/// holding one short word (decision 0062, row 754): laid out at the narrowest whole-point wrap
/// that takes no more lines. egui wraps at whole points, and a wider wrap never takes more
/// lines, so halving finds it.
fn balanced(
    painter: &egui::Painter,
    text: &str,
    (font, tint): (&egui::FontId, Color32),
    wrap: f32,
) -> Arc<Galley> {
    let lay = |w: f32| painter.layout(text.to_string(), font.clone(), tint, w);
    let full = lay(wrap);
    let rows = full.rows.len();
    if !wrap.is_finite() || rows < 2 {
        return full;
    }
    let (mut short, mut fits) = (0.0, wrap.floor());
    while fits - short > 1.0 {
        let mid = f32::midpoint(short, fits).floor();
        if lay(mid).rows.len() <= rows {
            fits = mid;
        } else {
            short = mid;
        }
    }
    lay(fits)
}

/// Where a block of lines `size` large beside the pen at `at` (its first line `high`) lies
/// inside the 3D view `view` and keeps clear of a lit handle's `outline` (empty with none lit):
/// the top-left corner of the first of these places that does: right of the cursor ring level
/// with the pen, left of it, under it, over it, and with a handle lit, past the outline's right
/// side, level with the pen. Near the view's foot that is over the ring. Else the first of them
/// that keeps clear once moved inside the view, else the first moved inside it.
fn clear_of(
    (view, outline): (egui::Rect, &[egui::Pos2]),
    (at, high): (egui::Pos2, f32),
    size: egui::Vec2,
) -> egui::Pos2 {
    let (gap, level) = (LABEL_AT.x, at.y - high / 2.0);
    let (under, over) = (at.y + PEN_RING + 4.0, at.y - PEN_RING - 4.0 - size.y);
    let past = (!outline.is_empty())
        .then(|| egui::pos2(egui::Rect::from_points(outline).max.x + gap, level));
    let places: Vec<egui::Pos2> = [
        egui::pos2(at.x + gap, level),
        egui::pos2(at.x - gap - size.x, level),
        egui::pos2(at.x - size.x / 2.0, under),
        egui::pos2(at.x - size.x / 2.0, over),
    ]
    .into_iter()
    .chain(past)
    .collect();
    let block = |p: egui::Pos2| egui::Rect::from_min_size(p, size);
    let clear = |p: &egui::Pos2| !covers(block(*p), outline);
    let inside = |p: &egui::Pos2| view.contains_rect(block(*p));
    let kept = |p: egui::Pos2| p.clamp(view.min, (view.max - size).max(view.min));
    let first = places.iter().copied().find(|p| inside(p) && clear(p));
    first
        .or_else(|| places.iter().map(|&p| kept(p)).find(clear))
        .unwrap_or_else(|| kept(places[0]))
}

/// Whether `block`, with room for an arrow's head, takes in any of the closed `outline`'s
/// edges, walked every two points.
fn covers(block: egui::Rect, outline: &[egui::Pos2]) -> bool {
    let block = block.expand(6.0);
    let mut edges = outline.iter().zip(outline.iter().cycle().skip(1));
    edges.any(|(&a, &b)| {
        let steps = ((b - a).length() / 2.0).ceil().max(1.0);
        (0..=steps as usize).any(|i| block.contains(a.lerp(b, i as f32 / steps)))
    })
}

/// The symmetry as the window writes it: `Mirror X`, `Mirror X+Z`, `Parallel X`, `Radial 6`,
/// or `No symmetry`.
pub fn symmetry_text(b: &BrushState) -> String {
    let (m, p) = (&b.symmetry.mirror, &b.symmetry.parallel);
    let named = |word: &str, [x, y, z]: [bool; 3]| {
        let axes: Vec<&str> = [(x, "X"), (y, "Y"), (z, "Z")]
            .into_iter()
            .filter_map(|(on, axis)| on.then_some(axis))
            .collect();
        (!axes.is_empty()).then(|| format!("{word} {}", axes.join("+")))
    };
    let mirror = named("Mirror", [m.x, m.y, m.z]);
    let parallel = named("Parallel", [p.x, p.y, p.z]);
    let radial = b.symmetry.radial.map(|r| format!("Radial {}", r.count));
    let parts: Vec<String> = mirror.into_iter().chain(parallel).chain(radial).collect();
    if parts.is_empty() {
        "No symmetry".into()
    } else {
        parts.join(", ")
    }
}

/// Where the brush's label starts, from the pen, in points.
const LABEL_AT: egui::Vec2 = egui::vec2(16.0, 0.0);
/// How wide the lines over a lit handle grow before they wrap, in points.
const WORDS_WRAP: f32 = 280.0;

/// The brush, its size and its strength.
fn brush_text(b: &BrushState) -> String {
    format!("{}  {}  {:.0}%", b.brush, size_text(b), b.strength * 100.0)
}

/// The brush's size, in pixels or as the length it was set as.
fn size_text(b: &BrushState) -> String {
    match (b.size_px, b.size) {
        (Some(px), _) => format!("{px:.0} px"),
        (None, Some(Length::Percent(p))) => format!("{p}%"),
        (None, Some(Length::Units(u))) => format!("{u} units"),
        (None, None) => String::new(),
    }
}
