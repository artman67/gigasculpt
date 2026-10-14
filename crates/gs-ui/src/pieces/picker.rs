//! Brush tray > Paint's picker and the color pick (decision 0039, section 11, as amended
//! 2026-10-12 in step P8b). The picker: a hue ring, red at the top and the hues going clockwise,
//! around a square of saturation (left to right) and value (bottom to top), and a gray bar under
//! it, black to white, for the edited swatch, in sRGB bytes so it agrees exactly with the hex
//! field and the sliders. The pick: what a swatch's drag and C take from the 3D view, the color
//! stored under the pen as the pen's reads found it, or with Alt the pixel the window drew there.

use std::f32::consts::FRAC_1_SQRT_2;

use egui::epaint::Mesh;
use egui::{Color32, Id, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use gs_core::Vec3;
use gs_core::camera::Camera;
use gs_engine::PenReads;
use gs_protocol::Step;
use serde_json::json;

use super::hover;
use super::paint::{edited_key, hex};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::shell::Shell;
use crate::theme::theme;

/// The wheel's greatest width in points.
const WHEEL: f32 = 168.0;
/// The ring's inside, as a fraction of the wheel's half width: the ring is its outer fifth.
const INNER: f32 = 0.8;
/// The square's half width, as a fraction of the wheel's: a corner on a circle a little inside
/// the ring.
const SQUARE: f32 = 0.76 * FRAC_1_SQRT_2;
/// The gray bar's height in points.
const BAR: f32 = 14.0;
/// The ring's segments: every 60 degrees among their ends, where the hues bend, so the colors
/// between are exact.
const SEGMENTS: u16 = 72;
/// The square's cells along each side.
const GRID: u16 = 16;

/// The picker's color, `[hue in degrees, saturation, value]` beside the bytes they made, and
/// what a press on the picker drags until the pen lets go.
#[derive(Clone, Copy, Default)]
struct Held {
    hsv: [f32; 3],
    bytes: [u8; 3],
    drag: Option<Area>,
}

/// What a press fixes a drag to set: the hue on the ring, saturation and value anywhere else in
/// the wheel, held inside the square, and a gray on the bar.
#[derive(Clone, Copy, PartialEq)]
enum Area {
    Ring,
    Square,
    Gray,
}

/// The picker's rows for the edited color `key`, showing `value`: the wheel on a row, the gray
/// bar on the next. Each frame of a drag that changes the color returns one `color.set`.
pub(super) fn rows(
    ui: &mut egui::Ui,
    key: &str,
    value: [u8; 3],
    rec: &mut Recorder,
) -> Option<Step> {
    let id = Id::new("paint picker");
    let held: Held = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut now = if held.drag.is_some() || held.bytes == value {
        held
    } else {
        let hsv = hsv(value, held.hsv[0]);
        Held {
            hsv,
            bytes: value,
            drag: None,
        }
    };
    let side = ui.available_width().min(WHEEL);
    let (wheel, on_wheel) = ui.allocate_exact_size(Vec2::splat(side), Sense::drag());
    ui.end_row();
    let (bar, on_bar) = ui.allocate_exact_size(vec2(side, BAR), Sense::drag());
    ui.end_row();
    let square = Rect::from_center_size(wheel.center(), Vec2::splat(SQUARE * side));
    let pressed = [(&on_wheel, false), (&on_bar, true)]
        .into_iter()
        .find(|(r, _)| r.is_pointer_button_down_on());
    let before = now.bytes;
    match pressed.and_then(|(r, gray)| Some((r.interact_pointer_pos()?, gray))) {
        Some((at, gray)) => {
            let inside = (at - wheel.center()).length() < INNER * side / 2.0;
            let area = now.drag.unwrap_or(match (gray, inside) {
                (true, _) => Area::Gray,
                (false, false) => Area::Ring,
                (false, true) => Area::Square,
            });
            now.hsv = moved(area, now.hsv, at, [wheel, square, bar]);
            now.bytes = bytes_of(now.hsv);
            now.drag = Some(area);
        }
        None => now.drag = None,
    }
    ui.data_mut(|d| d.insert_temp(id, now));
    draw(ui, now.hsv, [wheel, square, bar], gray_of(now.bytes));
    let step = |c: [u8; 3]| with_args("color.set", json!({ key: hex(c) }));
    let areas = [
        (
            on_wheel,
            "Hue",
            "Hue: drag around the ring; in the square, saturation and value",
        ),
        (on_bar, "Gray", "Gray: black to white"),
    ];
    for (shown, title, said) in areas {
        rec.widget(
            Widget::runs(title, Role::Slider, step(now.bytes), None),
            shown.rect,
        );
        hover(shown, Some(said), rec);
    }
    let said = "Saturation and value: saturation left to right, value bottom to top";
    let widget = Widget::runs("Saturation and value", Role::Slider, step(now.bytes), None);
    rec.widget(widget, square);
    rec.hint(said);
    (now.bytes != before).then(|| step(now.bytes))
}

/// `hsv` moved to the point `at` by a drag on `area` of the wheel, square and bar `rects`.
fn moved(area: Area, [h, s, v]: [f32; 3], at: Pos2, [wheel, square, bar]: [Rect; 3]) -> [f32; 3] {
    let unit = |x: f32, from: f32, to: f32| ((x - from) / (to - from)).clamp(0.0, 1.0);
    match area {
        Area::Ring => {
            let d = at - wheel.center();
            let hue = libm::atan2f(d.x, -d.y).to_degrees();
            [hue.rem_euclid(360.0), s, v]
        }
        Area::Square => {
            let s = unit(at.x, square.left(), square.right());
            [h, s, 1.0 - unit(at.y, square.top(), square.bottom())]
        }
        Area::Gray => [h, 0.0, unit(at.x, bar.left(), bar.right())],
    }
}

/// Draws the wheel and the bar with their markers: the ring's at the hue, the square's at the
/// saturation and value, and the bar's only while the color is a gray (`gray`, its level).
fn draw(ui: &egui::Ui, [h, s, v]: [f32; 3], [wheel, square, bar]: [Rect; 3], gray: Option<u8>) {
    let colors = &theme(ui.ctx()).colors;
    let painter = ui.painter();
    let half = wheel.width() / 2.0;
    painter.add(ring(wheel.center(), INNER * half, half));
    painter.add(plane(square, h));
    let mut ends = Mesh::default();
    for (x, level) in [(bar.left(), 0), (bar.right(), 255)] {
        let gray = Color32::from_gray(level);
        ends.colored_vertex(pos2(x, bar.top()), gray);
        ends.colored_vertex(pos2(x, bar.bottom()), gray);
    }
    ends.add_triangle(0, 1, 2);
    ends.add_triangle(1, 3, 2);
    painter.add(ends);
    let edge = Stroke::new(1.0, colors.edge);
    for rect in [square, bar] {
        painter.rect_stroke(rect, 0.0, edge, StrokeKind::Outside);
    }
    let mark = |at: Pos2| {
        painter.circle_stroke(at, 4.0, Stroke::new(3.0, colors.panel));
        painter.circle_stroke(at, 4.0, Stroke::new(1.5, colors.text));
    };
    let turn = h.to_radians();
    let way = vec2(libm::sinf(turn), -libm::cosf(turn));
    mark(wheel.center() + way * (1.0 + INNER) / 2.0 * half);
    mark(square.lerp_inside(vec2(s, 1.0 - v)));
    if let Some(level) = gray {
        mark(bar.lerp_inside(vec2(f32::from(level) / 255.0, 0.5)));
    }
}

/// The hue ring between radii `inner` and `outer` about `center`, red at the top.
fn ring(center: Pos2, inner: f32, outer: f32) -> Mesh {
    let mut mesh = Mesh::default();
    for k in 0..=SEGMENTS {
        let hue = 360.0 * f32::from(k) / f32::from(SEGMENTS);
        let turn = hue.to_radians();
        let way = vec2(libm::sinf(turn), -libm::cosf(turn));
        let color = shown(bytes_of([hue, 1.0, 1.0]));
        mesh.colored_vertex(center + way * inner, color);
        mesh.colored_vertex(center + way * outer, color);
        let i = 2 * u32::from(k);
        if k > 0 {
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i + 1, i);
        }
    }
    mesh
}

/// The square of saturation and value at hue `h` over `rect`.
fn plane(rect: Rect, h: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let n = u32::from(GRID);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let f = vec2(f32::from(i), f32::from(j)) / f32::from(GRID);
            mesh.colored_vertex(rect.lerp_inside(f), shown(bytes_of([h, f.x, 1.0 - f.y])));
            let k = u32::from(j) * (n + 1) + u32::from(i);
            if i > 0 && j > 0 {
                mesh.add_triangle(k - n - 2, k - n - 1, k);
                mesh.add_triangle(k - n - 2, k, k - 1);
            }
        }
    }
    mesh
}

/// `c` as egui draws it.
fn shown([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The level of `c` when it is a gray.
fn gray_of([r, g, b]: [u8; 3]) -> Option<u8> {
    (r == g && g == b).then_some(r)
}

/// The bytes of the standard HSV color `[hue in degrees, saturation, value]`, each channel
/// rounded to the nearest byte.
fn bytes_of([h, s, v]: [f32; 3]) -> [u8; 3] {
    let c = v * s;
    let sector = (h / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let rgb = match sector as u8 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    rgb.map(|k| (255.0 * (k + v - c)).round().clamp(0.0, 255.0) as u8)
}

/// The standard HSV of `c`'s bytes taken as 0 to 1, `[hue in degrees, saturation, value]`, with
/// the hue `kept` for a gray, which has none of its own.
fn hsv(c: [u8; 3], kept: f32) -> [f32; 3] {
    let [r, g, b] = c.map(i32::from);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = (max - min) as f32;
    let hue = if max == min {
        kept
    } else if max == r {
        60.0 * ((g - b) as f32 / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) as f32 / d + 2.0)
    } else {
        60.0 * ((r - g) as f32 / d + 4.0)
    };
    let s = if max == 0 { 0.0 } else { d / max as f32 };
    [hue, s, max as f32 / 255.0]
}

/// What the color under the pen is kept with: the document's revision and the camera it was read
/// under. It is taken only while both still hold.
type Stamp = (u64, Camera);

/// The color stored under the pen as the pen's reads last found it: `None` off every part, or
/// while the pen is off the 3D view or stroking.
#[derive(Clone, Copy)]
struct Under {
    color: Option<[u8; 3]>,
    stamp: Stamp,
}

fn under_id() -> Id {
    Id::new("paint pick under")
}

/// The stamp the shell's state gives now.
pub(crate) fn stamp(shell: &Shell) -> Stamp {
    (shell.status.rev, shell.main.camera)
}

/// Keeps the color stored where the scene ray `ray` (from the eye under the pen, `None` while
/// the pen is off the 3D view or stroking) first meets a part that shows, through the pen's
/// `reads`, as of `stamp`.
pub(crate) fn under(
    ctx: &egui::Context,
    ray: Option<(Vec3, Vec3)>,
    reads: &PenReads,
    stamp: Stamp,
) {
    let color = ray.and_then(|(o, d)| (reads.under)(o, d)).map(|s| s.color);
    ctx.data_mut(|d| d.insert_temp(under_id(), Under { color, stamp }));
}

/// Forgets the color under the pen once `stamp` no longer holds: an edit or a turn of the view
/// since the pen's reads found it.
pub(crate) fn still(ctx: &egui::Context, stamp: Stamp) {
    let id = under_id();
    let stale = ctx
        .data(|d| d.get_temp::<Under>(id))
        .is_some_and(|u| u.stamp != stamp);
    if stale {
        ctx.data_mut(|d| d.remove::<Under>(id));
    }
}

/// The color stored under the pen, if it is still good.
fn hovered(ctx: &egui::Context) -> Option<[u8; 3]> {
    ctx.data(|d| d.get_temp::<Under>(under_id()))?.color
}

/// The keys' steps with C's pick resolved, as a view key's `view = "pointed"` is: `color.set`
/// with `pick` becomes the edited swatch set to the color stored under the pen, and is dropped
/// when the pen is off every part or the color is stale.
pub(crate) fn keyed(ctx: &egui::Context, steps: Vec<Step>) -> Vec<Step> {
    let picks = |s: &Step| s.command == "color.set" && s.args.get("pick").is_some();
    (steps.into_iter())
        .filter_map(|s| {
            if !picks(&s) {
                return Some(s);
            }
            let key = edited_key(ctx);
            hovered(ctx).map(|c| with_args("color.set", json!({ key: hex(c) })))
        })
        .collect()
}

fn wanted_id() -> Id {
    Id::new("paint pick pixel wanted")
}

fn drawn_id() -> Id {
    Id::new("paint pick pixel drawn")
}

/// Whether a swatch's drag with Alt asked this frame for the pixel under the pen; asking
/// again is up to the next frame.
pub(crate) fn pixel_wanted(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(wanted_id()))
        .unwrap_or(false)
}

/// Keeps the pixel the frame drew under the pen, `[r, g, b, a]`, for the next frame's swatch:
/// `None` when no frame was asked for one.
pub(crate) fn drawn(ctx: &egui::Context, pixel: Option<[u8; 4]>) {
    let rgb = pixel.map(|[r, g, b, _]| [r, g, b]);
    ctx.data_mut(|d| d.insert_temp(drawn_id(), rgb));
}

/// A swatch's drag onto the 3D view, as the swatch `shown` (for the color `key`, now `value`)
/// sees it this frame: the color it shows, and on the release the color it picked. While Alt
/// is held it follows the pixel the window drew under the pen (over a panel too), else the
/// color stored there; Escape during the drag gives it up.
pub(super) fn dragged(
    ui: &egui::Ui,
    shown: &egui::Response,
    key: &str,
    value: [u8; 3],
) -> ([u8; 3], Option<[u8; 3]>) {
    let id = Id::new(("paint swatch drag", key));
    let (ending, going) = (shown.drag_stopped(), shown.dragged());
    let (escape, alt) = ui.input(|i| (i.key_pressed(Key::Escape), i.modifiers.alt));
    let mut given_up = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    given_up |= (going || ending) && escape;
    ui.data_mut(|d| {
        if going && given_up {
            d.insert_temp(id, true);
        } else {
            d.remove::<bool>(id);
        }
    });
    if given_up || !(going || ending) {
        return (value, None);
    }
    let picked = if alt {
        ui.data_mut(|d| d.insert_temp(wanted_id(), true));
        ui.data(|d| d.get_temp::<Option<[u8; 3]>>(drawn_id()))
            .flatten()
    } else {
        hovered(ui.ctx())
    };
    (picked.unwrap_or(value), picked.filter(|_| ending))
}
