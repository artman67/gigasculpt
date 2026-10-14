//! Live Boolean's drawing (decision 0057, section 9, as amended for step LB7), measured once:
//! research 26's 1M body (`sphere(289)`, 1,002,252 triangles) minus its 20k cutter
//! (`sphere(41)`, 20,172 triangles) placed by `shapes::cutter()`, as gs-engine's `live_probe`
//! makes them, in the real window host (gs-ui's studio) drawing 1920x1080 frames offscreen on
//! lavapipe, so its times are only to be read against each other. Prints the frame with the
//! operands drawn plain (the live mode off) and with the result drawn in their place (its walls
//! tinted, the cutter as an outline), each frame drawn whole and finished by the GPU; the
//! landing's upload in bytes, and the time from the landed run to the window's copies being
//! current; and the memory the drawing adds. Heavy: run it through `gate/heavy.sh`.
//!
//! ```text
//! gate/heavy.sh cargo run --release --locked -p gs-ui --example live_bench -- [frames]
//! ```

#[expect(dead_code, reason = "shared with gs-geo's tests, which use the rest")]
#[path = "../../gs-geo/tests/boolean/shapes.rs"]
mod shapes;

use std::time::{Duration, Instant};

use gs_engine::{Engine, EngineConfig, Origin};
use gs_protocol::{Outcome, Request, Role, RunParams};
use gs_render::{AdapterKind, RenderSize, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[expect(clippy::disallowed_methods, reason = "a benchmark reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn ms(from: Instant) -> f64 {
    now().duration_since(from).as_secs_f64() * 1e3
}

fn run(engine: &mut Engine, command: &str, args: Value) -> Result<Value> {
    let window = Origin {
        client_id: 0,
        client: "window".into(),
        role: Role::Human,
    };
    let request = Request::Run(RunParams::new(command, args));
    match engine.execute(&window, request).outcome {
        Outcome::Ok(ok) => Ok(ok.result),
        Outcome::Error(e) => Err(format!("{command}: {}", e.message).into()),
    }
}

/// The body and its cutter, marked Difference, as `live_probe` places them.
fn scene(engine: &mut Engine) -> Result<()> {
    let body = json!({ "name": "body", "mesh": shapes::sphere(289) });
    run(engine, "part.create", body)?;
    let cutter = json!({ "name": "cutter", "mesh": shapes::sphere(41) });
    run(engine, "part.create", cutter)?;
    let c = shapes::cutter();
    let frame = json!({ "origin": [0.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] });
    let scale = json!({ "scale": [c.scale, c.scale, c.scale] });
    run(
        engine,
        "transform.apply",
        json!({ "part": "cutter", "frame": frame, "op": scale }),
    )?;
    let to = json!({ "move": c.translation.to_array() });
    run(
        engine,
        "transform.apply",
        json!({ "part": "cutter", "op": to }),
    )?;
    let difference = json!({ "part": "cutter", "operation": "difference" });
    run(engine, "part.set_operation", difference)?;
    Ok(())
}

/// The median milliseconds of `count` frames, each drawn whole (the frame target forgets its
/// last frame, so every part is drawn again) and finished by the GPU before the next.
fn frames(host: &Host, count: usize) -> Result<f64> {
    let mut studio = host.lock();
    let renderer = &studio.render.as_ref().ok_or("no renderer")?.renderer;
    let pixel = renderer.frame_target(RenderSize::square(1))?;
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let t = now();
        if let Some(target) = studio.render.as_ref().and_then(Render::target) {
            target.restart();
        }
        studio.frame()?;
        if let Some(render) = &studio.render {
            // Reading back a pixel waits for the whole queue: the GPU has finished the frame.
            render.renderer.read_frame(&pixel)?;
        }
        times.push(ms(t));
    }
    times.sort_by(f64::total_cmp);
    Ok(times.get(count / 2).copied().unwrap_or(f64::NAN))
}

fn uploaded(host: &Host) -> u64 {
    host.lock().render.as_ref().map_or(0, Render::uploaded)
}

/// This process's resident memory in MB, as Linux reports it.
fn resident() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let line = status.lines().find(|l| l.starts_with("VmRSS:"));
    let kb = line.and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok());
    kb.unwrap_or(f64::NAN) / 1024.0
}

fn main() -> Result<()> {
    let count: usize = std::env::args().nth(1).map_or(Ok(5), |a| a.parse())?;
    let mut engine = Engine::new(EngineConfig::default())?;
    let layout = LayoutFile::in_memory(Layout::default());
    let shell = Shell::new(engine.catalog().clone(), Keymap::default(), layout);
    let render = Render::new(Renderer::new(AdapterKind::Lavapipe)?, 1)?;
    let host = Host::new(Studio::new(shell, Some(render), [1920, 1080], 1.0));
    engine.attach_window(Box::new(host.clone()));
    scene(&mut engine)?;
    let plain = frames(&host, count)?;
    let before = resident();
    run(&mut engine, "view.display", json!({ "live_boolean": true }))?;
    // The run takes about a second (live_probe); waiting longer leaves the landing alone.
    std::thread::sleep(Duration::from_secs(15));
    let t = now();
    let _ = engine.tick();
    let landing = ms(t);
    let sent = uploaded(&host);
    let state = run(&mut engine, "session.state", json!({}))?["boolean.live"].clone();
    let t = now();
    run(
        &mut engine,
        "view.orbit",
        json!({ "view": "main", "right_deg": 10 }),
    )?;
    let (call, turned) = (ms(t), uploaded(&host));
    run(
        &mut engine,
        "view.orbit",
        json!({ "view": "main", "right_deg": -10 }),
    )?;
    let drawn = frames(&host, count)?;
    let after = resident();
    let group = &state["groups"][0];
    eprintln!(
        "group: {} {}, {} faces",
        group["state"], group["solver"], group["faces"]
    );
    eprintln!("frame, median of {count}: plain {plain:.1} ms, result drawn {drawn:.1} ms");
    eprintln!("landing: {sent} bytes uploaded, {landing:.1} ms from the landed run to the copies");
    eprintln!("a camera turn's call: {call:.1} ms, {turned} bytes uploaded");
    eprintln!(
        "resident: {before:.0} MB before, {after:.0} MB after ({:+.0} MB)",
        after - before
    );
    eprintln!("ghost cover: {} bytes", 1920 * 1080 * 4);
    Ok(())
}
