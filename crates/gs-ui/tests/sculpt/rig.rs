//! A real engine in a real window host (gs-ui's studio drawing on lavapipe), driven the way the
//! window drives it: pen samples one `pen.input` call per frame and keys through the studio's
//! input, every call as the human at the window. Tests find what they press with
//! [`Window::rect`], never by counting pixels from the window's edge.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::{Engine, EngineConfig, Origin};
use gs_platform::{Input, Modifiers};
use gs_protocol::{CmdResponse, Outcome, PenSample, Request, Role, RunParams, ScriptLine};
use gs_render::{AdapterKind, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Render, Shell, Studio};
use serde_json::{Value, json};

use crate::looks::looks;
use crate::region::region;

pub struct Window {
    pub engine: Engine,
    pub host: Host,
    pub log: Option<PathBuf>,
}

fn origin(role: Role) -> Origin {
    let (client_id, client) = match role {
        Role::Human => (0, "window"),
        _ => (9, "claude"),
    };
    Origin {
        client_id,
        client: client.into(),
        role,
    }
}

/// A session with a sphere in the window, logging when `log` is given, and the 3D view's
/// rectangle.
pub fn with_sphere(log: Option<PathBuf>) -> (Window, [f32; 4]) {
    let mut w = Window::new(log);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let rect = w.view_rect();
    (w, rect)
}

/// A fresh log path, in a folder of the build's scratch space shared by these tests.
pub fn log_path(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pen-logs");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    dir.join(format!("{name}-{}-{n}.jsonl", std::process::id()))
}

impl Window {
    /// A session in a 640 by 480 window, logging (with scene hashes) when `log` is given.
    pub fn new(log: Option<PathBuf>) -> Self {
        Self::with(log, Keymap::default(), 1.0)
    }

    /// A session in a 640 by 480 pixel window at `density` pixels per point, with `keymap`: the
    /// header takes two rows over a view this narrow, its menus folded, and the view under them
    /// is still tall enough that a stroke across its middle stays on the sphere.
    pub fn with(log: Option<PathBuf>, keymap: Keymap, density: f32) -> Self {
        Self::sized(log, keymap, [640, 480], density)
    }

    /// A session in a window of `size` pixels at `density` pixels per point, with `keymap`. A
    /// test that needs a wider 3D view than the small window leaves beside the trays asks for a
    /// bigger window, rather than taking the trays away.
    pub fn sized(log: Option<PathBuf>, keymap: Keymap, size: [u32; 2], density: f32) -> Self {
        let config = EngineConfig {
            log_path: log.clone(),
            verify: true,
            ..EngineConfig::default()
        };
        let mut engine = Engine::new(config).unwrap_or_else(|e| panic!("{e}"));
        let layout = LayoutFile::in_memory(Layout::default());
        let shell = Shell::new(engine.catalog().clone(), keymap, layout);
        let renderer = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
        let render = Render::new(renderer, 1).unwrap_or_else(|e| panic!("{e}"));
        let host = Host::new(Studio::new(shell, Some(render), size, density));
        engine.attach_window(Box::new(host.clone()));
        Self { engine, host, log }
    }

    /// Runs a command as `role`.
    pub fn call_as(&mut self, role: Role, command: &str, args: Value) -> CmdResponse {
        let request = Request::Run(RunParams::new(command, args));
        self.engine.execute(&origin(role), request)
    }

    /// Runs a command that must succeed as the human, and returns its result.
    pub fn ok(&mut self, command: &str, args: Value) -> Value {
        match self.call_as(Role::Human, command, args).outcome {
            Outcome::Ok(ok) => ok.result,
            Outcome::Error(e) => panic!("{command}: {}", e.message),
        }
    }

    pub fn hash(&mut self) -> String {
        self.ok("mesh.hash", json!({}))["hash"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Where `name` shows now, in window pixels (see [`region`]).
    pub fn rect(&mut self, name: &str) -> [f64; 4] {
        region(&self.ok("ui.describe", json!({})), name)
    }

    /// The 3D view's rectangle, in window pixels, once the frames have settled.
    pub fn view_rect(&mut self) -> [f32; 4] {
        self.rect("viewport").map(|v| v as f32)
    }

    /// One pen sample, as one frame of the window sends it.
    pub fn pen(&mut self, s: PenSample) -> CmdResponse {
        self.call_as(Role::Human, "pen.input", json!({ "samples": [s] }))
    }

    /// Holds `held` and sends `inputs` through the studio, draws a frame, and runs what the
    /// interface asked for, as the window does.
    pub fn input(&mut self, inputs: &[Input], held: Modifiers) {
        let steps = {
            let mut studio = self.host.lock();
            studio.input(inputs, held);
            studio.frame().unwrap_or_else(|e| panic!("{e}"));
            studio.shell.take_commands()
        };
        for step in steps {
            self.ok(&step.command, step.args);
        }
    }

    /// Lets `secs` pass on the window's clock with no input, as the window's loop does: it
    /// looks every few milliseconds (`looks`) and draws a frame only when the interface asked
    /// to draw again by then (a timer that ran out, egui's frames after an event).
    pub fn wait(&mut self, secs: f64) {
        for step in looks(secs) {
            let steps = {
                let mut studio = self.host.lock();
                let now = studio.now_ns() + step;
                studio.at(now);
                if !studio.dirty {
                    continue;
                }
                studio.frame().unwrap_or_else(|e| panic!("{e}"));
                studio.shell.take_commands()
            };
            for step in steps {
                self.ok(&step.command, step.args);
            }
        }
    }

    /// Every call the log holds, oldest first, as (command, arguments).
    pub fn logged(&self) -> Vec<(String, Value)> {
        self.records()
            .into_iter()
            .map(|r| (r.command, r.args))
            .collect()
    }

    /// Every record the log holds, oldest first.
    pub fn records(&self) -> Vec<gs_protocol::LogRecord> {
        let path = self.log.as_ref().unwrap_or_else(|| panic!("no log"));
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{e}"));
        (text.lines())
            .filter_map(|l| match ScriptLine::parse(l) {
                Ok(ScriptLine::Record(r)) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// The label beside the pen's cursor (the brush, its size and strength), and its
    /// rectangle in window pixels.
    pub fn brush_label(&mut self) -> Option<(String, [f64; 4])> {
        let d = self.ok("ui.describe", json!({}));
        let panels = d["panels"].as_array().cloned().unwrap_or_default();
        let view = panels.iter().find(|p| p["id"] == "viewport")?;
        let label = view["widgets"].as_array()?.first()?;
        let rect = std::array::from_fn(|i| label["rect"][i].as_f64().unwrap_or_default());
        Some((label["label"].as_str()?.to_string(), rect))
    }

    /// Every line beside the pen's cursor, top to bottom, and the rings' names.
    pub fn cursor_lines(&mut self) -> Vec<String> {
        let d = self.ok("ui.describe", json!({}));
        let panels = d["panels"].as_array().cloned().unwrap_or_default();
        let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
        let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
        (widgets.unwrap_or_default().iter())
            .filter_map(|x| Some(x["label"].as_str()?.to_string()))
            .collect()
    }

    /// What `pick.ray` says the window's own camera shows at (`x`, `y`) in window pixels.
    pub fn picked(&mut self, x: f32, y: f32) -> Value {
        let d = self.ok("ui.describe", json!({}));
        let v = &d["viewport"];
        let r: [f64; 4] = std::array::from_fn(|i| v["rect"][i].as_f64().unwrap_or_default());
        let camera = v["camera"].clone();
        let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
        let at = [f64::from(x) - r[0], f64::from(y) - r[1]];
        let args = json!({ "camera": camera, "viewport": size, "at": at });
        self.ok("pick.ray", args)
    }

    /// The 3D view's pixels as presented (RGBA rows), without the panels around it, whose
    /// text changes with every call.
    pub fn view_pixels(&mut self) -> Vec<u8> {
        let rect = self.view_rect().map(|v| v.round() as usize);
        let shot = self.ok("ui.screenshot", json!({}));
        let width = shot["width"].as_u64().unwrap_or_default() as usize;
        let rgba = decode_png(shot["png_base64"].as_str().unwrap_or_default());
        let row = width * 4;
        (rect[1]..rect[3])
            .flat_map(|y| rgba[y * row + rect[0] * 4..y * row + rect[2] * 4].to_vec())
            .collect()
    }
}

/// A base64 PNG's RGBA rows.
pub fn decode_png(base64: &str) -> Vec<u8> {
    use base64::Engine as _;
    let bytes = (base64::engine::general_purpose::STANDARD.decode(base64))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap_or_else(|e| panic!("{e}"));
    let mut rgba = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut rgba)
        .unwrap_or_else(|e| panic!("{e}"));
    rgba.truncate(info.buffer_size());
    rgba
}

/// A pen sample at (`x`, `y`): touching with `pressure` when it is above 0, else hovering.
pub fn pen_at(t_ms: u64, x: f32, y: f32, pressure: f32) -> PenSample {
    PenSample {
        pressure,
        contact: pressure > 0.0,
        ..PenSample::hover(t_ms * 1_000_000, f64::from(x), f64::from(y))
    }
}

/// A stroke across the middle of `rect`: a hover, a touch, `moves` samples sideways with a
/// rising pressure, and the lift.
pub fn across(rect: [f32; 4], moves: u64) -> Vec<PenSample> {
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    let mut out = vec![pen_at(0, cx - 60.0, cy, 0.0)];
    for k in 0..=moves {
        let f = k as f32 / moves as f32;
        out.push(pen_at(
            4 * (k + 1),
            cx - 60.0 + 120.0 * f,
            cy + 10.0 * f,
            0.3 + 0.5 * f,
        ));
    }
    let last = out[out.len() - 1];
    out.push(PenSample {
        contact: false,
        pressure: 0.0,
        t_ns: last.t_ns + 4_000_000,
        ..last
    });
    out
}
