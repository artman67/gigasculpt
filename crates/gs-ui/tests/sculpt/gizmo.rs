//! The gizmo tool in the window (decision 0027, section 11), driven by the pen as the window
//! drives it: a drag on a handle is logged as the one `transform.apply` it made, in scene terms,
//! so a session without a window (another gizmo size, other pixels per point) replays it to the
//! same bits; the `pen.input` calls that drove it are not logged. A drag is rigid unless the
//! gizmo's own falloff says otherwise, whatever the brush's Falloff. A press on the model puts
//! the gizmo there, and Esc cancels a drag. An unlocked gizmo, or Alt held, moves only the
//! gizmo; a sticky one stays where it was while the part moves.

use gs_platform::{Input, Modifiers};
use gs_testkit::Session;
use gs_testkit::measure::dot;
use gs_ui::Keymap;
use serde_json::{Value, json};

use crate::rig::{Window, log_path, pen_at};

/// A sphere in a window wide enough for the gizmo beside the trays, at `density` pixels per
/// point, with the gizmo tool on at `size` points.
pub(crate) fn gizmo_window(name: &str, density: f32, size: f32) -> Window {
    let mut w = Window::sized(
        Some(log_path(name)),
        Keymap::default(),
        [1600, 1000],
        density,
    );
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok("ui.gizmo", json!({ "tool": true, "size": size }));
    w
}

/// A handle as `gizmo.describe` reports it for the window's own camera, its outline moved into
/// window pixels.
struct Handle {
    name: String,
    kind: String,
    grabbable: bool,
    outline: Vec<[f32; 2]>,
}

/// The 3D view's rectangle and camera, as `gizmo.describe` takes them.
fn view(w: &mut Window) -> ([f32; 4], Value) {
    let d = w.ok("ui.describe", json!({}));
    let v = &d["viewport"];
    let r = std::array::from_fn(|i| v["rect"][i].as_f64().unwrap_or_default() as f32);
    (r, v["camera"].clone())
}

/// The gizmo as the window draws it. `gizmo.describe` shares the window's gizmo size and
/// pixels per point, so its outlines are where the window's handles are.
fn handles(w: &mut Window) -> Vec<Handle> {
    let (r, camera) = view(w);
    let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
    let d = w.ok(
        "gizmo.describe",
        json!({ "camera": camera, "viewport": size }),
    );
    let list = d["handles"].as_array().cloned().unwrap_or_default();
    (list.iter())
        .map(|h| Handle {
            name: h["name"].as_str().unwrap_or_default().into(),
            kind: h["kind"].as_str().unwrap_or_default().into(),
            grabbable: h["pickable"] == json!(true) && h["fade"].as_f64() == Some(1.0),
            outline: (h["outline"].as_array().cloned().unwrap_or_default().iter())
                .map(|p| {
                    let at = |i: usize| p[i].as_f64().unwrap_or_default() as f32;
                    [r[0] + at(0), r[1] + at(1)]
                })
                .collect(),
        })
        .collect()
}

/// The longest fully drawn arrow on screen.
fn arrow(w: &mut Window) -> Handle {
    let length = |h: &Handle| {
        let [a, b] = [h.outline[0], h.outline[1]];
        libm::hypotf(b[0] - a[0], b[1] - a[1])
    };
    let arrows = handles(w)
        .into_iter()
        .filter(|h| h.kind == "arrow" && h.grabbable);
    let longest = arrows.max_by(|a, b| length(a).total_cmp(&length(b)));
    longest.unwrap_or_else(|| panic!("no arrow to grab"))
}

/// The point `f` of the way from `a` to `b`.
fn lerp(a: [f32; 2], b: [f32; 2], f: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
}

/// A hover at `path`'s first point, a press there, the pen along `path`, and the lift, with
/// `held` held; after each sample the window takes what the interface asked for, as its
/// frames do.
pub(crate) fn stroke(w: &mut Window, path: &[[f32; 2]], held: Modifiers) {
    press_along(w, path, held);
    lift(w, path, held);
}

/// `stroke` up to its last sample, the pen still down.
fn press_along(w: &mut Window, path: &[[f32; 2]], held: Modifiers) {
    w.input(&[], held);
    let [x, y] = path[0];
    w.pen(pen_at(0, x, y, 0.0));
    for (k, [x, y]) in (1..).zip(path) {
        w.pen(pen_at(4 * k, *x, *y, 0.5));
        w.input(&[], held);
    }
}

/// `stroke`'s lift, after `press_along` the same `path`.
fn lift(w: &mut Window, path: &[[f32; 2]], held: Modifiers) {
    let [x, y] = path[path.len() - 1];
    w.pen(pen_at(4 * (path.len() as u64 + 1), x, y, 0.0));
    w.input(&[], held);
}

/// The logged calls' command names.
fn commands(w: &Window) -> Vec<String> {
    w.logged().into_iter().map(|(c, _)| c).collect()
}

/// The logged calls run again, in a session without a window: its hash.
pub(crate) fn replayed(w: &Window) -> String {
    let mut headless = Session::new();
    for (command, args) in w.logged() {
        if !matches!(command.as_str(), "pen.input" | "view.set") {
            headless.ok(&command, args);
        }
    }
    headless.hash()
}

fn three(v: &Value) -> [f64; 3] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or(f64::NAN))
}

/// Every vertex's position, by id.
pub(crate) fn positions(w: &mut Window) -> Vec<[f64; 3]> {
    let stats = w.ok("mesh.stats", json!({}));
    let count = stats["parts"][0]["vertices"].as_u64().unwrap_or_default() as u32;
    let mut out = Vec::new();
    for start in (0..count).step_by(1000) {
        let ids: Vec<u32> = (start..count.min(start + 1000)).collect();
        let read = w.ok("mesh.sample", json!({ "ids": ids, "attributes": [] }));
        let vertices = read["vertices"].as_array().cloned().unwrap_or_default();
        out.extend(vertices.iter().map(|v| three(&v["position"])));
    }
    assert_eq!(out.len(), count as usize, "{stats}");
    out
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|i| a[i] - b[i]);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The last `transform.apply` logged.
fn last_transform(w: &Window) -> Value {
    let logged = w.logged();
    let apply = logged.iter().rev().find(|(c, _)| c == "transform.apply");
    apply.map(|(_, args)| args.clone()).unwrap_or_default()
}

#[test]
fn a_default_drag_moves_the_whole_part_and_replays_without_a_window() {
    // Not the default gizmo size or pixels per point: the logged line must not depend on them.
    // The brush keeps its soft default Falloff, which a drag does not use.
    let mut w = gizmo_window("gizmo-move", 1.25, 100.0);
    let before = w.hash();
    let points = positions(&mut w);
    let origin = three(&w.ok("gizmo.describe", json!({}))["pose"]["origin"]);
    let held = arrow(&mut w);
    let [a, b] = [held.outline[0], held.outline[1]];
    let path: Vec<_> = (0..=6).map(|k| lerp(a, b, 0.7 + 0.1 * k as f32)).collect();
    stroke(&mut w, &path, Modifiers::default());
    let after = w.hash();
    assert_ne!(after, before, "the drag moved the sphere");
    assert_eq!(
        commands(&w),
        [
            "primitive.sphere",
            "view.set",
            "pen.input",
            "transform.apply",
            "gizmo.place"
        ],
        "only the hover is logged as pen input; the drag is one transform, then the gizmo follows"
    );
    let apply = last_transform(&w);
    assert_eq!(apply["falloff"].as_f64(), Some(-100.0), "rigid: {apply}");
    assert!(
        apply.get("reach").is_none(),
        "a rigid drag reaches everywhere: {apply}"
    );
    let moved = three(&apply["op"]["move"]);
    let axis = ["move_x", "move_y", "move_z"]
        .iter()
        .position(|n| *n == held.name);
    let axis = axis.unwrap_or_else(|| panic!("{}", held.name));
    for (i, m) in moved.iter().enumerate() {
        let along = if i == axis { m.abs() > 1e-3 } else { *m == 0.0 };
        assert!(along, "{} moves along its own axis: {moved:?}", held.name);
    }
    for (p, q) in points.iter().zip(positions(&mut w)) {
        let by: [f64; 3] = std::array::from_fn(|i| q[i] - p[i]);
        assert!(
            distance(by, moved) < 1e-4,
            "every point moved by {moved:?}: {p:?} by {by:?}"
        );
    }
    let follows = three(&w.ok("gizmo.describe", json!({}))["pose"]["origin"]);
    for i in 0..3 {
        let expected = origin[i] + moved[i];
        assert!(
            (follows[i] - expected).abs() < 1e-4,
            "{follows:?} {origin:?} {moved:?}"
        );
    }
    assert_eq!(
        replayed(&w),
        after,
        "the logged lines give the same bits without a window"
    );
    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before, "one undo takes back the whole drag");
}

#[test]
fn a_soft_gizmo_falloff_moves_only_the_points_within_the_brush_size() {
    let mut w = gizmo_window("gizmo-soft", 1.0, 75.0);
    // The brush's own Falloff is rigid: were a drag to use it, every point would move.
    w.ok("brush.set", json!({ "falloff": -100, "size_px": 24 }));
    w.ok("ui.gizmo", json!({ "falloff": 0 }));
    let r = view(&mut w).0;
    let hit = w.picked(f32::midpoint(r[0], r[2]), f32::midpoint(r[1], r[3]))["hit"].clone();
    w.ok(
        "gizmo.place",
        json!({ "origin": hit["position"], "normal": hit["normal"] }),
    );
    let o = three(&hit["position"]);
    let points = positions(&mut w);
    let held = arrow(&mut w);
    let [a, b] = [held.outline[0], held.outline[1]];
    let path: Vec<_> = (0..=6).map(|k| lerp(a, b, 0.7 + 0.1 * k as f32)).collect();
    stroke(&mut w, &path, Modifiers::default());
    let apply = last_transform(&w);
    assert_eq!(
        apply["falloff"].as_f64(),
        Some(0.0),
        "the gizmo's falloff: {apply}"
    );
    let reach = apply["reach"].as_f64().unwrap_or_default();
    assert!(
        reach > 0.0 && reach < 0.5,
        "the brush's Size at the gizmo, small: {apply}"
    );
    let moved = three(&apply["op"]["move"]);
    let now = positions(&mut w);
    let mut near = 0;
    for (p, q) in points.iter().zip(&now) {
        let by = distance(*p, *q);
        if distance(*p, o) > reach {
            assert!(
                by == 0.0,
                "{p:?} is beyond the reach {reach} but moved by {by}"
            );
        } else if distance(*p, o) < reach / 4.0 {
            assert!(
                by > distance(moved, [0.0; 3]) / 2.0,
                "{p:?} near the gizmo moved {by}"
            );
            near += 1;
        }
    }
    assert!(near > 0, "points near the gizmo were checked");
    assert_eq!(
        replayed(&w),
        w.hash(),
        "the logged lines give the same bits without a window"
    );
}

/// A point on the sphere that no handle covers, out from the view's centre, in window pixels,
/// and the pick there.
fn free_point(w: &mut Window) -> ([f32; 2], Value) {
    let (r, _) = view(w);
    let (cx, cy) = (f32::midpoint(r[0], r[2]), f32::midpoint(r[1], r[3]));
    let free = |w: &mut Window, x: f32| {
        let hit = w.picked(x, cy)["hit"].clone();
        let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
        let camera = view(w).1;
        let at = [x - r[0], cy - r[1]];
        let d = w.ok(
            "gizmo.describe",
            json!({ "camera": camera, "viewport": size, "at": at }),
        );
        (d.get("at").is_none() && hit.is_object()).then_some(hit)
    };
    ((0..60).map(|k| cx + 4.0 * k as f32))
        .find_map(|x| free(w, x).map(|hit| ([x, cy], hit)))
        .unwrap_or_else(|| panic!("no free point on the sphere"))
}

#[test]
fn a_press_on_the_model_places_the_gizmo_and_esc_cancels_a_drag() {
    let mut w = gizmo_window("gizmo-place", 1.0, 75.0);
    let before = w.hash();
    let (at, hit) = free_point(&mut w);
    stroke(&mut w, &[at], Modifiers::default());
    assert_eq!(w.hash(), before, "placing the gizmo leaves the sphere");
    let logged = w.logged();
    let (command, args) = logged.last().cloned().unwrap_or_default();
    assert_eq!(command, "gizmo.place", "{logged:?}");
    assert_eq!(args["normal"], hit["normal"], "along the surface: {args}");
    let placed = three(&w.ok("gizmo.describe", json!({}))["pose"]["origin"]);
    let point = three(&hit["position"]);
    for i in 0..3 {
        assert!(
            (placed[i] - point[i]).abs() < 1e-5,
            "{placed:?} at {point:?}"
        );
    }

    // Esc in the middle of a drag puts the sphere back, and nothing is logged for it.
    let held = arrow(&mut w);
    let [a, b] = [held.outline[0], held.outline[1]];
    let esc = |down| Input::Key {
        key: "Escape".into(),
        down,
        repeat: false,
        modifiers: Modifiers::default(),
    };
    w.input(&[], Modifiers::default());
    let start = lerp(a, b, 0.7);
    w.pen(pen_at(100, start[0], start[1], 0.0));
    for k in 0..4 {
        let [x, y] = lerp(a, b, 0.7 + 0.1 * k as f32);
        w.pen(pen_at(104 + 4 * k, x, y, 0.5));
        w.input(&[], Modifiers::default());
    }
    w.input(&[esc(true)], Modifiers::default());
    w.input(&[esc(false)], Modifiers::default());
    w.pen(pen_at(130, b[0], b[1], 0.0));
    w.input(&[], Modifiers::default());
    assert_eq!(w.hash(), before, "Esc put the sphere back");
    let after: Vec<String> = commands(&w).split_off(logged.len());
    assert!(
        after.iter().all(|c| c == "pen.input"),
        "a cancelled drag logs nothing but hovers: {after:?}"
    );
}

#[test]
fn a_turn_snaps_with_shift_and_logs_the_mask_and_mirror_it_was_sent_with() {
    let mut w = gizmo_window("gizmo-turn", 1.0, 75.0);
    w.ok("brush.set", json!({ "flip_mirror": "x" }));
    w.ok("ui.gizmo", json!({ "mask": false }));
    let before = w.hash();
    // The view ring is a whole circle that never fades: an eighth of a turn along it.
    let ring = handles(&mut w).into_iter().find(|h| h.kind == "view_ring");
    let ring = ring.unwrap_or_else(|| panic!("no view ring"));
    let eighth = ring.outline.len() / 8;
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    stroke(&mut w, &ring.outline[..=eighth], shift);
    let after = w.hash();
    assert_ne!(after, before, "the turn moved the sphere");
    let logged = w.logged();
    let turn = logged.iter().find(|(c, _)| c == "transform.apply");
    let turn = turn.map(|(_, args)| args.clone()).unwrap_or_default();
    let degrees = turn["op"]["rotate"]["degrees"].as_f64().unwrap_or_default();
    assert!(
        degrees.abs() >= 5.0 && (degrees / 5.0 - (degrees / 5.0).round()).abs() < 1e-4,
        "Shift snaps to 5 degrees: {turn}"
    );
    assert_eq!(
        turn["mask"],
        json!(false),
        "as the gizmo setting sent it: {turn}"
    );
    assert_eq!(
        turn["symmetry"]["mirror"],
        json!({ "x": true, "y": false, "z": false }),
        "the brush's mirror planes: {turn}"
    );
    assert_eq!(
        replayed(&w),
        after,
        "the logged lines give the same bits without a window"
    );

    // Ten steps of a full turn snap to 36 degrees.
    w.ok("ui.gizmo", json!({ "snap": 10 }));
    let ring = handles(&mut w).into_iter().find(|h| h.kind == "view_ring");
    let ring = ring.unwrap_or_else(|| panic!("no view ring"));
    stroke(&mut w, &ring.outline[..=ring.outline.len() / 8], shift);
    let turn = last_transform(&w);
    let degrees = turn["op"]["rotate"]["degrees"].as_f64().unwrap_or_default();
    assert!(
        degrees.abs() >= 36.0 && (degrees / 36.0 - (degrees / 36.0).round()).abs() < 1e-4,
        "ten steps snap to 36 degrees: {turn}"
    );
    let after = w.hash();

    // With radial symmetry on, the window refuses a drag that would not mirror, and says why
    // and what to do in a person's words ((Rough-tray-notes) (1)).
    let radial = json!({ "symmetry": { "radial": { "axis": "y", "count": 4 } } });
    w.ok("brush.set", radial);
    let held = arrow(&mut w);
    let [a, b] = [held.outline[0], held.outline[1]];
    let path: Vec<_> = (0..=4).map(|k| lerp(a, b, 0.7 + 0.1 * k as f32)).collect();
    stroke(&mut w, &path, Modifiers::default());
    assert_eq!(w.hash(), after, "no drag");
    let problem = w
        .host
        .lock()
        .shell
        .status
        .problem
        .clone()
        .unwrap_or_default();
    assert!(problem.starts_with("Radial symmetry"), "{problem}");
    for what_to_do in ["Symmetry menu", "hold Ctrl"] {
        assert!(problem.contains(what_to_do), "{problem}");
    }
    assert!(!problem.contains("transforms"), "{problem}");
}

/// The X arrow's drag from 70% of the way out, `f` of the arrow's length further.
fn x_path(w: &mut Window, f: f32) -> Vec<[f32; 2]> {
    let x = handle(w, "move_x");
    let [a, b] = [x.outline[0], x.outline[1]];
    (0..=6)
        .map(|k| lerp(a, b, 0.7 + f * k as f32 / 6.0))
        .collect()
}

/// The last logged move, and every point's offset since `points`, which must all be it.
fn moved_by(w: &mut Window, points: &[[f64; 3]]) -> [f64; 3] {
    let moved = three(&last_transform(w)["op"]["move"]);
    for (p, q) in points.iter().zip(positions(w)) {
        let by: [f64; 3] = std::array::from_fn(|i| q[i] - p[i]);
        assert!(distance(by, moved) < 1e-5, "{p:?} by {by:?}, not {moved:?}");
    }
    moved
}

/// R5: a Shift drag on a move arrow sent no spacing, so it did nothing and said nothing.
/// Shift snaps a move along each of the gizmo's axes to whole steps of 0.01 scene units
/// (`gizmo.gizmo-move`): a drag of about 0.537 along X logs 0.54, while without Shift the
/// offset is free. Both move every point by what they log, and replay without a window.
#[test]
fn a_shift_move_snaps_to_hundredths_along_the_gizmo_axes() {
    let mut w = gizmo_window("gizmo-move-snap", 1.0, 75.0);
    let before = w.hash();
    let points = positions(&mut w);
    // The X arrow lies across the view, so the move grows evenly with the pen: one free drag
    // measures it, and a second goes about 0.537. An undo leaves the gizmo where it followed
    // the part to, so each drag starts from the arrow where it is drawn now.
    let path = x_path(&mut w, 0.6);
    stroke(&mut w, &path, Modifiers::default());
    let per = moved_by(&mut w, &points)[0] / 0.6;
    w.ok("history.undo", json!({}));
    let f = (0.537 / per) as f32;
    let path = x_path(&mut w, f);
    stroke(&mut w, &path, Modifiers::default());
    let free = moved_by(&mut w, &points);
    assert!((free[0] - 0.537).abs() < 0.002, "free: {free:?}");
    assert_eq!([free[1], free[2]], [0.0, 0.0], "along X only: {free:?}");
    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before);

    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let path = x_path(&mut w, f);
    stroke(&mut w, &path, shift);
    let snapped = moved_by(&mut w, &points);
    assert!((snapped[0] - 0.54).abs() < 1e-6, "snapped: {snapped:?}");
    assert_eq!([snapped[1], snapped[2]], [0.0, 0.0], "{snapped:?}");
    let after = w.hash();
    assert_eq!(
        replayed(&w),
        after,
        "the snapped move replays to the same bits"
    );
}

/// The part as `part.get` reads it: each point's position and mask (0 with no mask), and
/// whether it is hidden: on no face that shows.
struct Points {
    at: Vec<[f32; 3]>,
    mask: Vec<f32>,
    hidden: Vec<bool>,
}

fn points(w: &mut Window) -> Points {
    let mesh = w.ok("part.get", json!({}));
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let at: Vec<[f32; 3]> = (list(&mesh["positions"]).iter())
        .map(|p| three(p).map(|c| c as f32))
        .collect();
    let mask = (list(&mesh["attributes"]["mask"]["values"]["f32"]).iter())
        .map(|v| v.as_f64().unwrap_or_default() as f32)
        .collect();
    let hide = list(&mesh["attributes"]["hide"]["values"]["u32"]);
    let mut hidden = vec![true; at.len()];
    for (f, face) in list(&mesh["faces"]).iter().enumerate() {
        if hide.get(f).and_then(Value::as_u64).unwrap_or_default() == 0 {
            for v in list(face).iter().filter_map(Value::as_u64) {
                if let Some(h) = hidden.get_mut(v as usize) {
                    *h = false;
                }
            }
        }
    }
    Points { at, mask, hidden }
}

/// Faces turned toward the front camera: the half of a sphere at +Z.
fn front() -> Value {
    json!({ "select": { "by": "facing" }, "camera": "front" })
}

/// `gizmo.gizmo-move` and `gizmo.gizmo`: with the half of a sphere at the back fully masked,
/// dragging a turned gizmo's X arrow by d moves every unmasked point by d along that axis only
/// and leaves every masked point to the bit. With Shift the move along X is a whole number of
/// 0.01 and nothing goes along Y or Z.
#[test]
fn a_move_takes_the_unmasked_points_along_the_gizmo_axis_and_leaves_the_masked() {
    let mut w = gizmo_window("gizmo-move-masked", 1.0, 75.0);
    w.ok("visibility.hide", front());
    w.ok("mask.set", json!({ "value": 1 }));
    w.ok("visibility.show_all", json!({}));
    // Turned 30 degrees about the line of sight, so its X is no scene axis.
    let half = 15f64.to_radians();
    let axes = [0.0, 0.0, libm::sin(half), libm::cos(half)];
    w.ok("gizmo.place", json!({ "part": "sphere", "axes": axes }));
    let x = [libm::cos(2.0 * half), libm::sin(2.0 * half), 0.0];
    let before = points(&mut w);
    let masked = before.mask.iter().filter(|m| **m >= 1.0).count();
    let free = before.mask.iter().filter(|m| **m == 0.0).count();
    assert!(
        masked > 100 && free > 100 && masked + free == before.at.len(),
        "a half each"
    );

    for (held, snaps) in [
        (Modifiers::default(), false),
        (
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            true,
        ),
    ] {
        let path = x_path(&mut w, 0.6);
        stroke(&mut w, &path, held);
        let d = three(&last_transform(&w)["op"]["move"]);
        let along = dot(d, x);
        let off: [f64; 3] = std::array::from_fn(|i| d[i] - along * x[i]);
        assert!(
            along.abs() > 0.05 && distance(off, [0.0; 3]) < 1e-6,
            "along X: {d:?}"
        );
        if snaps {
            let steps = along / 0.01;
            assert!((steps - steps.round()).abs() < 1e-4, "snapped: {along}");
        }
        let after = points(&mut w);
        for (i, (p, q)) in before.at.iter().zip(&after.at).enumerate() {
            if before.mask[i] >= 1.0 {
                assert_eq!(p, q, "masked point {i} stays");
            } else {
                let by: [f64; 3] = std::array::from_fn(|a| f64::from(q[a] - p[a]));
                assert!(
                    distance(by, d) < 1e-5,
                    "point {i} moved by {by:?}, not {d:?}"
                );
            }
        }
        w.ok("history.undo", json!({}));
    }
}

/// `gizmo.gizmo`: with the half of a sphere toward the viewer hidden, a drag on the X arrow
/// moves every point that shows by what it logs and leaves every hidden point to the bit; with
/// the whole sphere fully masked, the same drag moves nothing.
#[test]
fn a_drag_leaves_hidden_points_and_a_fully_masked_part_where_they_are() {
    let mut w = gizmo_window("gizmo-hidden-masked", 1.0, 75.0);
    w.ok("visibility.hide", front());
    let before = points(&mut w);
    let hidden = before.hidden.iter().filter(|h| **h).count();
    assert!(
        hidden > 100 && hidden + 100 < before.at.len(),
        "about half hidden: {hidden} of {}",
        before.at.len()
    );
    let path = x_path(&mut w, 0.6);
    stroke(&mut w, &path, Modifiers::default());
    let d = three(&last_transform(&w)["op"]["move"]);
    assert!(distance(d, [0.0; 3]) > 0.05, "it moved: {d:?}");
    let after = points(&mut w);
    for (i, (p, q)) in before.at.iter().zip(&after.at).enumerate() {
        if before.hidden[i] {
            assert_eq!(p, q, "hidden point {i} stays");
        } else {
            let by: [f64; 3] = std::array::from_fn(|a| f64::from(q[a] - p[a]));
            assert!(
                distance(by, d) < 1e-5,
                "point {i} moved by {by:?}, not {d:?}"
            );
        }
    }

    w.ok("visibility.show_all", json!({}));
    w.ok("mask.set", json!({ "value": 1 }));
    let (hash, from) = (w.hash(), w.logged().len());
    let path = x_path(&mut w, 0.6);
    stroke(&mut w, &path, Modifiers::default());
    let calls: Vec<String> = since(&w, from).into_iter().map(|(c, _)| c).collect();
    assert!(
        calls.contains(&"transform.apply".to_string()),
        "a drag: {calls:?}"
    );
    assert_eq!(w.hash(), hash, "a fully masked part does not move");
}

/// Whether the button `label` in the Brush tray is lit.
fn lit(w: &mut Window, label: &str) -> bool {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let widgets = (panels.iter().find(|p| p["id"] == "brush"))
        .and_then(|p| p["widgets"].as_array().cloned())
        .unwrap_or_default();
    widgets
        .iter()
        .any(|w| w["label"] == label && w["selected"] == true)
}

/// `gizmo.unmasked-center`: Follow mask, beside Lock and Sticky, is off by default. A tap turns
/// the shown gizmo's follow on and lights it; the gizmo goes to the unmasked half's middle at
/// once, and when the mask is extended it is drawn at the new centre with no gizmo command.
#[test]
fn follow_mask_takes_the_gizmo_to_the_unmasked_centre_as_the_mask_grows() {
    let mut w = gizmo_window("gizmo-follow", 1.0, 75.0);
    w.ok("visibility.hide", front());
    w.ok("mask.set", json!({ "value": 1 }));
    w.ok("visibility.show_all", json!({}));
    let start = origin(&mut w);
    assert!(!lit(&mut w, "Follow mask"), "off by default");
    let from = w.logged().len();
    let calls = tap(&mut w, "Follow mask", Modifiers::default());
    let part = w.ok("ui.gizmo", json!({}))["part"].clone();
    assert_eq!(
        calls,
        [(
            "gizmo.place".to_string(),
            json!({ "part": part, "follow": true })
        )],
        "the tap"
    );
    assert!(lit(&mut w, "Follow mask"), "lit while on");
    let half = steady(&mut w);
    assert!(
        f64::from(half[2]) > start[2] + 0.3,
        "to the unmasked half's middle: {start:?} to {half:?}"
    );

    // A ring more masked, fully, as a mask brush would extend it.
    w.ok("visibility.hide", json!({ "select": { "by": "unmasked" } }));
    w.ok("visibility.grow", json!({}));
    w.ok("mask.set", json!({ "value": 1 }));
    w.ok("visibility.show_all", json!({}));
    // The window reads the session's gizmo on its next frame.
    w.input(&[], Modifiers::default());
    let grown = steady(&mut w);
    assert!(
        grown[2] > half[2] + 1e-3,
        "it followed: {half:?} to {grown:?}"
    );
    let places = since(&w, from)
        .iter()
        .filter(|(c, _)| c == "gizmo.place")
        .count();
    assert_eq!(places, 1, "with no gizmo command");
}

/// A tap on the Brush tray's button `label` with `held` held: the calls it logged.
fn tap(w: &mut Window, label: &str, held: Modifiers) -> Vec<(String, Value)> {
    let r = w.rect(&format!("brush > {label}"));
    let at = [f64::midpoint(r[0], r[2]), f64::midpoint(r[1], r[3])].map(|c| c as f32);
    let from = w.logged().len();
    stroke(w, &[at], held);
    since(w, from)
}

/// Picks `label` from the gizmo's ⋯ in the Brush tray's header, with `held` held: the calls it
/// logged.
fn menu(w: &mut Window, label: &str, held: Modifiers) -> Vec<(String, Value)> {
    tap(w, "···", Modifiers::default());
    tap(w, label, held)
}

/// `gizmo.mesh-to-axis` and `gizmo.reset-orientation` in the window: Mesh to axis and Reset
/// orientation are in the gizmo's ⋯ (decision 0060, section 6). With Alt held a pick sends the
/// gizmo-only form (`gizmo.place` rule `world_origin`, `gizmo.reset`) and every vertex stays;
/// without, it sends the command that moves or turns the part with its gizmo.
#[test]
fn mesh_to_axis_and_reset_orientation_move_the_part_or_with_alt_only_the_gizmo() {
    let mut w = gizmo_window("gizmo-operators", 1.0, 75.0);
    let half = 15f64.to_radians();
    let turned = [0.0, 0.0, libm::sin(half), libm::cos(half)];
    let place = json!({ "part": "sphere", "origin": [0.3, 0.2, 0.1], "axes": turned });
    w.ok("gizmo.place", place.clone());
    let part = w.ok("ui.gizmo", json!({}))["part"].clone();
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let hash = w.hash();
    let reset = menu(&mut w, "Reset orientation", alt);
    assert_eq!(reset, [("gizmo.reset".into(), json!({ "part": part }))]);
    let to_origin = menu(&mut w, "Mesh to axis", alt);
    let rule = json!({ "part": part, "rule": "world_origin" });
    assert_eq!(to_origin, [("gizmo.place".into(), rule)]);
    assert_eq!(origin(&mut w), [0.0; 3], "the gizmo on the origin");
    assert_eq!(w.hash(), hash, "with Alt every vertex stays");

    w.ok("gizmo.place", place);
    let reset = menu(&mut w, "Reset orientation", Modifiers::default());
    let edit = |c: &str| vec![(c.to_string(), json!({ "part": part }))];
    assert_eq!(reset, edit("gizmo.reset_orientation"));
    let squared = w.hash();
    assert_ne!(squared, hash, "the part turned with its gizmo");
    let to_origin = menu(&mut w, "Mesh to axis", Modifiers::default());
    assert_eq!(to_origin, edit("gizmo.mesh_to_axis"));
    assert_ne!(w.hash(), squared, "and moved with it to the origin");
    assert_eq!(origin(&mut w), [0.0; 3], "the gizmo on the origin");
}

/// Decision 0060, section 6: while the Gizmo tool is on, the Brush tray is the gizmo's. Its
/// header is the tool's name and its ⋯, which holds Mesh to axis, Reset orientation and To
/// unmasked center; Basics holds Reach, Softness (the gizmo's falloff), Snap's three steps and
/// Mask; Pose holds Lock, Sticky and Follow mask. The brush's sections leave the tray, and the
/// gizmo's size on screen is in Settings > Cursor.
#[test]
fn the_gizmo_tools_tray_is_its_name_and_menu_basics_and_pose() {
    let mut w = gizmo_window("gizmo-tray", 1.0, 75.0);
    let sections = described(&mut w, "brush", ("sections", "id"));
    assert_eq!(sections, ["brush", "pose"], "Basics, Pose");
    let shown = labels(&mut w, "brush");
    let want = [
        "Gizmo",
        "···",
        "Reach",
        "Gizmo softness",
        "Snap",
        "5°",
        "15°",
        "45°",
        "Mask",
        "Lock",
        "Sticky",
        "Follow mask",
    ];
    for label in want {
        assert!(shown.iter().any(|l| l == label), "{label} in {shown:?}");
    }
    for label in ["Gizmo size", "Mesh to axis", "Imbed", "Spacing", "Turn"] {
        assert!(!shown.iter().any(|l| l == label), "no {label}: {shown:?}");
    }
    tap(&mut w, "···", Modifiers::default());
    let open = labels(&mut w, "brush");
    for label in ["Mesh to axis", "Reset orientation", "To unmasked center"] {
        assert!(open.iter().any(|l| l == label), "the ⋯ holds {label}");
    }
    let calls = tap(&mut w, "To unmasked center", Modifiers::default());
    let placed = calls.first().map(|(c, args)| (c.as_str(), &args["rule"]));
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(placed, Some(("gizmo.place", &json!("unmasked_center"))));
    let after = labels(&mut w, "brush");
    assert!(
        !after.iter().any(|l| l == "Mesh to axis"),
        "a pick shuts it"
    );
}

/// The labels of the window's panel `id`: the cursor's are the 3D view's.
fn labels(w: &mut Window, id: &str) -> Vec<String> {
    described(w, id, ("widgets", "label"))
}

/// Field `field` of everything in list `list` (`widgets` or `sections`) that `ui.describe`
/// reports for the window's panel `id`.
fn described(w: &mut Window, id: &str, (list, field): (&str, &str)) -> Vec<String> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let panel = panels.into_iter().find(|p| p["id"] == id);
    let items = panel.and_then(|p| p[list].as_array().cloned());
    (items.unwrap_or_default().iter())
        .filter_map(|item| Some(item[field].as_str()?.to_string()))
        .collect()
}

/// What the held keys say on the cursor and the status line with the pen hovering at `at`.
fn says(w: &mut Window, at: [f32; 2], keys: Modifiers, said: &str) {
    w.pen(pen_at(0, at[0], at[1], 0.0));
    w.input(&[], keys);
    let (cursor, status) = (labels(w, "viewport"), labels(w, "status"));
    assert!(cursor.iter().any(|l| l == said), "{said}: {cursor:?}");
    assert!(status.iter().any(|l| l == said), "{said}: {status:?}");
}

/// The keys say what they do to the handle a press would grab (R5): Shift snaps a move
/// handle's move to 0.01 and a ring's turn to the gizmo's step; a scale handle does not snap,
/// and its boxes ignore Ctrl. Away from the gizmo, Shift snaps both.
#[test]
fn the_held_keys_say_what_they_do_to_the_handle_under_the_pen() {
    let mut w = gizmo_window("gizmo-keys", 1.0, 75.0);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let x = middle(&handle(&mut w, "move_x"));
    says(&mut w, x, shift, "Shift: Snap moves to 0.01");
    says(&mut w, x, ctrl, "Ctrl: Copy");
    let ring = handle(&mut w, "rotate_view");
    says(&mut w, ring.outline[0], shift, "Shift: Snap turns to 5°");
    w.ok("ui.gizmo", json!({ "snap": 24 }));
    says(&mut w, ring.outline[0], shift, "Shift: Snap turns to 15°");
    let scale = middle(&handle(&mut w, "scale_x"));
    says(&mut w, scale, shift, "Shift: Scale does not snap");
    says(&mut w, scale, ctrl, "Ctrl: No copy on a box");
    let away = w.view_rect();
    says(
        &mut w,
        [away[0] + 20.0, away[1] + 20.0],
        shift,
        "Shift: Snap moves and turns",
    );
}

/// The gizmo's origin, as the session keeps it.
fn origin(w: &mut Window) -> [f64; 3] {
    three(&w.ok("gizmo.describe", json!({}))["pose"]["origin"])
}

/// A drag along the X arrow from 70% of the way out to 130%, with `held` held.
fn drag_x(w: &mut Window, held: Modifiers) {
    let path = path_x(w);
    stroke(w, &path, held);
}

/// The pen's path for `drag_x`.
pub(crate) fn path_x(w: &mut Window) -> Vec<[f32; 2]> {
    let x = handles(w)
        .into_iter()
        .find(|h| h.name == "move_x" && h.grabbable);
    let x = x.unwrap_or_else(|| panic!("the X arrow cannot be grabbed"));
    let [a, b] = [x.outline[0], x.outline[1]];
    (0..=6).map(|k| lerp(a, b, 0.7 + 0.1 * k as f32)).collect()
}

/// The gizmo's origin as the window draws it.
fn drawn(w: &Window) -> [f32; 3] {
    let studio = w.host.lock();
    let (_, pose) = studio
        .shell
        .gizmo
        .drawn()
        .unwrap_or_else(|| panic!("no gizmo drawn"));
    pose.origin.to_array()
}

/// Where the gizmo is drawn once a lift's calls ran: the same for three more frames, and where
/// the session keeps it, so it never jumps after the pen lifts.
fn steady(w: &mut Window) -> [f32; 3] {
    let first = drawn(w);
    for frame in 0..3 {
        w.input(&[], Modifiers::default());
        assert_eq!(drawn(w), first, "frame {frame} after the lift");
    }
    assert_eq!(
        origin(w).map(|c| c as f32),
        first,
        "drawn where the session keeps it"
    );
    first
}

/// Review 0008's S4: a sticky gizmo seemed to follow a drag and jump back when the pen lifted.
/// It does not: held, lifted and three frames on, it is drawn where it stays, and a locked one
/// is drawn where it followed the part to.
#[test]
fn a_held_sticky_gizmo_is_drawn_where_it_stays() {
    let mut w = gizmo_window("gizmo-sticky-drawn", 1.0, 75.0);
    let part = w.ok("ui.gizmo", json!({}))["part"].clone();

    // A locked gizmo is drawn following the part while the pen holds it, and after.
    let start = drawn(&w);
    let path = path_x(&mut w);
    press_along(&mut w, &path, Modifiers::default());
    let held = drawn(&w);
    assert!(held[0] > start[0] + 1e-3, "{start:?} to {held:?}");
    lift(&mut w, &path, Modifiers::default());
    let followed = steady(&mut w);
    assert!(followed[0] > start[0] + 1e-3, "{start:?} to {followed:?}");

    // A sticky one stays where it is drawn while the pen holds it, and after.
    w.ok("gizmo.place", json!({ "part": part, "sticky": true }));
    let start = drawn(&w);
    let path = path_x(&mut w);
    press_along(&mut w, &path, Modifiers::default());
    assert_eq!(
        drawn(&w),
        start,
        "a held sticky gizmo is drawn where it stays"
    );
    lift(&mut w, &path, Modifiers::default());
    assert_eq!(drawn(&w), start, "and stays there once the pen lifts");
    assert_eq!(steady(&mut w), start, "and in the frames after");
}

#[test]
fn an_unlocked_or_alt_drag_moves_only_the_gizmo_and_a_sticky_one_stays() {
    let mut w = gizmo_window("gizmo-lock", 1.0, 75.0);
    let part = w.ok("ui.gizmo", json!({}))["part"].clone();
    let before = w.hash();

    // Unlocked: the gizmo moves along X, the sphere does not.
    w.ok("gizmo.place", json!({ "part": part, "locked": false }));
    let start = origin(&mut w);
    drag_x(&mut w, Modifiers::default());
    assert_eq!(w.hash(), before, "an unlocked drag leaves the sphere");
    let moved = origin(&mut w);
    assert!(moved[0] > start[0] + 1e-3, "{start:?} to {moved:?}");
    assert_eq!([moved[1], moved[2]], [start[1], start[2]], "along X only");
    let logged = w.logged();
    assert_eq!(logged.last().map(|(c, _)| c.as_str()), Some("gizmo.place"));

    // Locked with Alt held: the same.
    w.ok("gizmo.place", json!({ "part": part, "locked": true }));
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    drag_x(&mut w, alt);
    assert_eq!(w.hash(), before, "an Alt drag leaves the sphere");
    assert!(origin(&mut w)[0] > moved[0] + 1e-3, "Alt moved the gizmo");

    // Sticky: the sphere moves, the gizmo stays where it was.
    w.ok("gizmo.place", json!({ "part": part, "sticky": true }));
    let stays = origin(&mut w);
    drag_x(&mut w, Modifiers::default());
    assert_ne!(w.hash(), before, "a locked drag moves the sphere");
    assert_eq!(origin(&mut w), stays, "a sticky gizmo stays");
    let logged = w.logged();
    assert_eq!(
        logged.last().map(|(c, _)| c.as_str()),
        Some("transform.apply"),
        "no gizmo.place follows a sticky drag"
    );
}

/// The unit vector from `a` toward `b`.
fn unit_from(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let d = distance(a, b);
    std::array::from_fn(|i| (b[i] - a[i]) / d)
}

/// The view's forward axis, as the window's camera has it.
fn forward(w: &Window) -> [f64; 3] {
    let f = w.host.lock().shell.main.camera.axes().2;
    f.to_array().map(f64::from)
}

/// The middle of a handle's outline, in window pixels.
fn middle(h: &Handle) -> [f32; 2] {
    let n = h.outline.len() as f32;
    let sum = (h.outline.iter()).fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
    [sum[0] / n, sum[1] / n]
}

fn handle(w: &mut Window, name: &str) -> Handle {
    let found = handles(w).into_iter().find(|h| h.name == name);
    found.unwrap_or_else(|| panic!("no {name} handle"))
}

/// The logged calls a test made since `from` of them, without the pen's hovers.
fn since(w: &Window, from: usize) -> Vec<(String, Value)> {
    let mut logged = w.logged();
    let after = logged.split_off(from.min(logged.len()));
    after
        .into_iter()
        .filter(|(c, _)| c != "pen.input")
        .collect()
}

/// A drag from a press on the model aims the gizmo (decision 0027, section 14): the press puts
/// it on the surface along the normal, and once the pen moves off, its Z points from the press
/// toward the pen, on the plane through the press facing the screen. The lift sends that as
/// one `gizmo.place` with `toward`, which a session without a window replays to the same pose.
#[test]
fn a_drag_from_the_model_aims_the_gizmo_and_the_aim_replays() {
    let mut w = gizmo_window("gizmo-aim", 1.0, 75.0);
    let before = w.hash();
    let (at, hit) = free_point(&mut w);
    let from = w.logged().len();
    let path: Vec<[f32; 2]> = (0..=5)
        .map(|k| [at[0] + 12.0 * k as f32, at[1] - 8.0 * k as f32])
        .collect();
    stroke(&mut w, &path, Modifiers::default());
    assert_eq!(w.hash(), before, "aiming leaves the sphere");
    let calls = since(&w, from);
    let names: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(names, ["gizmo.place", "gizmo.place"], "{calls:?}");
    assert_eq!(
        calls[0].1["normal"], hit["normal"],
        "the press: along the surface"
    );
    let aim = &calls[1].1;
    assert_eq!(
        aim["origin"], calls[0].1["origin"],
        "aimed from the press: {aim}"
    );
    let (o, toward) = (three(&aim["origin"]), three(&aim["toward"]));
    let d = unit_from(o, toward);
    assert!(
        dot(d, forward(&w)).abs() < 1e-4,
        "on the plane facing the screen: {d:?}"
    );
    let pose = w.ok("gizmo.describe", json!({}))["pose"].clone();
    let q = std::array::from_fn(|i| pose["axes"][i].as_f64().unwrap_or(f64::NAN) as f32);
    let z = gs_core::Quat::from_array(q)
        .axes()
        .2
        .to_array()
        .map(f64::from);
    assert!(dot(z, d) > 1.0 - 1e-5, "Z toward the pen: {z:?} {d:?}");
    assert_eq!(
        steady(&mut w),
        o.map(|c| c as f32),
        "drawn where it was put"
    );

    let mut headless = Session::new();
    for (command, args) in w.logged() {
        if !matches!(command.as_str(), "pen.input" | "view.set") {
            headless.ok(&command, args);
        }
    }
    assert_eq!(
        headless.query("gizmo.describe", json!({}))["pose"],
        pose,
        "the logged aim gives the same pose without a window"
    );
}

/// The centre square moves the part in the screen plane (decision 0027, section 14): the move
/// is across the view's forward axis, so the part keeps its depth in the view, and the gizmo
/// ends under the pen. The grip scales evenly, by how much farther from the centre the pen is.
#[test]
fn the_centre_square_moves_in_the_screen_plane_and_the_grip_scales_evenly() {
    let mut w = gizmo_window("gizmo-move-view", 1.0, 75.0);
    let start = origin(&mut w);
    let c = middle(&handle(&mut w, "move_view"));
    let path: Vec<[f32; 2]> = (0..=6)
        .map(|k| [c[0] + 10.0 * k as f32, c[1] + 5.0 * k as f32])
        .collect();
    stroke(&mut w, &path, Modifiers::default());
    let moved = three(&last_transform(&w)["op"]["move"]);
    let length = distance([0.0; 3], moved);
    assert!(length > 1e-3, "the part moved: {moved:?}");
    assert!(
        dot(moved, forward(&w)).abs() < 1e-5 * length.max(1.0),
        "across the view's forward axis only: {moved:?}"
    );
    let end = origin(&mut w);
    assert!(
        distance(end, std::array::from_fn(|i| start[i] + moved[i])) < 1e-5,
        "the gizmo followed: {start:?} to {end:?}"
    );
    let (r, _) = view(&mut w);
    let camera = w.host.lock().shell.main.camera;
    let size = [r[2] - r[0], r[3] - r[1]];
    let seen = camera.project(end.map(|c| c as f32), size);
    let seen = seen.unwrap_or_else(|| panic!("the gizmo is off screen"));
    let pen = path[path.len() - 1];
    let off = libm::hypotf(seen[0] + r[0] - pen[0], seen[1] + r[1] - pen[1]);
    assert!(off < 1.5, "the gizmo ends under the pen: {off} px");

    let grip = middle(&handle(&mut w, "scale"));
    let centre = middle(&handle(&mut w, "move_view"));
    let path: Vec<[f32; 2]> = (0..=6)
        .map(|k| lerp(centre, grip, 1.0 + k as f32 / 6.0))
        .collect();
    stroke(&mut w, &path, Modifiers::default());
    let scale = three(&last_transform(&w)["op"]["scale"]);
    assert!(
        scale.iter().all(|s| (s - scale[0]).abs() < 1e-9) && (scale[0] - 2.0).abs() < 0.02,
        "twice as far from the centre, twice the size: {scale:?}"
    );
}

/// Every name in the scene's list, in order, and the active part's.
fn listed(w: &mut Window) -> (Vec<String>, String) {
    let list = w.ok("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    let name = |p: &Value| p["name"].as_str().unwrap_or_default().to_string();
    let active = rows.iter().find(|p| p["active"] == json!(true));
    (
        rows.iter().map(name).collect(),
        active.map(name).unwrap_or_default(),
    )
}

fn part_hash(w: &mut Window, part: &str) -> Value {
    w.ok("mesh.hash", json!({ "part": part }))["hash"].clone()
}

/// Every level of `copy` is `part`'s, moved by `moved`: the same faces and columns (mask and
/// groups), each point moved whole.
fn same_levels_moved(w: &mut Window, (part, copy): (&str, &str), moved: [f64; 3]) {
    let levels = |w: &mut Window, part: &str| w.ok("level.describe", json!({ "part": part }));
    let described = levels(w, part);
    assert_eq!(
        levels(w, copy)["count"],
        described["count"],
        "as many levels"
    );
    assert_eq!(described["count"], json!(2), "{described}");
    let get = |w: &mut Window, part: &str| w.ok("part.get", json!({ "part": part }));
    for level in [2, 1] {
        for p in [part, copy] {
            w.ok("level.set", json!({ "part": p, "level": level }));
        }
        let (a, b) = (get(w, part), get(w, copy));
        assert_eq!(a["faces"], b["faces"], "level {level}");
        assert_eq!(
            a["attributes"], b["attributes"],
            "level {level}: mask and groups"
        );
        let points = |m: &Value| m["positions"].as_array().cloned().unwrap_or_default();
        let (p, q) = (points(&a), points(&b));
        assert!(!p.is_empty() && p.len() == q.len(), "level {level}");
        for (p, q) in p.iter().zip(&q) {
            let shifted: [f64; 3] = std::array::from_fn(|i| three(p)[i] + moved[i]);
            let gap = distance(shifted, three(q));
            assert!(gap < 1e-5, "level {level}: {p} copied to {q}");
        }
    }
}

/// A Ctrl drag on a handle copies the part (decision 0027, section 14): one `part.duplicate`
/// with the move, the copy below the part and active, with every level, column and group the
/// part has, moved whole even with a mask on, and its gizmo where the drag left it. One undo
/// takes the copy away and makes the part active again; the copy's edits never touch the part;
/// the log replays it without a window.
#[test]
fn a_ctrl_drag_copies_the_part_whole_and_one_undo_takes_the_copy_away() {
    let mut w = gizmo_window("gizmo-copy", 1.0, 75.0);
    let groups = json!({ "maker": { "by": "visible", "coverage": 0.3, "seed": 4 } });
    w.ok("group.make", groups);
    w.ok("level.subdivide", json!({}));
    w.ok("mask.set", json!({ "value": 0.25 }));
    w.ok("level.set", json!({ "level": 1 }));
    let before = w.hash();
    let source = part_hash(&mut w, "sphere");
    let start = origin(&mut w);
    let from = w.logged().len();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    drag_x(&mut w, ctrl);
    let calls = since(&w, from);
    let names: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(names, ["part.duplicate", "gizmo.place"], "{calls:?}");
    let copy = &calls[0].1;
    assert_eq!(copy["name"], json!("sphere-2"), "{copy}");
    let moved = three(&copy["transform"]["op"]["move"]);
    assert!(moved[0].abs() > 1e-3, "along X: {copy}");
    let after = w.hash();
    assert_eq!(
        listed(&mut w),
        (vec!["sphere".into(), "sphere-2".into()], "sphere-2".into()),
        "the copy below the part, active"
    );
    assert_eq!(part_hash(&mut w, "sphere"), source, "the part did not move");
    let shown = w.ok("ui.gizmo", json!({}))["part"].clone();
    assert_eq!(
        shown,
        json!("sphere-2"),
        "the window shows the copy's gizmo"
    );
    let kept = steady(&mut w);
    let want: [f64; 3] = std::array::from_fn(|i| start[i] + moved[i]);
    assert!(
        distance(kept.map(f64::from), want) < 1e-5,
        "the copy's gizmo is where the drag left it: {kept:?}"
    );
    let part_origin = |w: &mut Window, part: &str| {
        let d = w.ok("gizmo.describe", json!({ "part": part }));
        three(&d["pose"]["origin"])
    };
    assert_eq!(
        part_origin(&mut w, "sphere"),
        start,
        "the part's gizmo stays"
    );

    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before, "one undo takes the copy away");
    assert_eq!(listed(&mut w), (vec!["sphere".into()], "sphere".into()));
    w.ok("history.redo", json!({}));
    assert_eq!(w.hash(), after);

    same_levels_moved(&mut w, ("sphere", "sphere-2"), moved);

    // The copy's edits, of its columns and of its points, never touch the part.
    let source = part_hash(&mut w, "sphere");
    w.ok("mask.set", json!({ "part": "sphere-2", "value": 1.0 }));
    let stretch = json!({ "part": "sphere-2", "op": { "scale": [1.5, 1.0, 1.0] }, "mask": false });
    w.ok("transform.apply", stretch);
    assert_eq!(part_hash(&mut w, "sphere"), source, "the part is its own");
    assert_eq!(replayed(&w), w.hash(), "the log replays without a window");
}

/// A JSON array of numbers as the f32s the window sent.
fn f32s<const N: usize>(v: &Value) -> [f32; N] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or(f64::NAN) as f32)
}

/// Whether a gizmo handle is under the window pixel `at`.
fn on_a_handle(w: &mut Window, at: [f32; 2]) -> bool {
    let (r, camera) = view(w);
    let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
    let at = [at[0] - r[0], at[1] - r[1]];
    let d = w.ok(
        "gizmo.describe",
        json!({ "camera": camera, "viewport": size, "at": at }),
    );
    d.get("at").is_some()
}

/// A spot on a cube of one face a side, four pixels in from one of its corners and clear of
/// the gizmo's handles, in window pixels; that corner; and the pick there.
fn near_a_corner(w: &mut Window) -> ([f32; 2], [f32; 3], Value) {
    let (r, _) = view(w);
    let size = [r[2] - r[0], r[3] - r[1]];
    let camera = w.host.lock().shell.main.camera;
    let middle = camera.project([0.0; 3], size);
    let middle = middle.unwrap_or_else(|| panic!("the cube is off screen"));
    for corner in points(w).at {
        let Some(s) = camera.project(corner, size) else {
            continue;
        };
        let (dx, dy) = (middle[0] - s[0], middle[1] - s[1]);
        let l = libm::hypotf(dx, dy);
        let at = [r[0] + s[0] + 4.0 * dx / l, r[1] + s[1] + 4.0 * dy / l];
        let hit = w.picked(at[0], at[1])["hit"].clone();
        let near = distance(three(&hit["position"]), corner.map(f64::from)) < 0.1;
        if near && !on_a_handle(w, at) {
            return (at, corner, hit);
        }
    }
    panic!("no cube corner to click near")
}

/// `gizmo.placement`'s Alt + click: a click four pixels in from a cube's corner puts the
/// gizmo's origin on the corner exactly, along the face's normal, where a plain click puts it
/// where the pen pressed. Neither moves the cube.
#[test]
fn alt_click_near_a_cube_corner_puts_the_gizmo_on_the_corner() {
    let mut w = Window::sized(
        Some(log_path("gizmo-corner")),
        Keymap::default(),
        [1600, 1000],
        1.0,
    );
    w.ok("primitive.cube", json!({}));
    w.ok("ui.gizmo", json!({ "tool": true, "size": 75.0 }));
    let before = w.hash();
    let (at, corner, hit) = near_a_corner(&mut w);

    let from = w.logged().len();
    stroke(&mut w, &[at], Modifiers::default());
    let calls = since(&w, from);
    assert_eq!(calls.len(), 1, "{calls:?}");
    let plain = three(&calls[0].1["origin"]);
    let pressed = three(&hit["position"]);
    assert!(distance(plain, pressed) < 1e-4, "where the pen pressed");
    assert!(
        distance(plain, corner.map(f64::from)) > 1e-3,
        "not the corner"
    );

    // Back on the cube's middle, so no handle lies under the next press.
    w.ok(
        "gizmo.place",
        json!({ "part": "cube", "rule": "world_origin" }),
    );
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let from = w.logged().len();
    stroke(&mut w, &[at], alt);
    let calls = since(&w, from);
    let names: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(names, ["gizmo.place"], "{calls:?}");
    assert_eq!(f32s(&calls[0].1["origin"]), corner, "on the corner exactly");
    assert_eq!(calls[0].1["normal"], hit["normal"], "along the face");
    assert_eq!(origin(&mut w).map(|c| c as f32), corner);
    assert_eq!(w.hash(), before, "placing the gizmo leaves the cube");
}

/// The axes of the gizmo as the window draws it.
fn drawn_axes(w: &Window) -> [f32; 4] {
    let studio = w.host.lock();
    let drawn = studio.shell.gizmo.drawn();
    let (_, pose) = drawn.unwrap_or_else(|| panic!("no gizmo drawn"));
    pose.axes.to_array()
}

/// `gizmo.placement`'s Alt + drag off the model: from a press on the sphere, the pen beyond
/// the sphere on every side in turn squares the gizmo to the screen (X right, Y up, Z at the
/// viewer), so its axes never roll or flip as the pen goes round the model; with Shift too it
/// squares to the scene axes. The lift sends one `gizmo.place` with the press's origin and
/// those axes, and the log replays to the same pose without a window.
#[test]
fn an_alt_drag_off_the_model_squares_the_gizmo_to_the_screen_or_with_shift_the_scene() {
    let mut w = gizmo_window("gizmo-square", 1.0, 75.0);
    // Turned, so the screen's axes are not the scene's.
    w.ok("view.set", json!({ "camera": "iso" }));
    w.input(&[], Modifiers::default());
    let before = w.hash();
    let (r, _) = view(&mut w);
    let size = [r[2] - r[0], r[3] - r[1]];
    let camera = w.host.lock().shell.main.camera;
    assert_ne!(camera.orientation, gs_core::Quat::IDENTITY, "a turned view");
    let middle = camera.project([0.0; 3], size);
    let middle = middle.unwrap_or_else(|| panic!("the sphere is off screen"));
    let c = [r[0] + middle[0], r[1] + middle[1]];
    let edge = (0..400).map(|k| 2.0 * k as f32);
    let edge = edge
        .clone()
        .find(|k| !w.picked(c[0] + k, c[1])["hit"].is_object());
    let reach = (edge.unwrap_or_else(|| panic!("the sphere fills the row")) + 20.0) / 2f32.sqrt();
    let round = [[1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
    let off: Vec<[f32; 2]> = (round.iter())
        .map(|[a, b]| [c[0] + a * reach, c[1] + b * reach])
        .collect();
    for p in &off {
        assert!(!w.picked(p[0], p[1])["hit"].is_object(), "off the sphere");
        assert!(
            r[0] < p[0] && p[0] < r[2] && r[1] < p[1] && p[1] < r[3],
            "in the view"
        );
    }

    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let scene = Modifiers { shift: true, ..alt };
    for (held, want) in [(alt, camera.orientation), (scene, gs_core::Quat::IDENTITY)] {
        let want = want.to_array();
        w.ok(
            "gizmo.place",
            json!({ "part": "sphere", "rule": "world_origin" }),
        );
        let (at, _) = free_point(&mut w);
        let path: Vec<[f32; 2]> = std::iter::once(at).chain(off.iter().copied()).collect();
        let from = w.logged().len();
        w.input(&[], held);
        w.pen(pen_at(0, at[0], at[1], 0.0));
        for (k, [x, y]) in (1..).zip(&path) {
            w.pen(pen_at(4 * k, *x, *y, 0.5));
            w.input(&[], held);
            if k > 1 {
                assert_eq!(drawn_axes(&w), want, "sample {k} with {held:?}");
            }
        }
        lift(&mut w, &path, held);
        let calls = since(&w, from);
        let names: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["gizmo.place", "gizmo.place"], "{calls:?}");
        let squared = &calls[1].1;
        assert_eq!(f32s(&squared["axes"]), want, "{squared}");
        assert_eq!(squared["origin"], calls[0].1["origin"], "from the press");
        let pose = w.ok("gizmo.describe", json!({}))["pose"].clone();
        assert_eq!(f32s(&pose["axes"]), want, "kept so: {pose}");
    }
    assert_eq!(w.hash(), before, "squaring the gizmo leaves the sphere");

    let pose = w.ok("gizmo.describe", json!({}))["pose"].clone();
    let mut headless = Session::new();
    for (command, args) in w.logged() {
        if !matches!(command.as_str(), "pen.input" | "view.set") {
            headless.ok(&command, args);
        }
    }
    assert_eq!(
        headless.query("gizmo.describe", json!({}))["pose"],
        pose,
        "the logged squaring gives the same pose without a window"
    );
}

/// `canvas.edit-mode`, as far as the gizmo tool stands for its Move mode: with the tool off a
/// stroke on the sphere sculpts (`brush.stroke`), and with it on the same stroke, over the X
/// arrow, moves the sphere (`transform.apply`); one switch, so never both at once.
#[test]
fn the_same_stroke_sculpts_with_the_tool_off_and_moves_the_part_with_it_on() {
    let mut w = gizmo_window("gizmo-edit-mode", 1.0, 75.0);
    let path = x_path(&mut w, 0.6);
    let before = w.hash();
    let names = |calls: Vec<(String, Value)>| -> Vec<String> {
        calls.into_iter().map(|(c, _)| c).collect()
    };

    w.ok("ui.gizmo", json!({ "tool": false }));
    let from = w.logged().len();
    stroke(&mut w, &path, Modifiers::default());
    assert_eq!(names(since(&w, from)), ["brush.stroke"], "Draw sculpts");
    assert_ne!(w.hash(), before, "the stroke changed the sphere");
    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before);

    w.ok("ui.gizmo", json!({ "tool": true }));
    let from = w.logged().len();
    stroke(&mut w, &path, Modifiers::default());
    let calls = names(since(&w, from));
    assert_eq!(calls, ["transform.apply", "gizmo.place"], "the gizmo moves");
    let moved = three(&last_transform(&w)["op"]["move"]);
    assert!(
        moved[0] > 1e-3 && moved[1] == 0.0 && moved[2] == 0.0,
        "{moved:?}"
    );
}
