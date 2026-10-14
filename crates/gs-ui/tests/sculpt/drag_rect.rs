//! Space moves Drag Rect's rectangle while it is drawn (decision 0062, row 782), as it moves a
//! curve stroke's path (decision 0055): the stamp lands where the moved rectangle is, the size
//! the drag gave it; Space never opens the palette; and the logged stroke replays the same
//! without a window.

use gs_platform::Modifiers;
use serde_json::json;

use crate::curve::space;
use crate::gizmo::{positions, replayed};
use crate::region;
use crate::rig::{Window, log_path, pen_at, with_sphere};

/// How far the drag goes right of the press, in pixels: the rectangle's size.
const DRAG: f32 = 30.0;

/// One pen sample, touching with `pressure` above 0, then a frame of the window. Whether the
/// palette is open after it.
fn frame(w: &mut Window, t: u64, [x, y]: [f32; 2], pressure: f32) -> bool {
    w.pen(pen_at(t, x, y, pressure));
    w.input(&[], Modifiers::default());
    w.host.lock().shell.palette.open
}

/// A Clay Drag Rect stroke pressed at `at` and dragged [`DRAG`] right, a frame after each
/// sample; then, if `moved`, Space held while the pen goes on by that much; then the lift.
/// Whether the palette opened.
fn drag_rect(w: &mut Window, at: [f32; 2], moved: Option<[f32; 2]>) -> bool {
    w.ok(
        "brush.set",
        json!({ "brush": "Clay", "stroke": "drag_rect" }),
    );
    let mut opened = frame(w, 0, at, 0.0);
    let mut pen = at;
    for k in 0..4_u8 {
        pen = [at[0] + DRAG * f32::from(k) / 3.0, at[1]];
        opened |= frame(w, 4 + 4 * u64::from(k), pen, 0.5);
    }
    if let Some(by) = moved {
        let from = pen;
        w.input(&[space(true)], Modifiers::default());
        for k in 1..4_u8 {
            let f = f32::from(k) / 3.0;
            pen = [from[0] + by[0] * f, from[1] + by[1] * f];
            opened |= frame(w, 20 + 4 * u64::from(k), pen, 0.5);
        }
        w.input(&[space(false)], Modifiers::default());
    }
    opened | frame(w, 40, pen, 0.0)
}

/// What a stroke left between `before` and `after`: the middle of the points it moved, each
/// weighted by how far, and how many moved.
fn stamp(before: &[[f64; 3]], after: &[[f64; 3]]) -> ([f64; 3], usize) {
    let (mut sum, mut weight, mut count) = ([0.0; 3], 0.0, 0);
    for (p, q) in before.iter().zip(after) {
        let far = distance(*p, *q);
        if far > 1e-6 {
            (0..3).for_each(|i| sum[i] += far * p[i]);
            (weight, count) = (weight + far, count + 1);
        }
    }
    (sum.map(|s| s / weight), count)
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f64>()).sqrt()
}

/// Where the window's camera sees the sphere at `at`, in the scene.
fn hit(w: &mut Window, [x, y]: [f32; 2]) -> [f64; 3] {
    let p = w.picked(x, y)["hit"]["position"].clone();
    std::array::from_fn(|i| p[i].as_f64().unwrap_or(f64::NAN))
}

#[test]
fn space_moves_the_drag_rect_while_drawn_and_its_stamp_lands_there_its_size() {
    let by = [24.0, 16.0];
    let (mut w, rect) = with_sphere(Some(log_path("drag-rect-space")));
    let [x, y] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let (at, to) = ([x - 30.0, y - 20.0], [x - 30.0 + by[0], y - 20.0 + by[1]]);
    let apart = distance(hit(&mut w, at), hit(&mut w, to));
    let start = positions(&mut w);
    assert!(
        !drag_rect(&mut w, at, Some(by)),
        "Space left the palette shut"
    );
    let (moved, count) = stamp(&start, &positions(&mut w));
    let (mut there, _) = with_sphere(None);
    drag_rect(&mut there, to, None);
    let (want, want_count) = stamp(&start, &positions(&mut there));
    assert!(
        distance(moved, want) < 0.2 * apart,
        "where a press there stamps: {moved:?} for {want:?}, the presses {apart} apart"
    );
    let size = count as f64 / want_count as f64;
    assert!(
        (0.8..1.25).contains(&size),
        "the same size: {count} points for {want_count}"
    );
    assert_eq!(
        replayed(&w),
        w.hash(),
        "the logged stroke replays without a window"
    );
}
