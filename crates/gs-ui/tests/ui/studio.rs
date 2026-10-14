//! The studio on lavapipe, with a real engine: screenshots are byte-identical for the same
//! inputs and show the parts, the panels and the pen's cursor ring; `ui.describe` reports the
//! panels, their commands, the cursor and the camera.

use gs_engine::Engine;
use gs_render::BACKGROUND;
use serde_json::json;

use crate::canvas::{
    H, W, at, base64_decode, decode, middle, rect, run, studio, studio_with, view,
};

/// A sphere, the palette open on "sphere", and the pen hovering 100 pixels right of the 3D
/// view's middle.
fn scene(engine: &mut Engine) -> Vec<u8> {
    run(
        engine,
        false,
        "primitive.sphere",
        json!({ "subdivisions": 8 }),
    );
    run(engine, true, "ui.palette", json!({ "query": "sphere" }));
    let [x, y] = middle(engine);
    let pen = json!({ "samples": [{ "x": x + 100, "y": y }] });
    run(engine, false, "pen.input", pen);
    let shot = run(engine, false, "ui.screenshot", json!({}));
    base64_decode(shot["png_base64"].as_str().unwrap_or_default())
}

#[test]
fn screenshots_are_byte_identical_and_show_what_is_there() {
    let (mut a, _ha) = studio(1.0);
    let (mut b, _hb) = studio(1.0);
    let first = scene(&mut a);
    assert_eq!(
        first,
        scene(&mut b),
        "a second window, same inputs: same bytes"
    );
    // The palette closed and the status bar saying the agent ran ui.screenshot: a new frame;
    // from here on, every shot is equal.
    run(&mut a, true, "ui.palette", json!({ "open": false }));
    let second = run(&mut a, false, "ui.screenshot", json!({}));
    let third = run(&mut a, false, "ui.screenshot", json!({}));
    assert_eq!(second, third, "a settled window draws the same frame again");
    assert_ne!(
        base64_decode(second["png_base64"].as_str().unwrap_or_default()),
        first
    );
    assert_eq!(
        (third["width"].clone(), third["height"].clone()),
        (json!(W), json!(H))
    );
    // With the palette closed (it opens over the model).
    let image = decode(&base64_decode(
        second["png_base64"].as_str().unwrap_or_default(),
    ));
    assert_eq!(image.0, W);
    let tray = at(&image, 20, 300);
    assert_ne!(tray, BACKGROUND, "the brush tray covers the left");
    let [x, y] = middle(&mut a);
    let sphere = at(&image, x, y);
    let light = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    assert!(
        light(sphere) > light(BACKGROUND) + 60,
        "the lit MatCap sphere fills the view's middle: {sphere:?}"
    );
    let ring = at(&image, x + 110, y);
    assert!(
        ring[2] > ring[0] && ring[2] > 180,
        "the hover ring is cool blue, 10 points right of the pen: {ring:?}"
    );
    let [x0, _, _, y1] = rect(&view(&mut a)).map(|v| v as u32);
    let corner = at(&image, x0 + 5, y1 - 5);
    assert_eq!(corner, BACKGROUND, "empty view corner");
}

#[test]
fn describe_reports_panels_commands_cursor_and_camera() {
    // 800 by 600 points: a 3D view wide and tall enough for the corner figure beside both trays
    // and under the shelf, whose rows wrap.
    let (mut engine, _host) = studio_with([1000, 750], 1.25, 1.0);
    run(&mut engine, false, "primitive.cube", json!({}));
    let pen = json!({ "samples": [{ "x": 500.0, "y": 300.0, "contact": true, "pressure": 0.25 }] });
    run(&mut engine, false, "pen.input", pen);
    let d = run(&mut engine, false, "ui.describe", json!({}));
    assert_eq!(d["window"], json!(true));
    assert_eq!(d["size"], json!([1000, 750]));
    let ids: Vec<&str> = d["panels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str())
        .collect();
    // The status line, the brush shelf and the two trays, the header bar, then the view, its
    // corner figure and the Zoom and Pan buttons under it.
    assert_eq!(
        ids,
        [
            "status", "shelf", "brush", "tray", "header", "viewport", "figure", "pads"
        ]
    );
    let toggle = &d["panels"][0]["widgets"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["role"] == "toggle").cloned())
        .unwrap_or_default();
    assert_eq!(toggle["label"], "Pause agents");
    assert_eq!(
        toggle["command"],
        json!({ "command": "session.control", "args": { "paused": true } })
    );
    assert_eq!(toggle["enabled"], json!(true));
    assert_eq!(d["cursor"]["pressure"], json!(0.25));
    assert_eq!(d["viewport"]["parts"], json!(["cube"]));
    assert_eq!(d["viewport"]["shading"], "matcap gray");
    assert!(
        d["viewport"]["camera"]["distance"]
            .as_f64()
            .unwrap_or_default()
            > 0.0,
        "fitted"
    );
    assert_eq!(d["pixel_density"], json!(1.25));
}
