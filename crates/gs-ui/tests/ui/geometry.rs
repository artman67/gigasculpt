//! Where things land in pixels, and what the studio keeps between frames: the display density
//! times the interface scale, the model centered and framed in its view, resizing, typing,
//! which changes ask for a new frame, and following the engine's parts.

use gs_engine::PenReads;
use gs_platform::{Input, Modifiers as Held};
use gs_protocol::PenSample;
use gs_render::BACKGROUND;
use serde_json::json;

use crate::canvas::{H, Shot, W, at, num, rect, run, shot, studio, studio_with, target, view};

/// The first and last pixel that is not background along a row (`horizontal`) or a column,
/// between `from` and `to`.
fn extent(image: &Shot, horizontal: bool, line: u32, from: u32, to: u32) -> Option<(u32, u32)> {
    let hit = |i: u32| {
        let (x, y) = if horizontal { (i, line) } else { (line, i) };
        at(image, x, y) != BACKGROUND
    };
    let first = (from..to).find(|&i| hit(i))?;
    let last = (from..to).rev().find(|&i| hit(i))?;
    Some((first, last))
}

#[test]
fn density_times_interface_scale_gives_pixels() {
    let (mut engine, host) = studio_with([800, 600], 2.0, 1.5);
    let d = run(&mut engine, false, "ui.describe", json!({}));
    let [x0, y0, x1, y1] = rect(&d["viewport"]);
    // The brush shelf runs along the top, as tall as its file says, and the header bar sits
    // under it at the top of the view's room, so the view's picture starts under the header.
    let shelf = (host.lock().shell.directions.list[0].panels[0]).size;
    let header = (d["panels"].as_array().into_iter().flatten())
        .find(|p| p["id"] == "header")
        .map_or_else(|| panic!("a header: {d}"), rect);
    assert!(
        (header[1] - f64::from(shelf) * 3.0).abs() < 2.0 && (y0 - header[3]).abs() < 2.0,
        "under the shelf and the header: {y0} vs {header:?}, {shelf} points"
    );
    assert!(
        0.0 < x0 && x1 < 800.0 && x1 - x0 >= 400.0 - 2.0,
        "between the trays, with at least half the width (egui keeps 1/32 point): {x0}, {x1}"
    );
    let status = &d["panels"][0]["rect"];
    let edge = |i: usize| status[i].as_f64().unwrap_or_default();
    assert!(
        y1 <= edge(1) && 600.0 - y1 >= 32.0 * 3.0,
        "above the status bar, a pen target at least 32 points tall at 3 pixels per point: \
         {y1}, {status}"
    );
    assert!(
        (12.0..45.0).contains(&edge(0)),
        "the status bar starts at the left margin: {status}"
    );
    assert!(
        (760.0..=800.0).contains(&edge(2)),
        "and spans the width: {status}"
    );
    assert!(
        (570.0..=600.0).contains(&edge(3)),
        "down to the bottom margin: {status}"
    );
    let widget = &d["panels"][0]["widgets"][0]["rect"];
    assert!(
        widget[1].as_f64().unwrap_or_default() > y1,
        "its widgets sit below the view: {widget}"
    );
}

#[test]
fn the_model_is_centered_in_its_view() {
    // 800 by 500 points: at 400 by 250 the header's rows (with Symmetry at its pen end) take
    // the whole view.
    let (mut engine, _host) = studio_with([2 * W, 2 * H], 2.0, 1.0);
    run(
        &mut engine,
        false,
        "primitive.sphere",
        json!({ "subdivisions": 16 }),
    );
    // Two pixels in from each side, past the lines between the view and the panels.
    let [x0, y0, x1, y1] = rect(&view(&mut engine)).map(|v| v as u32);
    let [x0, y0, x1, y1] = [x0 + 2, y0 + 2, x1 - 2, y1 - 2];
    let image = shot(&mut engine);
    let (mx, my) = (u32::midpoint(x0, x1), u32::midpoint(y0, y1));
    let (left, right) = extent(&image, true, my, x0, x1).unwrap_or_default();
    let (top, bottom) = extent(&image, false, mx, y0, y1).unwrap_or_default();
    let (cx, cy) = (u32::midpoint(left, right), u32::midpoint(top, bottom));
    assert!(
        cx.abs_diff(mx) <= 2 && cy.abs_diff(my) <= 2,
        "centered: ({cx}, {cy}) vs ({mx}, {my})"
    );
    assert!(
        left > x0 && right + 1 < x1,
        "inside the view: {left}..{right} in {x0}..{x1}"
    );
}

#[test]
fn a_tall_view_frames_the_model_by_its_width() {
    let (mut engine, _host) = studio_with([400, 800], 1.0, 1.0);
    run(
        &mut engine,
        false,
        "primitive.sphere",
        json!({ "subdivisions": 16 }),
    );
    let [x0, y0, x1, y1] = rect(&view(&mut engine)).map(|v| v as u32);
    let image = shot(&mut engine);
    let (left, right) = extent(&image, true, u32::midpoint(y0, y1), x0, x1).unwrap_or_default();
    let (width, model) = (x1 - x0, right - left);
    assert!(
        model * 2 > width && model + 8 < width,
        "fits the width: {model} of {width}"
    );
}

#[test]
fn resizing_changes_frames_and_the_view() {
    let (mut engine, host) = studio(1.0);
    shot(&mut engine);
    let before = rect(&view(&mut engine));
    host.lock().resize([640, 400], 1.0);
    let image = shot(&mut engine);
    assert_eq!(
        (image.0, image.1.len()),
        (640, 640 * 400 * 4),
        "the new size"
    );
    let d = run(&mut engine, false, "ui.describe", json!({}));
    assert_eq!(d["size"], json!([640, 400]));
    let after = rect(&d["viewport"]);
    assert!(
        after[2] < before[2] && after[2] > 320.0,
        "the view follows: {after:?}"
    );
}

#[test]
fn typed_text_reaches_the_focused_palette() {
    let (mut engine, host) = studio(1.0);
    run(&mut engine, true, "ui.palette", json!({}));
    run(&mut engine, false, "ui.describe", json!({}));
    host.lock()
        .input(&[Input::Text("cube".into())], Held::default());
    run(&mut engine, false, "ui.describe", json!({}));
    assert_eq!(
        host.lock().shell.palette.query,
        "cube",
        "the search field took the text"
    );
}

#[test]
fn only_changes_ask_for_a_new_frame() {
    let (mut engine, host) = studio(1.0);
    assert!(host.lock().dirty, "a new window needs its first frame");
    let settle = |host: &gs_ui::Host| {
        let frame = host.lock().frame();
        assert!(frame.is_ok(), "{frame:?}");
    };
    settle(&host);
    assert!(!host.lock().dirty, "drawn");
    host.lock().input(&[], Held::default());
    assert!(!host.lock().dirty, "no input, nothing new");
    host.lock()
        .input(&[Input::MouseMove { x: 1.0, y: 1.0 }], Held::default());
    assert!(host.lock().dirty, "the mouse moved");
    settle(&host);
    // Nothing to read: no part, no surface.
    let reads = PenReads {
        pose: &|_| None,
        under: &|_, _| None,
        masked: &|| false,
        dabs: &|_, _, _| Vec::new(),
    };
    host.lock().pen(&[], &reads);
    assert!(!host.lock().dirty, "no samples, nothing new");
    host.lock().pen(&[PenSample::hover(0, 5.0, 5.0)], &reads);
    assert!(host.lock().dirty, "the pen moved");
    settle(&host);
    run(&mut engine, false, "session.info", json!({}));
    assert!(host.lock().dirty, "the engine ran a call");
}

#[test]
fn the_view_follows_the_engines_parts_and_frames_new_ones() {
    let (mut engine, _host) = studio(1.0);
    run(&mut engine, false, "primitive.sphere", json!({}));
    let v0 = view(&mut engine);
    let [x0, y0, x1, y1] = rect(&v0).map(|v| v as u32);
    let middle = |engine: &mut _| at(&shot(engine), u32::midpoint(x0, x1), u32::midpoint(y0, y1));
    assert_ne!(middle(&mut engine), BACKGROUND, "the sphere");
    run(
        &mut engine,
        false,
        "primitive.cube",
        json!({ "center": [0.0, 30.0, 0.0] }),
    );
    let v1 = view(&mut engine);
    assert_eq!(
        v1["parts"],
        json!(["sphere", "cube"]),
        "both drawn, once each, in list order"
    );
    assert_eq!(
        v1["camera"], v0["camera"],
        "a new part does not move the camera"
    );
    run(
        &mut engine,
        false,
        "mesh.translate",
        json!({ "part": "sphere", "direction": [1.0, 0.0, 0.0], "distance": 40.0 }),
    );
    assert_eq!(
        middle(&mut engine),
        BACKGROUND,
        "the moved sphere left the middle"
    );
    for _ in 0..3 {
        run(&mut engine, false, "history.undo", json!({}));
    }
    let empty = view(&mut engine);
    assert_eq!(empty["parts"], json!([]), "everything undone");
    run(
        &mut engine,
        false,
        "primitive.cube",
        json!({ "center": [9.0, 0.0, 0.0] }),
    );
    let refit = view(&mut engine);
    assert!(
        (target(&refit, 0) - 9.0).abs() < 1e-3,
        "an empty view frames the next part: {refit}"
    );
    assert!(num(&refit["camera"], "distance") > 0.0, "fitted");
}

#[test]
fn the_opened_palettes_search_field_takes_the_keyboard() {
    let (mut engine, _host) = studio(1.0);
    run(&mut engine, true, "ui.palette", json!({}));
    let d = run(&mut engine, false, "ui.describe", json!({}));
    let palette = d["panels"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["id"] == "palette").cloned());
    let field = palette.unwrap_or_default()["widgets"][0]["rect"].clone();
    let f = |i: usize| field[i].as_f64().unwrap_or_default();
    let image = shot(&mut engine);
    let x = f64::midpoint(f(0), f(2)) as u32;
    let top = f(1).floor() as u32;
    let bluest = (top.saturating_sub(1)..=top + 1)
        .map(|y| at(&image, x, y)[2])
        .max();
    assert!(
        bluest > Some(200),
        "the search field shows the focus border: {bluest:?}"
    );
}
