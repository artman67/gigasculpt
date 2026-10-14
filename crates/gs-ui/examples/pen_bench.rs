//! Pen-to-screen latency and stroke-step time for sculpting in the window, on a big sphere.
//! Heavy: run it through `gate/heavy.sh`, never in the default test run.
//!
//! ```text
//! gate/heavy.sh cargo run -p gs-ui --release --example pen_bench -- [adapter] [n] [frames] [sizes]
//! ```
//!
//! `adapter` is `nvidia`, `radv` or `lavapipe` (default `nvidia`), `n` the cells per cube
//! edge of a spherified cube (default 289: 6n² quads, about 1M triangles; 913 is about 10M),
//! `frames` the pen samples per stroke (default 120), `sizes` the brush sizes in pixels
//! (default `48,240`). The engine runs with the real window host
//! (gs-ui's studio) drawing 1920x1080 frames offscreen, and the pen goes through `pen.input`
//! one sample per frame, exactly as the window sends it. For a small and a large brush, each
//! frame times: the engine's part (aim, stroke step, normals, dirty-node upload), building and
//! submitting the frame, and the GPU finishing it. Then the commit at pen-up and an undo.
//! Prints the 50th and 95th percentiles and the worst.

use std::collections::BTreeMap;
use std::time::Instant;

use gs_engine::{Engine, EngineConfig, Origin};
use gs_protocol::{Outcome, PenSample, PlainMesh, Request, Role, RunParams, Success};
use gs_render::{AdapterKind, RenderSize, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[expect(clippy::disallowed_methods, reason = "a benchmark reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn ms(from: Instant, to: Instant) -> f64 {
    to.duration_since(from).as_secs_f64() * 1e3
}

/// A spherified cube of 6n² quads, its sides welded only by position (not shared), which
/// does not matter for timing.
fn sphere(n: u32) -> PlainMesh {
    const SIDES: [[[f32; 3]; 3]; 6] = [
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
    ];
    let row = n + 1;
    let mut positions = Vec::with_capacity(6 * (row * row) as usize);
    let mut faces = Vec::with_capacity(6 * (n * n) as usize);
    for (s, [normal, u, v]) in SIDES.iter().enumerate() {
        for j in 0..row {
            for i in 0..row {
                let (a, b) = (
                    2.0 * i as f32 / n as f32 - 1.0,
                    2.0 * j as f32 / n as f32 - 1.0,
                );
                let c: [f32; 3] = std::array::from_fn(|k| normal[k] + a * u[k] + b * v[k]);
                let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                positions.push(c.map(|x| x / len));
            }
        }
        let base = s as u32 * row * row;
        for j in 0..n {
            for i in 0..n {
                let id = |i: u32, j: u32| base + j * row + i;
                faces.push([id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)]);
            }
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

fn human() -> Origin {
    Origin {
        client_id: 0,
        client: "window".into(),
        role: Role::Human,
    }
}

fn call(engine: &mut Engine, command: &str, args: Value) -> Result<Success> {
    let request = Request::Run(RunParams::new(command, args));
    match engine.execute(&human(), request).outcome {
        Outcome::Ok(ok) => Ok(ok),
        Outcome::Error(e) => Err(format!("{command}: {}", e.message).into()),
    }
}

fn run(engine: &mut Engine, command: &str, args: Value) -> Result<Value> {
    Ok(call(engine, command, args)?.result)
}

/// The world radius of a brush `size_px` wide at the middle of the view, and how many
/// vertices one dab of it moves there, from a one-dab `brush.stroke` (undone after).
fn footprint(engine: &mut Engine, size_px: f32) -> Result<(f64, u64)> {
    let d = run(engine, "ui.describe", json!({}))?;
    let v = &d["viewport"];
    let rect: [f64; 4] = std::array::from_fn(|i| v["rect"][i].as_f64().unwrap_or_default());
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let camera = v["camera"].clone();
    let size = [w.round() as u32, h.round() as u32];
    let args = json!({ "camera": camera, "viewport": size, "at": [w / 2.0, h / 2.0] });
    let hit = run(engine, "pick.ray", args)?["hit"].clone();
    let t = hit["distance"]
        .as_f64()
        .ok_or("the middle of the view misses the sphere")?;
    let radius = f64::from(size_px) * 2.0 * t * libm::tan(std::f64::consts::FRAC_PI_8) / h;
    let dab =
        json!({ "brush": "Draw", "size": radius, "samples": [{ "position": hit["position"] }] });
    let done = call(engine, "brush.stroke", dab)?;
    run(engine, "history.undo", json!({}))?;
    Ok((radius, done.effect.map_or(0, |e| e.verts_touched)))
}

/// 50th and 95th percentiles and the worst, as text.
fn summary(name: &str, mut v: Vec<f64>) -> String {
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    let worst = v.last().copied().unwrap_or_default();
    format!(
        "{name:<34} p50 {:7.2} ms  p95 {:7.2} ms  max {worst:7.2} ms",
        at(0.5),
        at(0.95)
    )
}

/// One stroke across the view with a brush `size_px` wide; prints its timings.
fn stroke(engine: &mut Engine, host: &Host, frames: u64, size_px: f32) -> Result<()> {
    run(
        engine,
        "brush.set",
        json!({ "brush": "Draw", "size_px": size_px }),
    )?;
    let view = run(engine, "ui.describe", json!({}))?["viewport"]["rect"].clone();
    let rect: [f64; 4] = std::array::from_fn(|i| view[i].as_f64().unwrap_or_default());
    let (cx, cy) = (
        f64::midpoint(rect[0], rect[2]),
        f64::midpoint(rect[1], rect[3]),
    );
    let half = (rect[2] - rect[0]) * 0.15;
    let (mut engine_ms, mut frame_ms, mut gpu_ms, mut total) = (vec![], vec![], vec![], vec![]);
    let pixel = (host.lock().render.as_ref().ok_or("no renderer")?)
        .renderer
        .frame_target(RenderSize::square(1))?;
    for k in 0..=frames {
        let f = k as f64 / frames as f64;
        let s = PenSample {
            pressure: 0.6,
            contact: true,
            ..PenSample::hover(k * 4_000_000, cx - half + 2.0 * half * f, cy + 20.0 * f)
        };
        let t0 = now();
        run(engine, "pen.input", json!({ "samples": [s] }))?;
        let t1 = now();
        let mut studio = host.lock();
        studio.frame()?;
        let t2 = now();
        // Reading back a pixel waits for the whole queue: the GPU has finished the frame.
        if let Some(render) = &studio.render {
            render.renderer.read_frame(&pixel)?;
        }
        let t3 = now();
        drop(studio);
        engine_ms.push(ms(t0, t1));
        frame_ms.push(ms(t1, t2));
        gpu_ms.push(ms(t2, t3));
        total.push(ms(t0, t3));
    }
    let lift = PenSample::hover(frames * 4_000_000 + 4_000_000, cx + half, cy + 20.0);
    let t = now();
    run(engine, "pen.input", json!({ "samples": [lift] }))?;
    let commit = ms(t, now());
    let t = now();
    run(engine, "history.undo", json!({}))?;
    let undo = ms(t, now());
    let (radius, verts) = footprint(engine, size_px)?;
    eprintln!(
        "brush {size_px} px (radius {radius:.3}, one dab moves {verts} vertices), {} frames",
        frames + 1
    );
    eprintln!(
        "  {}",
        summary("engine (aim, step, normals, upload)", engine_ms)
    );
    eprintln!("  {}", summary("frame build and submit", frame_ms));
    eprintln!("  {}", summary("GPU finishing the frame", gpu_ms));
    eprintln!("  {}", summary("pen sample to frame done", total));
    eprintln!("  commit at pen-up {commit:.2} ms, undo {undo:.2} ms");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let adapter = args.first().map_or("nvidia", String::as_str);
    let n: u32 = args.get(1).map_or(Ok(289), |a| a.parse())?;
    let frames: u64 = args.get(2).map_or(Ok(120), |a| a.parse())?;
    let sizes: Vec<f32> = (args.get(3).map_or("48,240", String::as_str).split(','))
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let kind = AdapterKind::from_name(adapter).ok_or("unknown adapter")?;
    let mut engine = Engine::new(EngineConfig::default())?;
    let layout = LayoutFile::in_memory(Layout::default());
    let shell = Shell::new(engine.catalog().clone(), Keymap::default(), layout);
    let render = Render::new(Renderer::new(kind)?, 1)?;
    let host = Host::new(Studio::new(shell, Some(render), [1920, 1080], 1.0));
    engine.attach_window(Box::new(host.clone()));
    let mesh = sphere(n);
    let t = now();
    run(
        &mut engine,
        "part.create",
        json!({ "name": "sphere", "mesh": mesh }),
    )?;
    let triangles = 2 * mesh.faces.len();
    eprintln!(
        "{triangles} triangles, {} vertices on {adapter}: made and uploaded in {:.0} ms",
        mesh.positions.len(),
        ms(t, now())
    );
    for size in sizes {
        stroke(&mut engine, &host, frames, size)?;
    }
    Ok(())
}
