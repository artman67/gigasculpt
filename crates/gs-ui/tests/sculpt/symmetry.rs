//! Symmetry in the window (decision 0044, sections 5 and 8): the planes drawn on the active
//! part, read back from what the 3D view drew and from its pixels, the gizmo under each type,
//! and a select outline's mirrored copy while it is dragged.

use egui::Color32;
use gs_platform::Modifiers;
use gs_ui::{Keymap, Themes};
use serde_json::{Value, json};

use crate::gizmo::{gizmo_window, path_x, positions, stroke};
use crate::rig::{Window, pen_at};

/// A sphere in a window big enough that its planes fit in the 3D view, seen from `orbit`.
fn turned(orbit: &Value) -> (Window, [f32; 4]) {
    let mut w = Window::sized(None, Keymap::default(), [1000, 700], 1.0);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok("view.set", json!({ "camera": orbit }));
    let rect = w.view_rect();
    (w, rect)
}

/// The view's widgets that draw symmetry, as (label, rectangle) in window pixels.
fn planes(w: &mut Window) -> Vec<(String, [f64; 4])> {
    view_widgets(w, &["Mirror plane", "Parallel plane", "Radial axis"])
}

/// The view's widgets whose labels start with one of `drawn`, as (label, rectangle) in window
/// pixels.
fn view_widgets(w: &mut Window, drawn: &[&str]) -> Vec<(String, [f64; 4])> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
    let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
    (widgets.unwrap_or_default().iter())
        .filter_map(|x| {
            let label = x["label"].as_str()?;
            drawn.iter().any(|d| label.starts_with(d)).then(|| {
                let rect = std::array::from_fn(|i| x["rect"][i].as_f64().unwrap_or_default());
                (label.to_string(), rect)
            })
        })
        .collect()
}

/// Where the window draws the scene point `p`, in window pixels, its 3D view at `rect`.
fn drawn_at(w: &Window, rect: [f32; 4], p: [f64; 3]) -> [f32; 2] {
    let camera = w.host.lock().shell.main.camera;
    let size = [rect[2] - rect[0], rect[3] - rect[1]];
    let seen = camera.project(p.map(|c| c as f32), size);
    let [x, y] = seen.unwrap_or_else(|| panic!("{p:?} is behind the eye"));
    [rect[0] + x, rect[1] + y]
}

/// The sphere's box, as `mesh.bounds` reports it: its least and greatest corners.
fn bounds(w: &mut Window) -> [[f64; 3]; 2] {
    let b = w.ok("mesh.bounds", json!({}));
    let three = |v: &Value| std::array::from_fn(|i| v[i].as_f64().unwrap_or(f64::NAN));
    [three(&b["min"]), three(&b["max"])]
}

/// The corners of the plane across axis `a` through the origin, where it crosses `bounds`, in
/// order round it.
fn plane(a: usize, [lo, hi]: [[f64; 3]; 2]) -> Vec<[f64; 3]> {
    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
    let corner = |u: &[f64; 3], v: &[f64; 3]| {
        let mut q = [0.0; 3];
        (q[b], q[c]) = (u[b], v[c]);
        q
    };
    vec![
        corner(&lo, &lo),
        corner(&hi, &lo),
        corner(&hi, &hi),
        corner(&lo, &hi),
    ]
}

/// The share of points along the lines through `path`'s points in turn (closed when `closed`),
/// as drawn on screen, that have a pixel near `color` beside them. Points near the 3D view's
/// edge are left out.
fn coverage(
    w: &mut Window,
    rect: [f32; 4],
    (path, closed): (&[[f64; 3]], bool),
    color: Color32,
) -> f64 {
    let at: Vec<[f32; 2]> = path.iter().map(|&p| drawn_at(w, rect, p)).collect();
    let width = (rect[2] - rect[0]).round() as usize;
    let height = (rect[3] - rect[1]).round() as usize;
    let pixels = w.view_pixels();
    let near = |x: usize, y: usize| {
        let p = &pixels[(y * width + x) * 4..][..3];
        let d = |a: u8, b: u8| (f32::from(a) - f32::from(b)) * (f32::from(a) - f32::from(b));
        d(p[0], color.r()) + d(p[1], color.g()) + d(p[2], color.b()) < 60.0 * 60.0
    };
    let edges = if closed { at.len() } else { at.len() - 1 };
    let (mut seen, mut hit) = (0_u32, 0_u32);
    for i in 0..edges {
        let [a, b] = [at[i], at[(i + 1) % at.len()]];
        for k in 1..60 {
            let f = k as f32 / 60.0;
            let x = (a[0] + (b[0] - a[0]) * f - rect[0]).floor() as i64;
            let y = (a[1] + (b[1] - a[1]) * f - rect[1]).floor() as i64;
            let inside =
                |v: i64, most: usize| usize::try_from(v).is_ok_and(|v| (4..most - 4).contains(&v));
            if !inside(x, width) || !inside(y, height) {
                continue;
            }
            seen += 1;
            let beside = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy)));
            hit += u32::from(
                beside
                    .into_iter()
                    .any(|(x, y)| near(x as usize, y as usize)),
            );
        }
    }
    assert!(
        seen as usize > 40 * edges,
        "the lines are in the view: {at:?}"
    );
    f64::from(hit) / f64::from(seen)
}

/// The theme's color for axis `a`, as the axis figure shows it.
fn axis(a: usize) -> Color32 {
    Themes::default().list()[0].axis(a)
}

/// Each plane on is drawn on the active part: X Mirror as one red rectangle where the plane
/// x = 0 crosses the sphere's box, X Parallel as the same rectangle dashed, Radial about Y as a
/// green line through the box. They are drawn over the model in every look, and not at all while
/// the master is off.
#[test]
fn each_symmetry_plane_on_is_drawn_across_the_part_in_its_axis_color() {
    let (mut w, rect) = turned(&json!({ "azimuth_deg": 50, "elevation_deg": 20 }));
    let none: Vec<(String, [f64; 4])> = Vec::new();
    assert_eq!(planes(&mut w), none, "no symmetry, no plane");
    let b = bounds(&mut w);
    let x = plane(0, b);
    let red = axis(0);

    w.ok(
        "brush.set",
        json!({ "symmetry": { "mirror": { "x": true } } }),
    );
    let drawn = planes(&mut w);
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].0, "Mirror plane X");
    let solid = coverage(&mut w, rect, (&x, true), red);
    assert!(solid > 0.95, "a solid red rectangle: {solid}");
    w.ok("view.look", json!({ "wireframe": true }));
    let wired = coverage(&mut w, rect, (&x, true), red);
    assert!(wired > 0.95, "over the model in another look too: {wired}");
    w.ok("view.look", json!({ "wireframe": false }));

    w.ok("symmetry.set", json!({ "x_type": "parallel" }));
    let drawn = planes(&mut w);
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].0, "Parallel plane X");
    let dashed = coverage(&mut w, rect, (&x, true), red);
    assert!(
        (0.3..0.85).contains(&dashed),
        "a dashed red rectangle: {dashed}"
    );

    w.ok(
        "symmetry.set",
        json!({ "x": false, "y": true, "y_type": "radial" }),
    );
    let drawn = planes(&mut w);
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].0, "Radial axis Y");
    let [lo, hi] = b;
    let line = [[0.0, lo[1], 0.0], [0.0, hi[1], 0.0]];
    let green = coverage(&mut w, rect, (&line, false), axis(1));
    assert!(green > 0.95, "a green line along Y: {green}");

    // Off, the axes kept: nothing is drawn.
    w.ok(
        "symmetry.set",
        json!({ "x": true, "x_type": "mirror", "on": false }),
    );
    assert_eq!(planes(&mut w), none, "the master is off");
    let gone = coverage(&mut w, rect, (&x, true), red);
    assert!(gone < 0.05, "no red rectangle: {gone}");
}

/// Turning the camera keeps the rectangle on the plane: drawn where the plane's corners are
/// seen from each camera.
#[test]
fn a_turned_camera_keeps_the_rectangle_on_the_plane() {
    let (mut w, rect) = turned(&json!({ "azimuth_deg": 50, "elevation_deg": 20 }));
    w.ok(
        "brush.set",
        json!({ "symmetry": { "mirror": { "x": true } } }),
    );
    let x = plane(0, bounds(&mut w));
    let mut seen = Vec::new();
    for orbit in [
        json!({ "azimuth_deg": 50, "elevation_deg": 20 }),
        json!({ "azimuth_deg": -35, "elevation_deg": -25 }),
    ] {
        w.ok("view.set", json!({ "camera": orbit }));
        let corners: Vec<[f32; 2]> = x.iter().map(|&p| drawn_at(&w, rect, p)).collect();
        let span = |i: usize, most: bool| {
            let all = corners.iter().map(|c| f64::from(c[i]));
            if most {
                all.fold(f64::MIN, f64::max)
            } else {
                all.fold(f64::MAX, f64::min)
            }
        };
        let want = [span(0, false), span(1, false), span(0, true), span(1, true)];
        let drawn = planes(&mut w);
        assert_eq!(drawn.len(), 1, "{orbit}: {drawn:?}");
        let off = (0..4)
            .map(|i| (drawn[0].1[i] - want[i]).abs())
            .fold(0.0, f64::max);
        assert!(
            off < 1.5,
            "{orbit}: drawn {:?}, the plane at {want:?}",
            drawn[0].1
        );
        let red = coverage(&mut w, rect, (&x, true), axis(0));
        assert!(red > 0.95, "{orbit}: {red}");
        seen.push(want);
    }
    assert!(
        (seen[0][0] - seen[1][0]).abs() > 20.0,
        "the cameras see the plane apart: {seen:?}"
    );
}

/// With X Radial the gizmo refuses a drag that would move the part and says why: transforms
/// have no Radial (decision 0027, section 13), and a drag that silently did not copy would
/// sculpt one side. With X Mirror the same drag moves both sides, each the other's mirror image;
/// with X Parallel both the same way (decision 0044, section 5).
#[test]
fn a_gizmo_drag_is_refused_under_radial_and_moves_both_sides_under_mirror_and_parallel() {
    let mut w = gizmo_window("symmetry-gizmo", 1.0, 75.0);
    // Off the X plane: a gizmo on a plane does not mirror across it (decision 0027, section 6).
    w.ok("gizmo.place", json!({ "origin": [0.5, 0.0, 0.0] }));
    let applied = |w: &Window| (w.logged().iter()).any(|(c, _)| c == "transform.apply");
    let problem = |w: &Window| w.host.lock().shell.status.problem.clone();
    w.ok("symmetry.set", json!({ "x": true, "x_type": "radial" }));
    let before = w.hash();
    let path = path_x(&mut w);
    stroke(&mut w, &path, Modifiers::default());
    assert_eq!(w.hash(), before, "no drag under Radial");
    assert!(!applied(&w), "{:?}", w.logged());
    let refused = problem(&w).unwrap_or_default();
    assert!(refused.starts_with("Radial symmetry"), "{refused:?}");

    // How the +x and the -x end of the sphere move under one drag along the X arrow.
    let ends = |w: &mut Window| {
        let (points, path) = (positions(w), path_x(w));
        stroke(w, &path, Modifiers::default());
        let after = positions(w);
        let end = |most: bool| {
            let x = |i: &usize| points[*i][0];
            let ids = 0..points.len();
            let i = if most {
                ids.max_by(|a, b| x(a).total_cmp(&x(b)))
            } else {
                ids.min_by(|a, b| x(a).total_cmp(&x(b)))
            };
            let i = i.unwrap_or_else(|| panic!("no points"));
            std::array::from_fn::<f64, 3, _>(|k| after[i][k] - points[i][k])
        };
        [end(true), end(false)]
    };
    let apart = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max);
    w.ok("symmetry.set", json!({ "x_type": "mirror" }));
    let [right, left] = ends(&mut w);
    assert!(applied(&w), "the drag is logged as a transform");
    assert!(
        right[0].abs() > 1e-3,
        "the drag moved the +x side: {right:?}"
    );
    let mirrored = [-right[0], right[1], right[2]];
    assert!(
        apart(left, mirrored) < 1e-4,
        "under Mirror the -x side moved as its mirror image: {left:?} {right:?}"
    );

    w.ok("symmetry.set", json!({ "x_type": "parallel" }));
    let [right, left] = ends(&mut w);
    let logged = w
        .logged()
        .into_iter()
        .rev()
        .find(|(c, _)| c == "transform.apply");
    let symmetry = logged.map(|(_, args)| args["symmetry"].clone());
    assert_eq!(
        symmetry,
        Some(
            json!({ "mirror": { "x": false, "y": false, "z": false }, "parallel": { "x": true, "y": false, "z": false } })
        ),
        "the drag sends the Parallel plane"
    );
    assert!(right[0].abs() > 1e-3, "the drag moved: {right:?}");
    assert!(
        apart(left, right) < 1e-4,
        "under Parallel the -x side moved the same way: {left:?} {right:?}"
    );
    let said = problem(&w).unwrap_or_default();
    assert!(!said.contains("parallel"), "{said:?}");
}

/// While Select Rect's outline is dragged (Shift+Ctrl) with X Mirror or X Parallel on, the
/// window draws the outline and, across the X plane, the copy the hide also applies (gs-core
/// `tests/core/gestures.rs`); nothing once the pen lifts. From the front the X plane is seen
/// edge on, so the copy is the outline reflected about it on the screen.
#[test]
fn a_dragged_select_outline_is_drawn_with_its_mirrored_copy() {
    let (mut w, rect) = turned(&json!({ "azimuth_deg": 0, "elevation_deg": 0 }));
    let [cx, cy] = drawn_at(&w, rect, [0.0; 3]);
    let right = bounds(&mut w)[1][0];
    let r = drawn_at(&w, rect, [right, 0.0, 0.0])[0] - cx;
    // Off the view's axis, a point on the X plane is drawn above the origin only when the
    // plane is seen edge on.
    let on_plane = drawn_at(&w, rect, [0.0, 0.5, 0.5]);
    assert!(
        r > 100.0 && (on_plane[0] - cx).abs() < 0.5,
        "a front view: {r}"
    );
    let select = Modifiers {
        shift: true,
        ctrl: true,
        ..Modifiers::default()
    };
    // A box over the sphere's +x side, as window pixels: the pen drags across it, corner to
    // corner (Select Rect, decision 0051).
    let path = [[0.2, -0.3], [0.4, 0.0], [0.6, 0.3]];
    let path = path.map(|[x, y]| [cx + x * r, cy + y * r]);
    let outlines = |w: &mut Window| view_widgets(w, &["Gesture outline", "Mirrored outline"]);
    for (t, kind) in [(0, "mirror"), (100, "parallel")] {
        w.ok("symmetry.set", json!({ "x": true, "x_type": kind }));
        w.input(&[], select);
        w.pen(pen_at(t, path[0][0], path[0][1], 0.0));
        for (k, &[x, y]) in (1..).zip(&path) {
            w.pen(pen_at(t + 4 * k, x, y, 0.5));
            w.input(&[], select);
        }
        let drawn = outlines(&mut w);
        let labels: Vec<&str> = drawn.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(labels, ["Gesture outline", "Mirrored outline"], "{kind}");
        let [own, copy] = [drawn[0].1, drawn[1].1];
        let box_ = [path[0][0], path[0][1], path[2][0], path[2][1]].map(f64::from);
        let mirrored = [
            2.0 * f64::from(cx) - box_[2],
            box_[1],
            2.0 * f64::from(cx) - box_[0],
            box_[3],
        ];
        for (got, want, what) in [(own, box_, "the outline"), (copy, mirrored, "its copy")] {
            let off = (0..4).map(|i| (got[i] - want[i]).abs()).fold(0.0, f64::max);
            assert!(off < 1.5, "{kind}: {what} drawn at {got:?}, not {want:?}");
        }
        let [x, y] = path[2];
        w.pen(pen_at(t + 20, x, y, 0.0));
        w.input(&[], Modifiers::default());
        assert_eq!(
            outlines(&mut w),
            Vec::new(),
            "{kind}: none once the pen lifts"
        );
        // The next press lands on the model again, the keys let go longer than a tap, which
        // would step the Select slot to its next brush (gs-ui `held.rs`, `TAP`).
        w.ok("visibility.show_all", json!({}));
        w.wait(0.5);
    }
}
