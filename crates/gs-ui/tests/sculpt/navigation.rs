//! Navigating with the pen in the window (decision 0017), through the real window host and
//! engine as the window drives them: a barrel drag turns the view anywhere and never sculpts,
//! as one log line that replays to the same camera; a tip press on empty canvas turns the view
//! (the classic scheme) while one on the model sculpts; Alt held pans and Alt tapped first
//! dollies; barrel button 2 pans; Alt with a barrel click picks the part under it; the Zoom and Pan buttons under the corner figure move the
//! view with the tip alone, one gesture per drag; the corner figure looks from the side clicked; a drag over the Reference View turns
//! main while their rotation is linked and only the reference once it is not; every open
//! reference shows in each place a layout can dock it and settles, and Frame acts on the picture under the
//! pointer, else the active one; and a look through `main` shows what the window shows.

use gs_engine::ExactCamera;
use gs_platform::{Button, Input, Modifiers};
use gs_protocol::{PenSample, ScriptLine, Step};
use gs_render::Camera;
use gs_testkit::Session;
use gs_ui::RefPlace;
use serde_json::json;

use crate::region;
use crate::rig::{Window, log_path, pen_at, with_sphere};

/// `main`'s camera as JSON text (equal text is equal bits), and as a camera.
fn main_camera(w: &mut Window) -> (String, Camera) {
    let v = w.ok("view.get", json!({ "view": "main" }))["camera"].clone();
    let exact: ExactCamera = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}"));
    (
        v.to_string(),
        exact.camera("").unwrap_or_else(|e| panic!("{}", e.message)),
    )
}

/// Document undo steps.
fn steps(w: &mut Window) -> usize {
    let list = w.ok("history.list", json!({}));
    list["steps"].as_array().map_or(0, Vec::len)
}

/// One pen sample, then one frame of the window with `held` and the commands it asks for.
fn frame(w: &mut Window, s: PenSample, held: Modifiers) {
    w.pen(s);
    w.input(&[], held);
}

/// A drag of the pen from `from` to `to` in five frames, touching (`tip`) or with the barrel
/// button held, then lifted.
fn pen_drag(w: &mut Window, from: [f32; 2], to: [f32; 2], tip: bool, held: Modifiers) {
    let at = |k: u64| {
        let f = k as f32 / 4.0;
        let (x, y) = (
            from[0] + (to[0] - from[0]) * f,
            from[1] + (to[1] - from[1]) * f,
        );
        let mut s = pen_at(4 * k + 4, x, y, if tip { 0.5 } else { 0.0 });
        s.buttons = u8::from(!tip);
        s
    };
    frame(w, pen_at(0, from[0], from[1], 0.0), held);
    for k in 0..5 {
        frame(w, at(k), held);
    }
    let mut up = at(4);
    (up.contact, up.pressure, up.buttons) = (false, 0.0, 0);
    up.t_ns += 4_000_000;
    frame(w, up, held);
}

/// [`region::middle`] for the window's pixels as this file keeps them.
fn middle(rect: [f32; 4]) -> [f32; 2] {
    region::middle(rect.map(f64::from)).map(|v| v as f32)
}

#[test]
fn a_barrel_drag_over_the_model_turns_the_view_never_sculpts_and_replays() {
    let log = log_path("barrel");
    let (mut w, rect) = with_sphere(Some(log.clone()));
    let (hash, undo) = (w.hash(), steps(&mut w));
    let (before, _) = main_camera(&mut w);
    let [cx, cy] = middle(rect);
    pen_drag(
        &mut w,
        [cx, cy],
        [cx + 120.0, cy + 40.0],
        false,
        Modifiers::default(),
    );
    let (after, _) = main_camera(&mut w);
    assert_ne!(after, before, "the view turned");
    assert_eq!(
        (w.hash(), steps(&mut w)),
        (hash, undo),
        "nothing was sculpted or undoable"
    );
    let records = w.records();
    let sets: Vec<&gs_protocol::LogRecord> = (records.iter())
        .filter(|r| r.command == "view.set")
        .collect();
    assert_eq!(
        sets.len(),
        2,
        "the framing main kept, and the drag: one line each"
    );
    assert_eq!(
        sets[1].args["camera"].to_string(),
        after,
        "the drag's line holds its camera"
    );
    let mut replay = Session::new();
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("{e}"));
    for line in text.lines() {
        if let Ok(ScriptLine::Record(r)) = ScriptLine::parse(line) {
            replay.request(r.request().unwrap_or_else(|e| panic!("{e}")));
        }
    }
    let replayed = replay.query("view.get", json!({ "view": "main" }))["camera"].to_string();
    assert_eq!(
        replayed, after,
        "a replay without the window gives the same camera"
    );
}

#[test]
fn a_tip_press_on_empty_canvas_turns_the_view_and_on_the_model_sculpts() {
    let (mut w, rect) = with_sphere(None);
    let none = Modifiers::default();
    let corner = [rect[0] + 30.0, rect[3] - 30.0];
    let (start, _) = main_camera(&mut w);
    let hash = w.hash();
    pen_drag(&mut w, corner, [corner[0] + 80.0, corner[1]], true, none);
    let (turned, _) = main_camera(&mut w);
    assert_ne!(turned, start, "empty canvas: the view turned");
    assert_eq!(w.hash(), hash, "and nothing was sculpted");
    let [cx, cy] = middle(rect);
    pen_drag(&mut w, [cx - 40.0, cy], [cx + 40.0, cy], true, none);
    assert_ne!(w.hash(), hash, "on the model: it sculpted");
    assert_eq!(main_camera(&mut w).0, turned, "and the view stayed");
    w.ok("view.lock", json!({ "locked": true }));
    pen_drag(&mut w, corner, [corner[0] + 80.0, corner[1]], true, none);
    assert_eq!(main_camera(&mut w).0, turned, "a locked view does not turn");
    w.ok("view.lock", json!({ "locked": false }));
    w.ok("ui.navigation", json!({ "classic": false }));
    pen_drag(&mut w, corner, [corner[0] + 80.0, corner[1]], true, none);
    assert_eq!(
        main_camera(&mut w).0,
        turned,
        "with the classic scheme off, it does not either"
    );
}

#[test]
fn alt_held_pans_and_alt_tapped_first_dollies_on_empty_canvas() {
    let (mut w, rect) = with_sphere(None);
    let corner = [rect[0] + 30.0, rect[3] - 30.0];
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let (_, c0) = main_camera(&mut w);
    w.input(&[], alt);
    pen_drag(&mut w, corner, [corner[0] + 80.0, corner[1]], true, alt);
    w.input(&[], Modifiers::default());
    let (_, c1) = main_camera(&mut w);
    assert_eq!(c1.orientation, c0.orientation, "Alt held: no turn");
    assert_eq!(c1.distance, c0.distance, "no dolly");
    assert_ne!(c1.target, c0.target, "a pan");
    // Alt pressed and let go before the press.
    w.input(&[], alt);
    w.input(&[], Modifiers::default());
    pen_drag(
        &mut w,
        corner,
        [corner[0], corner[1] - 60.0],
        true,
        Modifiers::default(),
    );
    let (_, c2) = main_camera(&mut w);
    assert_eq!(c2.orientation, c1.orientation, "Alt tapped: no turn");
    assert!(
        c2.distance < c1.distance,
        "it dollied closer: {} from {}",
        c2.distance,
        c1.distance
    );
}

#[test]
fn the_corner_figure_looks_from_the_side_clicked_then_from_the_other() {
    // The figure shows in a view at least three figures across, wider than the small window
    // leaves beside the trays.
    let mut w = Window::sized(None, gs_ui::Keymap::default(), [1280, 800], 1.0);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    for want in [-1.0, 1.0] {
        let [x, y] = region::middle(w.rect("figure > Look from right"));
        let click = |down| Input::MouseButton {
            button: Button::Left,
            down,
            x,
            y,
        };
        w.input(
            &[Input::MouseMove { x, y }, click(true)],
            Modifiers::default(),
        );
        w.input(&[click(false)], Modifiers::default());
        let forward = main_camera(&mut w).1.axes().2;
        assert!(
            (forward.x - want).abs() < 1e-5 && forward.y.abs() < 1e-5,
            "looking along {want} X: {forward:?}"
        );
    }
}

/// The `view.set` lines in the window's log so far: one per gesture.
fn view_sets(w: &Window) -> usize {
    w.records()
        .iter()
        .filter(|r| r.command == "view.set")
        .count()
}

#[test]
fn the_zoom_and_pan_buttons_under_the_figure_move_the_view_with_the_tip_alone() {
    let mut w = Window::sized(
        Some(log_path("pads")),
        gs_ui::Keymap::default(),
        [1280, 800],
        1.0,
    );
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let none = Modifiers::default();
    let (hash, undo) = (w.hash(), steps(&mut w));
    let (_, c0) = main_camera(&mut w);
    let sets = view_sets(&w);
    let zoom = middle(w.rect("pads > Zoom").map(|v| v as f32));
    pen_drag(&mut w, zoom, [zoom[0], zoom[1] - 100.0], true, none);
    let (_, c1) = main_camera(&mut w);
    let ratio = f64::from(c1.distance / c0.distance);
    assert!(
        (ratio - libm::pow(1.005, -100.0)).abs() < 1e-3,
        "100 points up come 0.5% closer each: {ratio}"
    );
    assert_eq!(
        (c1.orientation, c1.target),
        (c0.orientation, c0.target),
        "no turn, no pan"
    );
    assert_eq!(view_sets(&w), sets + 1, "the drag is one gesture");
    let pan = middle(w.rect("pads > Pan").map(|v| v as f32));
    pen_drag(&mut w, pan, [pan[0] + 60.0, pan[1]], true, none);
    let (_, c2) = main_camera(&mut w);
    assert_eq!(
        (c2.orientation, c2.distance),
        (c1.orientation, c1.distance),
        "no turn, no dolly"
    );
    assert_ne!(c2.target, c1.target, "a pan");
    assert_eq!(view_sets(&w), sets + 2, "one gesture again");
    assert_eq!(
        (w.hash(), steps(&mut w)),
        (hash, undo),
        "the tip on a button never sculpts"
    );
}

#[test]
fn barrel_button_two_pans_over_the_model_like_the_middle_mouse_button() {
    let (mut w, rect) = with_sphere(None);
    let none = Modifiers::default();
    let (hash, (_, c0)) = (w.hash(), main_camera(&mut w));
    let [cx, cy] = middle(rect);
    let at = |k: u64| {
        let mut s = pen_at(4 * k + 4, cx + 15.0 * k as f32, cy, 0.0);
        s.buttons = 2;
        s
    };
    frame(&mut w, pen_at(0, cx, cy, 0.0), none);
    for k in 0..5 {
        frame(&mut w, at(k), none);
    }
    frame(&mut w, pen_at(30, cx + 60.0, cy, 0.0), none);
    let (_, c1) = main_camera(&mut w);
    assert_eq!(
        (c1.orientation, c1.distance),
        (c0.orientation, c0.distance),
        "no turn, no dolly"
    );
    let moved = gs_core::Vec3::from_array(c1.target) - gs_core::Vec3::from_array(c0.target);
    assert!(
        moved.dot(c0.axes().0) < 0.0,
        "the scene follows the pen right, so the target goes left: {moved:?}"
    );
    assert_eq!(w.hash(), hash, "nothing was sculpted");
}

/// The active part, as `session.state` reports it.
fn active(w: &mut Window) -> serde_json::Value {
    w.ok("session.state", json!({}))["parts.active"].clone()
}

/// A barrel click at `at`, holding `held`: a hover, the button down, the button up.
fn barrel_click(w: &mut Window, at: [f32; 2], held: Modifiers) {
    w.input(&[], held);
    let mut down = pen_at(2, at[0], at[1], 0.0);
    down.buttons = 1;
    for s in [
        pen_at(0, at[0], at[1], 0.0),
        down,
        pen_at(4, at[0], at[1], 0.0),
    ] {
        frame(w, s, held);
    }
}

#[test]
fn alt_with_a_barrel_click_picks_the_part_under_it_and_nothing_on_empty_canvas() {
    let mut w = Window::new(Some(log_path("pick")));
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let cube = json!({ "center": [0.0, 0.0, 1.6], "size": 0.6 });
    w.ok("primitive.cube", cube);
    w.ok("part.select", json!({ "part": "sphere" }));
    let rect = w.view_rect();
    let [cx, cy] = middle(rect);
    let (before, _) = main_camera(&mut w);
    let (hash, undo) = (w.hash(), steps(&mut w));
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    barrel_click(&mut w, [cx, cy], alt);
    assert_eq!(
        active(&mut w),
        json!("cube"),
        "the cube in front of the sphere"
    );
    assert_eq!(
        (w.hash(), steps(&mut w)),
        (hash, undo),
        "a pick changes no mesh and is no undo step"
    );
    let logged = w.logged().into_iter().rfind(|l| l.0 == "part.select");
    assert_eq!(
        logged.map(|l| l.1),
        Some(json!({ "part": "cube" })),
        "the log names the part"
    );
    assert!(w.host.lock().shell.wheel.at.is_none(), "no brush wheel");
    assert_eq!(main_camera(&mut w).0, before, "the view stayed");
    // Beside the cube, on the sphere.
    let side = (1..60)
        .map(|k| [cx + 4.0 * k as f32, cy])
        .find(|&[x, y]| w.picked(x, y)["hit"]["part"] == "sphere")
        .unwrap_or_else(|| panic!("the sphere shows beside the cube"));
    barrel_click(&mut w, side, alt);
    assert_eq!(active(&mut w), json!("sphere"), "the sphere");
    barrel_click(&mut w, [rect[0] + 30.0, rect[3] - 30.0], alt);
    assert_eq!(
        active(&mut w),
        json!("sphere"),
        "empty canvas picks nothing"
    );
    // A hidden part is never picked: the click reaches the sphere behind it.
    let hide = json!({ "parts": ["cube"], "visible": false });
    w.ok("part.set_visible", hide);
    let picks = |w: &Window| {
        (w.logged().into_iter())
            .filter(|l| l.0 == "part.select")
            .count()
    };
    let picked = picks(&w);
    barrel_click(&mut w, [cx, cy], alt);
    let last = w.logged().into_iter().rfind(|l| l.0 == "part.select");
    assert_eq!(picks(&w), picked + 1, "the click picked");
    assert_eq!(last.map(|l| l.1), Some(json!({ "part": "sphere" })));
    barrel_click(&mut w, [cx, cy], Modifiers::default());
    assert_eq!(
        active(&mut w),
        json!("sphere"),
        "without Alt nothing is picked"
    );
    assert!(
        w.host.lock().shell.wheel.at.is_some(),
        "and the click opens the wheel, as before"
    );
}

/// The part under the pen is the nearest by scene distance, each part where its transform puts
/// it (decision 0029, section 4): a cube made inside the sphere and moved out in front of it is
/// picked, and moved behind it, the sphere is.
#[test]
fn alt_with_a_barrel_click_picks_a_moved_part_where_it_was_moved() {
    let mut w = Window::new(None);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok("primitive.cube", json!({ "size": 0.6 }));
    let to = |z: f32| json!({ "part": "cube", "op": { "move": [0.0, 0.0, z] } });
    w.ok("transform.apply", to(1.6));
    w.ok("part.select", json!({ "part": "sphere" }));
    let at = middle(w.view_rect());
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    barrel_click(&mut w, at, alt);
    assert_eq!(
        active(&mut w),
        json!("cube"),
        "moved in front of the sphere"
    );
    w.ok("transform.apply", to(-3.2));
    barrel_click(&mut w, at, alt);
    assert_eq!(active(&mut w), json!("sphere"), "moved behind it");
}

#[test]
fn a_look_through_main_shows_what_the_window_shows() {
    let (mut w, _) = with_sphere(None);
    // A layout with no panels and no header, so the 3D view starts at the frame's corner: the
    // occlusion noise and the dither are keyed by the frame's pixel (decision 0019).
    {
        let shown = &mut w.host.lock().shell.directions.list[0];
        shown.panels.clear();
        shown.header = None;
    }
    let rect = w.view_rect();
    assert_eq!([rect[0], rect[1]], [0.0, 0.0], "{rect:?}");
    // Looks render on the window's own device, as in the window.
    let host = w.host.clone();
    w.engine.attach_renderer(Box::new(host));
    let [cx, cy] = middle(rect);
    pen_drag(
        &mut w,
        [cx, cy],
        [cx + 70.0, cy - 30.0],
        false,
        Modifiers::default(),
    );
    // The pen leaves, so the window draws no cursor over the picture.
    let gone = PenSample {
        in_proximity: false,
        ..pen_at(100, cx, cy, 0.0)
    };
    frame(&mut w, gone, Modifiers::default());
    let size = [
        (rect[2] - rect[0]).round() as u32,
        (rect[3] - rect[1]).round() as u32,
    ];
    let window = w.view_pixels();
    let args = json!({ "view": "main", "size": size, "antialias": false });
    let shot = w.ok("view.snapshot", args);
    let look = crate::rig::decode_png(shot["png_base64"].as_str().unwrap_or_default());
    assert_eq!(look.len(), window.len(), "the same size");
    // The corner figure, the Zoom and Pan buttons under it and the view's name draw over the
    // window's picture only, inside the rectangles `ui.describe` gives them; every other pixel
    // is the same bytes: both draw the part from its mesh store (decision 0024).
    let overlays = [w.rect("figure"), w.rect("pads"), w.rect("view_name")];
    let over = |x: usize, y: usize| {
        let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
        (overlays.iter()).any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
    };
    let differ = (look.chunks(4).zip(window.chunks(4)).enumerate())
        .filter(|(i, (a, b))| a != b && !over(i % size[0] as usize, i / size[0] as usize))
        .count();
    assert_eq!(differ, 0, "pixels that differ outside {overlays:?}");
    let all = window.len() / 4;
    let covered = (0..all)
        .filter(|i| over(i % size[0] as usize, i / size[0] as usize))
        .count();
    assert!(
        covered < all / 5,
        "the overlays are small: {covered} of {all}"
    );
}

/// The orientation of `view`'s camera as JSON text (equal text is equal bits).
fn turn_of(w: &mut Window, view: &str) -> String {
    w.ok("view.get", json!({ "view": view }))["camera"]["orientation"].to_string()
}

#[test]
fn a_drag_over_the_reference_turns_main_while_linked_and_only_the_reference_once_unlinked() {
    let (mut w, _) = with_sphere(None);
    w.ok("reference.open", json!({ "scene": true }));
    let none = Modifiers::default();
    w.input(&[], none);
    let [cx, cy] = middle(w.rect("reference > ref:1").map(|v| v as f32));
    let drag = |w: &mut Window| pen_drag(w, [cx - 50.0, cy], [cx + 50.0, cy + 20.0], false, none);
    let before = turn_of(&mut w, "main");
    drag(&mut w);
    let turned = turn_of(&mut w, "main");
    assert_ne!(
        turned, before,
        "a drag over the linked reference turned main"
    );
    assert_eq!(
        turn_of(&mut w, "ref:1"),
        turned,
        "and the reference with it"
    );
    w.ok("reference.link", json!({ "rotation": false }));
    drag(&mut w);
    assert_eq!(turn_of(&mut w, "main"), turned, "unlinked, main stays");
    assert_ne!(
        turn_of(&mut w, "ref:1"),
        turned,
        "and only the reference turns"
    );
}

/// Frames until the window has nothing left to draw (every picture settled), at most 12.
fn settle(w: &mut Window) {
    for _ in 0..12 {
        w.input(&[], Modifiers::default());
        if !w.host.lock().dirty {
            return;
        }
    }
    panic!("the window never settled");
}

/// Each reference picture the window shows, by name, in window pixels.
fn pictures(w: &mut Window) -> Vec<(String, [f32; 4])> {
    let d = w.ok("ui.describe", json!({}));
    (d["panels"].as_array().into_iter().flatten())
        .filter(|p| p["id"] == "reference")
        .flat_map(|p| p["widgets"].as_array().cloned().unwrap_or_default())
        .filter_map(|w| {
            let name = w["label"].as_str().filter(|l| l.starts_with("ref:"))?;
            let r: [f32; 4] = std::array::from_fn(|i| w["rect"][i].as_f64().unwrap_or(0.0) as f32);
            Some((name.to_string(), r))
        })
        .collect()
}

/// How far `view`'s eye is from what it looks at.
fn distance(w: &mut Window, view: &str) -> f64 {
    w.ok("view.get", json!({ "view": view }))["camera"]["distance"]
        .as_f64()
        .unwrap_or_default()
}

/// The `reference` set's Frame control run as a key would run it, with the pen hovering at `at`.
fn frame_control(w: &mut Window, at: [f32; 2]) {
    w.pen(pen_at(0, at[0], at[1], 0.0));
    let step = Step {
        command: "view.frame".into(),
        args: json!({ "view": "reference" }),
    };
    w.host.lock().shell.queue(step);
    w.input(&[], Modifiers::default());
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "pictures share edges exactly, and a view Frame left alone keeps its exact distance"
)]
fn every_open_reference_shows_in_its_place_and_frame_picks_the_one_under_the_pointer() {
    let (mut w, _) = with_sphere(None);
    // Docked beside the view as Workbench places it, and floating over or splitting the view
    // as a layout file may place it.
    for place in [RefPlace::Right, RefPlace::Float, RefPlace::Split] {
        w.host.lock().shell.directions.list[0].reference.place = place;
        let layout = format!("{place:?}");
        w.ok("reference.open", json!({ "scene": true }));
        w.ok("reference.open", json!({ "parts": "sphere" }));
        settle(&mut w);
        let shown = pictures(&mut w);
        let names: Vec<&str> = shown.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["ref:1", "ref:2"], "{layout}");
        let [(_, a), (_, b)] = [&shown[0], &shown[1]].map(Clone::clone);
        // The area both share: the second picture is right of or under the first.
        let (wide, tall) = (b[2] - a[0], b[3] - a[1]);
        let beside = if wide >= tall {
            a[2] <= b[0] && a[1] == b[1] && a[3] == b[3]
        } else {
            a[3] <= b[1] && a[0] == b[0] && a[2] == b[2]
        };
        assert!(beside, "{layout}: split along the longer side: {a:?} {b:?}");
        for (name, r) in &shown {
            assert!(
                r[2] - r[0] > 20.0 && r[3] - r[1] > 20.0,
                "{layout}: {name} {r:?}"
            );
            let studio = w.host.lock();
            let render = studio
                .render
                .as_ref()
                .unwrap_or_else(|| panic!("a renderer"));
            let target = (render.reference_frame(name)).unwrap_or_else(|| panic!("{name} drawn"));
            assert!(target.settled(), "{layout}: {name} settled");
        }
        // Frame: the picture under the pointer, else the active one (ref:2, opened last).
        for v in ["ref:1", "ref:2"] {
            w.ok("view.dolly", json!({ "view": v, "factor": 0.5 }));
        }
        let dollied = [distance(&mut w, "ref:1"), distance(&mut w, "ref:2")];
        frame_control(&mut w, middle(a));
        let framed = [distance(&mut w, "ref:1"), distance(&mut w, "ref:2")];
        assert!(
            framed[0] != dollied[0] && framed[1] == dollied[1],
            "{layout}: {framed:?}"
        );
        let v = w.view_rect();
        let corners = [[v[0] + 4.0, v[1] + 4.0], [v[2] - 4.0, v[3] - 4.0]];
        let inside =
            |p: [f32; 2], r: [f32; 4]| (r[0]..r[2]).contains(&p[0]) && (r[1]..r[3]).contains(&p[1]);
        let off = (corners.into_iter()).find(|p| !inside(*p, a) && !inside(*p, b));
        frame_control(
            &mut w,
            off.unwrap_or_else(|| panic!("{layout}: no bare view")),
        );
        let both = [distance(&mut w, "ref:1"), distance(&mut w, "ref:2")];
        assert!(both[1] != dollied[1], "{layout}: the active one: {both:?}");
        w.ok("reference.close", json!({ "view": "ref:1" }));
        w.ok("reference.close", json!({ "view": "ref:2" }));
        settle(&mut w);
        assert!(pictures(&mut w).is_empty(), "{layout}: closed");
    }
}
