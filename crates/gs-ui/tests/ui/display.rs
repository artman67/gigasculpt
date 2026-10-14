//! The session's display (decision 0023) through gs-ui's render hosts: `view.display`,
//! `view.environment` and `light.set` change what the looks and the window draw, and setting
//! them back draws the same bytes again. Ambient occlusion's strength and radius among them
//! (decision 0019 as amended for Q8), and the floor and the mask's tint (decision 0023 as
//! amended for Q2).

use gs_engine::{Engine, EngineConfig};
use gs_ui::Offscreen;
use serde_json::{Value, json};

use crate::canvas::{at, rect, shot, studio, view};
use crate::looks::{Shot, headless, run, snapshot};

/// The mean RGB byte of the pixels the surface covers.
fn mean(shot: &Shot) -> f64 {
    let background = gs_render::BACKGROUND;
    let covered: Vec<&[u8]> = shot.1.chunks(4).filter(|p| *p != background).collect();
    let sum: f64 = covered
        .iter()
        .flat_map(|p| &p[..3])
        .map(|&b| f64::from(b))
        .sum();
    sum / (covered.len() * 3).max(1) as f64
}

#[test]
fn the_display_changes_what_looks_draw() {
    let mut e = headless();
    let front = json!({ "view": "front", "size": 96 });
    let with = |shading: &str| json!({ "view": "front", "size": 96, "shading": shading });
    let look = |e: &mut _| snapshot(e, front.clone());
    let matcap = look(&mut e);
    let lit = snapshot(&mut e, with("lit"));
    assert_ne!(lit, matcap, "lit shading draws the look lit live");
    let out = run(&mut e, "view.display", json!({ "shading": "lit" }));
    assert_eq!(out["shading"], json!("lit"), "{out}");
    assert_eq!(
        look(&mut e),
        lit,
        "a look without shading takes the session's"
    );

    run(&mut e, "view.display", json!({ "exposure": 1 }));
    assert!(mean(&look(&mut e)) > mean(&lit) + 10.0, "a stop brighter");
    run(&mut e, "view.display", json!({ "exposure": 0 }));
    run(&mut e, "light.set", json!({ "light": 1, "on": false }));
    assert!(mean(&look(&mut e)) < mean(&lit) - 5.0, "the key light off");
    run(&mut e, "light.set", json!({ "light": 1, "reset": true }));
    assert_eq!(look(&mut e), lit, "reset draws the same bytes");

    let shipped = run(
        &mut e,
        "view.environment",
        json!({ "name": "studio-small-08" }),
    );
    assert!(
        shipped["key"]["intensity"].as_f64() > Some(0.0),
        "{shipped}"
    );
    let probe = look(&mut e);
    assert_ne!(probe, lit, "another environment, another light");
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/environments/studio_small_08_1k.hdr"
    );
    let read = run(&mut e, "view.environment", json!({ "path": file }));
    assert_eq!(read["name"], json!("studio_small_08_1k.hdr"), "{read}");
    assert_eq!(
        read["key"], shipped["key"],
        "the file's light 1 is the probe's"
    );
    assert_eq!(
        look(&mut e),
        probe,
        "the probe's file lights as the shipped probe"
    );

    // MatCaps keep the light they were baked in.
    run(&mut e, "light.set", json!({ "light": 1, "on": false }));
    run(&mut e, "view.display", json!({ "shading": "matcap" }));
    assert_eq!(look(&mut e), matcap, "MatCap shading ignores the light");
    run(&mut e, "view.environment", json!({ "reset": true }));
    run(&mut e, "light.set", json!({ "light": 1, "reset": true }));
    assert_eq!(
        snapshot(&mut e, with("lit")),
        lit,
        "back to the look's own light"
    );
}

#[test]
fn the_window_draws_the_session_display() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args: Value| crate::canvas::run(e, false, command, args);
    run(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": 8 }),
    );
    // The 3D view's middle, where the framed sphere faces the camera, wherever the trays and
    // the header leave the view.
    let v = rect(&view(&mut engine)).map(|v| v as u32);
    let sphere = |e: &mut _| {
        at(
            &shot(e),
            u32::midpoint(v[0], v[2]),
            u32::midpoint(v[1], v[3]),
        )
    };
    let light = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    let matcap = sphere(&mut engine);
    let shading = |e: &mut _| run(e, "ui.describe", json!({}))["viewport"]["shading"].clone();
    run(&mut engine, "view.display", json!({ "shading": "clay" }));
    assert_eq!(shading(&mut engine), json!("clay"));
    assert_ne!(sphere(&mut engine), matcap, "the window draws clay");
    // Lit gray matches its MatCap where the sphere faces the camera, as it should; turning the
    // key light off shows the window lights it live.
    run(&mut engine, "view.display", json!({ "shading": "lit" }));
    assert_eq!(shading(&mut engine), json!("lit gray"));
    run(&mut engine, "light.set", json!({ "light": 1, "on": false }));
    let unlit = sphere(&mut engine);
    assert!(light(unlit) + 30 < light(matcap), "{unlit:?} {matcap:?}");
    run(&mut engine, "view.display", json!({ "shading": "matcap" }));
    assert_eq!(
        sphere(&mut engine),
        matcap,
        "and MatCap again, which ignores the light"
    );
}

/// A headless session on lavapipe with a sphere of radius `scale` and a box pushed into its
/// side, so the box's face meets the sphere in a crease.
pub fn creased(scale: f32) -> Engine {
    let mut e = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let render = Offscreen::host(Some("lavapipe")).unwrap_or_else(|e| panic!("{e}"));
    e.attach_renderer(render);
    let sphere = json!({ "subdivisions": 12, "radius": scale });
    run(&mut e, "primitive.sphere", sphere);
    let size = 1.2 * scale;
    let center = [0.9 * scale, 0.3 * scale, 0.5 * scale];
    let cube = json!({ "subdivisions": 4, "size": size, "center": center });
    run(&mut e, "primitive.cube", cube);
    e
}

/// The mean difference between two pictures of the same size, per channel byte.
fn difference(a: &Shot, b: &Shot) -> f64 {
    assert_eq!(a.0, b.0, "the same size");
    let sum: f64 = (a.1.iter().zip(&b.1))
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum();
    sum / a.1.len() as f64
}

/// Looks draw the session's occlusion: strength 0 lightens the crease and setting it back
/// draws the same bytes, Clay takes it, a contact sheet draws it too, and a radius in scene
/// units draws the same picture of a model twice the size with twice the radius.
#[test]
fn looks_draw_the_sessions_occlusion() {
    let front = json!({ "view": "front", "size": 128 });
    let clay = json!({ "view": "front", "size": 128, "shading": "clay" });
    let display = |e: &mut _, args: Value| {
        run(e, "view.display", args);
    };
    let mut e = creased(1.0);
    let full = snapshot(&mut e, front.clone());
    let full_clay = snapshot(&mut e, clay.clone());
    let sheet = |e: &mut _| {
        run(
            e,
            "view.contact_sheet",
            json!({ "views": ["front"], "size": 128 }),
        )
    };
    let full_sheet = sheet(&mut e);
    display(&mut e, json!({ "occlusion_strength": 0 }));
    let none = snapshot(&mut e, front.clone());
    assert!(mean(&none) > mean(&full), "strength 0 lightens the crease");
    assert_ne!(
        snapshot(&mut e, clay.clone()),
        full_clay,
        "Clay takes occlusion"
    );
    assert_ne!(
        sheet(&mut e),
        full_sheet,
        "a contact sheet draws the setting too"
    );
    display(&mut e, json!({ "occlusion_strength": 1 }));
    assert_eq!(
        snapshot(&mut e, front.clone()),
        full,
        "back to the same bytes"
    );

    display(&mut e, json!({ "occlusion_radius": 0.3 }));
    let small = snapshot(&mut e, front.clone());
    assert_ne!(small, full, "a set radius is not the automatic one here");
    let mut big = creased(2.0);
    display(&mut big, json!({ "occlusion_radius": 0.6 }));
    let twice = snapshot(&mut big, front.clone());
    display(&mut big, json!({ "occlusion_radius": 0.3 }));
    let same_radius = snapshot(&mut big, front.clone());
    let (scaled, unscaled) = (difference(&small, &twice), difference(&small, &same_radius));
    assert!(
        unscaled > 0.03 && scaled < unscaled / 4.0,
        "twice the model with twice the radius draws the same: {scaled} apart, against \
         {unscaled} with the radius left as it was"
    );
    display(&mut e, json!({ "occlusion_radius": "auto" }));
    assert_eq!(
        snapshot(&mut e, front),
        full,
        "and back to the automatic radius"
    );
}

/// The window draws the session's occlusion too: the same scene, strength 0, another picture;
/// strength 1 again, the first picture. Only the 3D view's middle is compared: the status line
/// names the last command.
#[test]
fn the_window_draws_the_sessions_occlusion() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args: Value| crate::canvas::run(e, false, command, args);
    run(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": 12 }),
    );
    let cube = json!({ "subdivisions": 4, "size": 1.2, "center": [0.9, 0.3, 0.5] });
    run(&mut engine, "primitive.cube", cube);
    let middle = |e: &mut _| {
        let shot = shot(e);
        let rows = (100..400).flat_map(|y| (250..650).map(move |x| (x, y)));
        rows.map(|(x, y)| at(&shot, x, y)).collect::<Vec<_>>()
    };
    let full = middle(&mut engine);
    assert!(
        middle(&mut engine) == full,
        "the window draws the same picture again"
    );
    run(
        &mut engine,
        "view.display",
        json!({ "occlusion_strength": 0 }),
    );
    assert!(
        middle(&mut engine) != full,
        "strength 0 draws another picture"
    );
    run(
        &mut engine,
        "view.display",
        json!({ "occlusion_strength": 1 }),
    );
    assert!(
        middle(&mut engine) == full,
        "and strength 1 the first again"
    );
}

/// Review 0005, m7: `session.state` says whether live subsurface runs on the session's device,
/// `auto` resolved, and the pictures agree. Lavapipe is not an integrated GPU, so `auto` runs
/// there and draws as `on` (decision 0023); a session that cannot render reports null.
#[test]
fn session_state_says_whether_subsurface_runs() {
    let running = |e: &mut _| run(e, "session.state", json!({}))["view.subsurface.running"].clone();
    let mode = |e: &mut _, mode: &str| {
        run(e, "view.display", json!({ "subsurface": { "mode": mode } }));
    };
    let mut e = headless();
    assert_eq!(running(&mut e), json!(true), "before the device opens");
    run(&mut e, "view.display", json!({ "shading": "lit" }));
    run(&mut e, "material.select", json!({ "name": "skin-4" }));
    let front = json!({ "view": "front", "size": 96 });
    let auto = snapshot(&mut e, front.clone());
    assert_eq!(running(&mut e), json!(true), "on the open device");
    mode(&mut e, "off");
    assert_eq!(running(&mut e), json!(false));
    assert_ne!(
        snapshot(&mut e, front.clone()),
        auto,
        "off draws another picture"
    );
    mode(&mut e, "on");
    assert_eq!(running(&mut e), json!(true));
    assert_eq!(snapshot(&mut e, front), auto, "auto drew as on");

    let (mut window, host) = studio(1.0);
    window.attach_renderer(Box::new(host));
    let window_running = |e: &mut _| {
        let state = crate::canvas::run(e, false, "session.state", json!({}));
        state["view.subsurface.running"].clone()
    };
    assert_eq!(window_running(&mut window), json!(true), "in the window");
    let mut bare = gs_engine::Engine::new(gs_engine::EngineConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(running(&mut bare), Value::Null, "without a renderer");
}

/// The mask's tint (decision 0023 as amended for Q2): hidden, a masked part draws exactly as
/// with no mask; shown again, the tint is back; less opacity darkens less. Hiding changes only
/// the picture: a stroke over a fully masked part, made while the tint is hidden, moves nothing,
/// and the same stroke moves the part once the mask is cleared.
#[test]
fn hiding_the_mask_changes_the_picture_and_nothing_else() {
    let mut e = headless();
    let front = json!({ "view": "front", "size": 96 });
    let display = |e: &mut _, args: Value| {
        run(e, "view.display", args);
    };
    let plain = snapshot(&mut e, front.clone());
    run(&mut e, "mask.set", json!({ "part": "sphere", "value": 1 }));
    let masked = snapshot(&mut e, front.clone());
    assert!(mean(&masked) < mean(&plain) - 10.0, "the mask darkens");
    display(&mut e, json!({ "mask": false }));
    assert_eq!(
        snapshot(&mut e, front.clone()),
        plain,
        "hidden draws as no mask"
    );
    display(&mut e, json!({ "mask": true }));
    assert_eq!(
        snapshot(&mut e, front.clone()),
        masked,
        "shown, the tint is back"
    );
    display(&mut e, json!({ "mask_opacity": 0.4 }));
    let faint = mean(&snapshot(&mut e, front.clone()));
    assert!(
        faint > mean(&masked) + 5.0 && faint < mean(&plain) - 5.0,
        "less opacity darkens less: {faint} between {} and {}",
        mean(&masked),
        mean(&plain)
    );

    display(&mut e, json!({ "mask": false }));
    let hash = |e: &mut _| run(e, "mesh.hash", json!({ "part": "sphere" }))["hash"].clone();
    let stroke = json!({
        "part": "sphere", "brush": "Draw", "size": "20%",
        "samples": [{ "position": [0, 0, 1] }]
    });
    let before = hash(&mut e);
    run(&mut e, "brush.stroke", stroke.clone());
    assert_eq!(
        hash(&mut e),
        before,
        "masked points stay put while the tint is hidden"
    );
    run(&mut e, "mask.set", json!({ "part": "sphere", "value": 0 }));
    let cleared = hash(&mut e);
    run(&mut e, "brush.stroke", stroke);
    assert_ne!(
        hash(&mut e),
        cleared,
        "the same stroke moves an unmasked part"
    );
}

/// The floor is the window's (decision 0023 as amended for Q2): with it on, a look draws the
/// same bytes as before, `main` and contact sheets included, unless it passes `grid`.
#[test]
fn looks_leave_the_floor_out_unless_they_ask_for_it() {
    let mut e = headless();
    let front = json!({ "view": "front", "size": 96 });
    let main = json!({ "view": "main", "size": 96 });
    let grid = json!({ "view": "front", "size": 96, "grid": true });
    let sheet = |e: &mut _| {
        run(
            e,
            "view.contact_sheet",
            json!({ "views": ["front", "top"], "size": 96 }),
        )
    };
    let shots =
        |e: &mut _| [front.clone(), main.clone(), grid.clone()].map(|args| snapshot(e, args));
    let off = (shots(&mut e), sheet(&mut e));
    assert_ne!(off.0[2], off.0[0], "grid draws the floor");
    run(&mut e, "view.display", json!({ "floor": true }));
    assert!(
        (shots(&mut e), sheet(&mut e)) == off,
        "the window's floor never reaches a look"
    );
}

/// The window draws the session's floor under the parts: Floor on changes only pixels where
/// nothing was drawn, all lower on screen than the middle of the sphere (the plane lies just
/// below it), and Floor off draws the first picture again. Only the 3D view's own pixels are
/// compared, wherever the trays leave it: the header lights its Floor button and the
/// status line names the last command.
#[test]
fn the_window_draws_the_floor_under_the_parts() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args: Value| crate::canvas::run(e, false, command, args);
    run(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": 8 }),
    );
    let [left, top, right, bottom] = crate::canvas::rect(&crate::canvas::view(&mut engine));
    let (rows, columns) = (top as u32..bottom as u32, left as u32..right as u32);
    let points = || {
        rows.clone()
            .flat_map(|y| columns.clone().map(move |x| (x, y)))
    };
    let off = shot(&mut engine);
    let background = gs_render::BACKGROUND;
    let covered: Vec<u32> = (points().filter(|&(x, y)| at(&off, x, y) != background))
        .map(|(_, y)| y)
        .collect();
    let (high, low) = (covered.iter().min(), covered.iter().max());
    let middle = high.zip(low).map(|(h, l)| (h + l) / 2);
    let middle = middle.unwrap_or_else(|| panic!("no sphere in the window"));
    run(&mut engine, "view.display", json!({ "floor": true }));
    let on = shot(&mut engine);
    let changed: Vec<(u32, u32)> = points()
        .filter(|&(x, y)| at(&on, x, y) != at(&off, x, y))
        .collect();
    assert!(
        changed.len() > 500,
        "the floor shows: {} pixels",
        changed.len()
    );
    let over = changed
        .iter()
        .filter(|&&(x, y)| at(&off, x, y) != background)
        .count();
    assert_eq!(over, 0, "the floor draws only where nothing was drawn");
    let above = changed.iter().filter(|&&(_, y)| y <= middle).count();
    assert_eq!(
        above,
        0,
        "the floor lies under the sphere, whose middle is row {middle}: {} pixels changed, \
         highest row {:?}",
        changed.len(),
        changed.iter().map(|p| p.1).min()
    );
    run(&mut engine, "view.display", json!({ "floor": false }));
    let again = shot(&mut engine);
    assert!(
        points().all(|(x, y)| at(&again, x, y) == at(&off, x, y)),
        "Floor off draws the first picture again"
    );
}

/// The window draws the mask's tint as the session says: hidden, the masked sphere draws as
/// before it was masked; shown, as masked.
#[test]
fn the_window_hides_and_shows_the_mask() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args: Value| crate::canvas::run(e, false, command, args);
    run(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": 8 }),
    );
    // The 3D view's pixels only: the scene tray's parts list beside it shows the mask too.
    let [left, top, right, bottom] = rect(&view(&mut engine)).map(|v| v as u32);
    let middle = |e: &mut _| {
        let shot = shot(e);
        let rows = (top..bottom).flat_map(|y| (left..right).map(move |x| (x, y)));
        rows.map(|(x, y)| at(&shot, x, y)).collect::<Vec<_>>()
    };
    let plain = middle(&mut engine);
    run(&mut engine, "mask.set", json!({ "value": 1 }));
    let masked = middle(&mut engine);
    assert!(masked != plain, "the window draws the mask");
    run(&mut engine, "view.display", json!({ "mask": false }));
    assert!(middle(&mut engine) == plain, "hidden draws as no mask");
    run(&mut engine, "view.display", json!({ "mask": true }));
    assert!(middle(&mut engine) == masked, "shown draws the tint again");
}

/// The wire's style (`view.wire`, catalog item `canvas.wireframe-view`) reaches every look that
/// draws a wire, and with no edges and no fill the wire draws the surface alone. A look colors
/// its parts by polygon size only when it passes `density`, whatever the window does.
#[test]
fn looks_draw_the_wire_style_and_the_density_view_only_when_asked() {
    let mut e = headless();
    let plain = json!({ "view": "front", "size": 96, "antialias": false });
    let wired = json!({ "view": "front", "size": 96, "antialias": false, "wireframe": true });
    let dense = json!({ "view": "front", "size": 96, "antialias": false, "density": true });
    let first = [plain.clone(), wired.clone(), dense.clone()].map(|args| snapshot(&mut e, args));
    assert_ne!(first[1], first[0], "the wire shows");
    assert_ne!(first[2], first[0], "the density view shows");
    run(
        &mut e,
        "view.wire",
        json!({ "color": "#ff0000", "opacity": 1 }),
    );
    let red = snapshot(&mut e, wired.clone());
    assert_ne!(red, first[1], "the edges' style reaches a look");
    run(
        &mut e,
        "view.wire",
        json!({ "line": false, "density": true }),
    );
    assert!(
        snapshot(&mut e, wired) == first[0],
        "no edges, no fill: the surface"
    );
    assert!(
        snapshot(&mut e, plain) == first[0],
        "left out: no density view"
    );
    assert!(snapshot(&mut e, dense) == first[2], "asked for: as before");
}
