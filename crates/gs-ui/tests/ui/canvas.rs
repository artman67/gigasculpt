//! A full studio drawing on lavapipe, attached to a real engine, and helpers to drive it with
//! the mouse and read back screenshots and the description.

use gs_engine::{Engine, EngineConfig};
use gs_platform::{Button, Input, Modifiers as Held};
use gs_render::{AdapterKind, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

use crate::rig::{agent, window};

pub const W: u32 = 800;
pub const H: u32 = 500;

/// An engine whose window is a studio of `size` pixels at `density`, with the interface scaled
/// by `scale`, drawing single-sampled on lavapipe (byte-identical frames).
pub fn studio_with(size: [u32; 2], density: f32, scale: f32) -> (Engine, Host) {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let layout = Layout {
        ui_scale: scale,
        ..Layout::default()
    };
    let shell = Shell::new(
        engine.catalog().clone(),
        Keymap::default(),
        LayoutFile::in_memory(layout),
    );
    let renderer = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
    let render = Render::new(renderer, 1).unwrap_or_else(|e| panic!("{e}"));
    let host = Host::new(Studio::new(shell, Some(render), size, density));
    engine.attach_window(Box::new(host.clone()));
    (engine, host)
}

/// An 800 x 500 studio at `density`.
pub fn studio(density: f32) -> (Engine, Host) {
    studio_with([W, H], density, 1.0)
}

/// Runs a command that must succeed, as the window or as an agent.
pub fn run(engine: &mut Engine, as_window: bool, command: &str, args: Value) -> Value {
    let origin = if as_window { window() } else { agent() };
    let request = gs_protocol::Request::Run(gs_protocol::RunParams::new(command, args));
    match engine.execute(&origin, request).outcome {
        gs_protocol::Outcome::Ok(ok) => ok.result,
        gs_protocol::Outcome::Error(e) => panic!("{command}: {}", e.message),
    }
}

/// A decoded image: width and RGBA rows.
pub type Shot = (u32, Vec<u8>);

pub fn decode(png: &[u8]) -> Shot {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap_or_else(|e| panic!("{e}"));
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("{e}"));
    buf.truncate(info.buffer_size());
    (info.width, buf)
}

pub fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap_or_else(|e| panic!("{e}"))
}

/// `ui.screenshot`, decoded.
pub fn shot(engine: &mut Engine) -> Shot {
    let out = run(engine, false, "ui.screenshot", json!({}));
    decode(&base64_decode(
        out["png_base64"].as_str().unwrap_or_default(),
    ))
}

pub fn at(image: &Shot, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.0 + x) * 4) as usize;
    [image.1[i], image.1[i + 1], image.1[i + 2], image.1[i + 3]]
}

/// The viewport part of `ui.describe`.
pub fn view(engine: &mut Engine) -> Value {
    run(engine, false, "ui.describe", json!({}))["viewport"].clone()
}

pub fn num(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or_default()
}

pub fn target(v: &Value, axis: usize) -> f64 {
    v["camera"]["target"][axis].as_f64().unwrap_or_default()
}

/// The 3D view's middle, in pixels.
pub fn middle(engine: &mut Engine) -> [u32; 2] {
    let [x0, y0, x1, y1] = rect(&view(engine));
    [f64::midpoint(x0, x1), f64::midpoint(y0, y1)].map(|v| v as u32)
}

/// The view's rectangle in pixels: min x, min y, max x, max y.
pub fn rect(v: &Value) -> [f64; 4] {
    let r = |i: usize| v["rect"][i].as_f64().unwrap_or_default();
    [r(0), r(1), r(2), r(3)]
}

/// Draws the frames the input so far asks for, then runs what the interface asked for as the
/// window, as the window's loop does.
pub fn pump(engine: &mut Engine, host: &Host) {
    run(engine, false, "ui.describe", json!({}));
    let steps = host.lock().shell.take_commands();
    for step in steps {
        run(engine, true, &step.command, step.args);
    }
}

/// The window's camera, `main`, exactly as the engine holds it.
pub fn camera(engine: &mut Engine) -> gs_render::Camera {
    let v = run(engine, false, "view.get", json!({ "view": "main" }));
    let exact: gs_engine::ExactCamera =
        serde_json::from_value(v["camera"].clone()).unwrap_or_else(|e| panic!("{e}"));
    exact.camera("").unwrap_or_else(|e| panic!("{}", e.message))
}

/// Drags the mouse with `button` from `from` to `to` in three frames, holding `held`, running
/// what the interface asks for after each frame.
pub fn drag(
    engine: &mut Engine,
    host: &Host,
    button: Button,
    held: Held,
    from: [f64; 2],
    to: [f64; 2],
) {
    let mid = [f64::midpoint(from[0], to[0]), f64::midpoint(from[1], to[1])];
    let press = |down, [x, y]: [f64; 2]| Input::MouseButton { button, down, x, y };
    let frames = [
        vec![
            Input::MouseMove {
                x: from[0],
                y: from[1],
            },
            press(true, from),
        ],
        vec![Input::MouseMove {
            x: mid[0],
            y: mid[1],
        }],
        vec![Input::MouseMove { x: to[0], y: to[1] }],
        vec![press(false, to)],
    ];
    for inputs in frames {
        host.lock().input(&inputs, held);
        pump(engine, host);
    }
    host.lock().input(&[], Held::default());
    pump(engine, host);
}
