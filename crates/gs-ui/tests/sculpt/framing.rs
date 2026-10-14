//! A pixel of a snapshot and a screen sample at that pixel mean the same point: along rows of
//! a snapshot, `pick.ray` through the pixels just inside the model's outline hits it and
//! through the pixels just outside misses, on a scene the renderer cuts into several chunks;
//! for a named view and for exact cameras of every kind (rolled, orthographic, a long lens on a
//! horizontal fit, a shifted lens); and a stroke aimed at a pixel lands where the pick says.

use gs_core::{Quat, Vec3};
use gs_engine::{Engine, EngineConfig, Origin};
use gs_protocol::{Outcome, Request, Role, RunParams};
use gs_ui::Offscreen;
use serde_json::{Value, json};

const BACKGROUND: [u8; 4] = [40, 42, 46, 255];
const SIZE: u32 = 160;

fn run(engine: &mut Engine, command: &str, args: Value) -> Value {
    let agent = Origin {
        client_id: 9,
        client: "agent".into(),
        role: Role::Agent,
    };
    let request = Request::Run(RunParams::new(command, args));
    match engine.execute(&agent, request).outcome {
        Outcome::Ok(ok) => ok.result,
        Outcome::Error(e) => panic!("{command}: {}", e.message),
    }
}

/// The snapshot's RGBA rows through `view`, one sample per pixel, so a pixel is covered
/// exactly when the ray through its center hits.
fn pixels(engine: &mut Engine, view: &Value) -> Vec<u8> {
    use base64::Engine as _;
    let shot = run(
        engine,
        "view.snapshot",
        json!({ "view": view, "size": SIZE, "antialias": false }),
    );
    let text = shot["png_base64"].as_str().unwrap_or_default();
    let bytes =
        (base64::engine::general_purpose::STANDARD.decode(text)).unwrap_or_else(|e| panic!("{e}"));
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("{e}"));
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("{e}"));
    buf.truncate(info.buffer_size());
    buf
}

fn pick(engine: &mut Engine, view: &Value, x: f32, y: f32) -> Value {
    let args = json!({ "camera": view, "viewport": [SIZE, SIZE], "at": [x, y] });
    run(engine, "pick.ray", args)
}

fn hits(engine: &mut Engine, view: &Value, x: u32, y: u32) -> bool {
    let at = pick(engine, view, x as f32 + 0.5, y as f32 + 0.5);
    at.get("hit").is_some()
}

/// Along rows of a snapshot through `view`, the pixels just inside the outline hit and those
/// just outside miss. Returns how many rows cross the model.
fn outline_agrees(engine: &mut Engine, view: &Value) -> u32 {
    let rgba = pixels(engine, view);
    let covered = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        rgba[i..i + 4] != BACKGROUND
    };
    let mut checked = 0;
    for y in 4..SIZE - 4 {
        let Some(edge) = (2..SIZE - 2).find(|&x| covered(x, y)) else {
            continue;
        };
        let inside = covered(edge + 1, y);
        let hit = hits(engine, view, edge + 1, y);
        assert_eq!(
            hit,
            inside,
            "{view}: row {y}: pick and picture agree at {}",
            edge + 1
        );
        assert!(
            !hits(engine, view, edge - 2, y),
            "{view}: row {y}: outside at {}",
            edge - 2
        );
        checked += 1;
    }
    checked
}

/// A session rendering on lavapipe, with the scene of [`snapshot_pixels_and_picks_agree`].
fn scene() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let renderer = Offscreen::host(Some("lavapipe")).unwrap_or_else(|e| panic!("{e}"));
    engine.attach_renderer(renderer);
    // Two spheres far back to the left and right and a small cube near the camera: framing
    // the box around all three, and framing the renderer's chunks one by one, give different
    // pictures. Each sphere fills exactly three chunks of 16,384 triangles and the parts are
    // drawn in name order, so no chunk holds faces of two parts.
    for (name, x) in [("a", -3.0), ("b", 3.0)] {
        let center = [x, 0.0, -2.0];
        let args = json!({ "name": name, "subdivisions": 64, "radius": 0.5, "center": center });
        run(&mut engine, "primitive.sphere", args);
    }
    let cube = json!({ "name": "c", "size": 0.5, "center": [0.0, 0.3, 2.0] });
    run(&mut engine, "primitive.cube", cube);
    engine
}

#[test]
fn snapshot_pixels_and_picks_agree() {
    let mut engine = scene();
    let checked = outline_agrees(&mut engine, &json!("front"));
    assert!(checked >= 12, "only {checked} rows cross the model");
}

/// Exact cameras of every kind looking at the scene: rolled, orthographic, a long lens on a
/// horizontal fit, a shifted lens.
fn cameras() -> Vec<Value> {
    let lens = |focal: f32, fit: &str, shift: [f32; 2]| json!({ "focal_mm": focal, "fit": fit, "shift": shift });
    // Turned `yaw` degrees about the vertical axis, rolled `roll` about the view direction.
    let turn = |yaw: f32, roll: f32| {
        let about =
            |axis: [f32; 3], deg: f32| Quat::about(Vec3::from_array(axis), deg.to_radians());
        (about([0.0, 1.0, 0.0], yaw) * about([0.0, 0.0, 1.0], roll)).to_array()
    };
    let at = |q: [f32; 4], d: f32, projection: &str, lens: Value| {
        json!({
            "orientation": q, "target": [0.0, 0.0, 0.0], "distance": d,
            "projection": projection, "lens": lens,
        })
    };
    vec![
        at(
            turn(20.0, 30.0),
            14.0,
            "perspective",
            lens(29.0, "vertical", [0.0, 0.0]),
        ),
        at(
            turn(-15.0, 0.0),
            14.0,
            "orthographic",
            lens(29.0, "vertical", [0.0, 0.0]),
        ),
        at(
            turn(10.0, -20.0),
            30.0,
            "perspective",
            lens(85.0, "horizontal", [0.0, 0.0]),
        ),
        at(
            turn(0.0, 10.0),
            14.0,
            "perspective",
            lens(35.0, "vertical", [0.12, -0.08]),
        ),
    ]
}

#[test]
fn picks_agree_with_snapshots_through_every_kind_of_camera() {
    let mut engine = scene();
    for camera in cameras() {
        let checked = outline_agrees(&mut engine, &camera);
        assert!(
            checked >= 12,
            "{camera}: only {checked} rows cross the model"
        );
    }
}

#[test]
fn a_stroke_aimed_at_a_pixel_lands_where_the_pick_says() {
    let mut engine = scene();
    for camera in cameras() {
        let rgba = pixels(&mut engine, &camera);
        // A covered pixel with covered neighbours on one of the spheres (the cube has only
        // corners to move), near the middle of the image.
        let covered = |x: u32, y: u32| rgba[((y * SIZE + x) * 4) as usize..][..4] != BACKGROUND;
        let solid = |x: u32, y: u32| (0..9).all(|k| covered(x + k % 3 - 1, y + k / 3 - 1));
        let middle = (0..SIZE / 2 - 2).flat_map(|d| [SIZE / 2 + d, SIZE / 2 - d]);
        let candidates: Vec<(u32, u32)> = (middle.clone())
            .flat_map(|y| middle.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| solid(x, y))
            .collect();
        let (px, py, hit) = (candidates.iter())
            .map(|&(x, y)| (x as f32 + 0.5, y as f32 + 0.5))
            .map(|(px, py)| (px, py, pick(&mut engine, &camera, px, py)["hit"].clone()))
            .find(|(_, _, hit)| hit["part"] != "c")
            .unwrap_or_else(|| panic!("{camera}: no sphere in the picture"));
        let part = hit["part"].as_str().unwrap_or_default().to_string();
        let near = json!({ "part": part, "near": hit["position"], "k": 300 });
        let before = run(&mut engine, "mesh.sample", near.clone());
        let stroke = json!({
            "part": part, "brush": "Draw", "camera": camera, "viewport": [SIZE, SIZE],
            "size_px": 3, "strength": 1, "samples": [{ "position": [px, py] }],
        });
        let radius = run(&mut engine, "brush.stroke", stroke)["radius"]
            .as_f64()
            .unwrap_or_default();
        let after = run(&mut engine, "mesh.sample", near);
        let at: Vec<f64> = (hit["position"].as_array().into_iter().flatten())
            .filter_map(Value::as_f64)
            .collect();
        let moved = moved_near(&before, &after, &at);
        assert!(moved.0 > 0, "{camera}: the stroke moved something");
        assert!(
            moved.1 <= radius * 1.02,
            "{camera}: it moved points {} from the pick, beyond its radius {radius}",
            moved.1
        );
        run(&mut engine, "history.undo", json!({}));
    }
}

/// How many of the points `mesh.sample` read near the pick moved, and how far the farthest
/// of them lies from `at`.
fn moved_near(before: &Value, after: &Value, at: &[f64]) -> (usize, f64) {
    let points = |v: &Value| -> Vec<Vec<f64>> {
        (v["vertices"].as_array().into_iter().flatten())
            .map(|p| {
                (p["position"].as_array().into_iter().flatten())
                    .filter_map(Value::as_f64)
                    .collect()
            })
            .collect()
    };
    let (a, b) = (points(before), points(after));
    let moved: Vec<&Vec<f64>> = (a.iter().zip(&b))
        .filter(|(p, q)| p != q)
        .map(|(p, _)| p)
        .collect();
    let far = (moved.iter())
        .map(|p| {
            p.iter()
                .zip(at)
                .map(|(x, y)| (x - y) * (x - y))
                .sum::<f64>()
                .sqrt()
        })
        .fold(0.0, f64::max);
    (moved.len(), far)
}
