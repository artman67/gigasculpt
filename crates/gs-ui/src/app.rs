//! The window: SDL and the surface around the [`Studio`], and the engine running in the same
//! process with its socket server alongside, so `gs` and agents attach to the live window.
//! The window is the human (ARCHITECTURE 7.2); socket clients are agents.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use gs_engine::server::{JobWatch, ServeOptions, Server, Window};
use gs_engine::{EngineConfig, cli};
use gs_platform::{Input, Platform, Pumped, WindowOptions};
use gs_protocol::{
    Catalog, CmdResponse, CommandError, Outcome, PROTOCOL_VERSION, Request, RunParams, Step, method,
};
use gs_render::{AdapterKind, Renderer, UiLayer, WindowSurface};
use serde_json::json;

use crate::direction::Directions;
use crate::glue::{MousePen, split_pen};
use crate::host::Host;
use crate::keymap::Keymap;
use crate::layout::{Layout, LayoutFile};
use crate::perf::PenTimes;
use crate::record::with_args;
use crate::shell::Shell;
use crate::status::Busy;
use crate::studio::{Frame, Render, Studio};

/// How long the window sleeps waiting for input when nothing is moving. Pen and mouse events
/// wake it at once; agents' changes show within this time.
const IDLE: Duration = Duration::from_millis(8);
/// How often the window measures the memory the app holds, for the status line.
const MEMORY_EVERY_NS: u64 = 500_000_000;
/// The wait while egui animates something.
const ANIMATING: Duration = Duration::from_millis(4);

/// Opens the GigaSculpt window. The engine runs in this process, with its socket server
/// alongside, so `gs` and agents can attach to the live window.
#[derive(Parser, Debug)]
#[command(name = "gigasculpt window", version, after_help = cli::FOLDERS)]
pub struct WindowArgs {
    /// Session name: letters, digits, '-' and '_'.
    #[arg(long, default_value = "main")]
    pub session: String,
    #[arg(long, help = cli::SOCKET)]
    pub socket: Option<PathBuf>,
    #[arg(long, help = cli::LOG)]
    pub log: Option<PathBuf>,
    /// Write the scene hash after every logged call that changes the scene, as `gigasculpt
    /// serve --verify` does, so `gs run` checks a replay line by line. A session recorded to
    /// replay: `gigasculpt window --log PATH --verify`.
    #[arg(long)]
    pub verify: bool,
    /// Folder of the keymap, the layout state and the `layouts` folder of layout files.
    /// Default: `$XDG_CONFIG_HOME/gigasculpt`, else `~/.config/gigasculpt` on Linux,
    /// `~/Library/Application Support/GigaSculpt` on macOS, `%APPDATA%\GigaSculpt` on Windows.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// GPU: `lavapipe`, `nvidia` or `radv`. Default: the fastest one that can present.
    #[arg(long)]
    pub adapter: Option<String>,
    /// SDL video driver (`wayland`, `x11`). Default: SDL's choice.
    #[arg(long)]
    pub driver: Option<String>,
    /// Close the window by itself after this many seconds.
    #[arg(long)]
    pub close_after: Option<f64>,
    /// Window width in points.
    #[arg(long, default_value_t = 1280)]
    pub width: u32,
    /// Window height in points.
    #[arg(long, default_value_t = 800)]
    pub height: u32,
    /// Turn autosave and crash recovery off.
    #[arg(long)]
    pub no_autosave: bool,
}

/// True if the command line asks for the window: no arguments, or `window`.
pub fn wants_window(args: &[OsString]) -> bool {
    args.len() < 2 || args[1] == "window"
}

/// Runs `gigasculpt` or `gigasculpt window [options]`.
pub fn main(args: &[OsString]) -> ExitCode {
    let rest = args.iter().skip(1).filter(|a| *a != "window");
    let args = args.iter().take(1).chain(rest);
    let options = match WindowArgs::try_parse_from(args) {
        Ok(options) => options,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(if err.use_stderr() { 2 } else { 0 });
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gigasculpt: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The interface with the keymap, the layout state, the preferences and the layout files in
/// `config`, read as the window starts (and the tests' rig, see [`launch`]). A file that does
/// not read falls back to the defaults (a broken layout file is left out; a broken preferences
/// file is left as it is, never written over), and the status bar says why; it also says when
/// an old keymap file was moved aside and which keys name a command the program does not have.
/// What an older layout state holds of the preferences is handed to `preferences.toml` here,
/// before any frame.
pub fn shell(config: &std::path::Path) -> Shell {
    let mut problems = Vec::new();
    let keymap = match Keymap::load(&config.join("keymap.toml")) {
        Ok((keymap, said)) => {
            problems.extend(said);
            keymap
        }
        Err(e) => {
            problems.push(e);
            Keymap::default()
        }
    };
    // The user's preferences (decision 0052), which the first frame runs. Read before the
    // layout state, so what an older layout.toml holds of them is in preferences.toml before
    // layout.toml is written without it (decision 0052 as amended by step Prefs-c).
    let (mut prefs, problem) = crate::Preferences::load(&config.join("preferences.toml"));
    problems.extend(problem);
    let layout = LayoutFile::read(&config.join("layout.toml")).and_then(|mut file| {
        let handed = prefs.hand_over(&file.layout.moved());
        // A file that is never written already said why.
        if let Err(e) = handed
            && !problems.contains(&e)
        {
            problems.push(e);
        }
        file.write().map(|()| file)
    });
    let layout = layout.unwrap_or_else(|e| {
        problems.push(e);
        LayoutFile::in_memory(Layout::default())
    });
    let catalog = Catalog {
        engine_version: String::new(),
        protocol: PROTOCOL_VERSION,
        hash: String::new(),
        commands: Vec::new(),
    };
    let mut shell = Shell::new(catalog, keymap, layout);
    // The folder for the user's own layout files; shipped ones are built in.
    std::fs::create_dir_all(config.join("layouts")).ok();
    let (directions, broken) = Directions::load(&config.join("layouts"));
    problems.extend(broken);
    if !directions.list.is_empty() {
        shell.directions = directions;
    }
    shell.layouts = Some(config.join("layouts"));
    shell.keymap_file = Some(config.join("keymap.toml"));
    // The user's own themes, beside their layouts.
    std::fs::create_dir_all(config.join("themes")).ok();
    let (themes, broken) = crate::theme::Themes::load(&config.join("themes"));
    problems.extend(broken);
    shell.themes = themes;
    shell.prefs = prefs;
    shell.status.problem = (!problems.is_empty()).then(|| problems.join("; "));
    shell
}

/// What the window runs at start to read the user's own materials (decision 0043, step (d)):
/// `material.folder` with the `materials` folder in `config`, beside `layouts` and `themes`,
/// made if it is missing.
pub fn materials(config: &std::path::Path) -> Step {
    let folder = config.join("materials");
    std::fs::create_dir_all(&folder).ok();
    with_args("material.folder", json!({ "folder": folder }))
}

/// Opens the window and serves the session until the window closes.
pub fn run(o: &WindowArgs) -> Result<(), String> {
    // Checked before the window opens, so a bad or too long name never flashes one.
    socket(o)?;
    let kind = match &o.adapter {
        Some(name) => Some(AdapterKind::from_name(name).ok_or(format!("unknown adapter {name}"))?),
        None => None,
    };
    let config = o.config.clone().map_or_else(gs_protocol::config_dir, Ok)?;
    let shell = shell(&config);
    let mut platform = Platform::open(&WindowOptions {
        title: format!("GigaSculpt - {}", o.session),
        width: o.width,
        height: o.height,
        driver: o.driver.clone(),
    })?;
    let first = platform.pump(Duration::ZERO);
    let (w, h) = platform.size_in_pixels();
    let (renderer, mut surface) =
        Renderer::for_window(platform.surface_handle(), kind, w, h).map_err(|e| e.to_string())?;
    let render = Render::new(renderer, 16)?;
    let mut studio = Studio::new(shell, Some(render), [w, h], first.density);
    studio.content_scale(first.content_scale);
    let info = surface.info();
    studio.perf.adapter = info.adapter;
    studio.perf.present_mode = info.present_mode;
    let host = Host::new(studio);
    let server = start(o, &host)?;
    let window = server.window();
    launch(&config, &host, |request| window.execute(request));
    let closed = frames(&mut platform, &mut surface, &host, &window, o.close_after);
    window.close();
    server.wait().map_err(|e| e.to_string())?;
    closed
}

/// Starts the session behind the window: the engine as `o` sets it up, with its socket server
/// and `host` attached as the window. The log, its scene hashes and autosave follow the rules
/// `gigasculpt serve` uses (`EngineConfig::with_log`), so a window log replays as a headless
/// one does.
pub fn start(o: &WindowArgs, host: &Host) -> Result<Server, String> {
    let socket = socket(o)?;
    let config = EngineConfig {
        session: o.session.clone(),
        verify: o.verify,
        autosave: (!o.no_autosave)
            .then(gs_engine::StateFolder::standard)
            .flatten(),
        ..EngineConfig::default()
    }
    .with_log(o.log.clone())?;
    let options = ServeOptions {
        socket,
        stop_key: None,
        max_line_bytes: 256 << 20,
    };
    Server::start_with(config, &options, |engine| {
        host.lock().shell.catalog = engine.catalog().clone();
        // The renderer first: attaching the window reports the session's state, look included.
        engine.attach_renderer(Box::new(host.clone()));
        engine.attach_window(Box::new(host.clone()));
    })
    .map_err(|e| e.to_string())
}

/// The socket `o` names, else the session's own; a bad session name, or one too long for its
/// socket, is refused.
fn socket(o: &WindowArgs) -> Result<PathBuf, String> {
    gs_protocol::check_session_name(&o.session)?;
    match &o.socket {
        Some(socket) => Ok(socket.clone()),
        None => gs_protocol::socket_path(&o.session),
    }
}

/// What the window runs between attaching to the session and its first frame, in [`run`] and in
/// the tests' rig alike (decision 0052 as amended by step Prefs-c): the user's materials, then
/// what [`Shell::launched`] asks for with `file.recovery`'s and `file.recent`'s answers.
/// `execute` runs a request as the human at the window; `None` once the engine has stopped.
pub fn launch(
    config: &std::path::Path,
    host: &Host,
    mut execute: impl FnMut(Request) -> Option<CmdResponse>,
) {
    // The user's materials first; what it refuses, the Material section lists.
    execute(request(materials(config)));
    // A crashed session's work is offered back at start, or else the last scene opens when the
    // setting says so (decision 0016, sections 10 and 17); with autosave off both questions are
    // refused and nothing is offered or opened.
    let mut ask = |command: &str| match execute(request(with_args(command, json!({}))))?.outcome {
        Outcome::Ok(done) => Some(done.result),
        Outcome::Error(_) => None,
    };
    let (found, recent) = (ask("file.recovery"), ask("file.recent"));
    host.lock().shell.launched(found.as_ref(), recent.as_ref());
}

/// `step` run as the human at the window: a step named `cmd.batch` as a batch, one undo step
/// (decision 0041, amended for `DF7a`, section 5).
pub fn request(step: Step) -> Request {
    if step.command == gs_protocol::method::BATCH
        && let Ok(batch) = serde_json::from_value(step.args.clone())
    {
        return Request::Batch(batch);
    }
    Request::Run(RunParams::new(step.command, step.args))
}

/// Runs a command as the human at the window: done at once, a job for the status line's bar to
/// show (decision 0010 as amended 2026-10-13), or refused. `None` once the engine has stopped.
fn call(window: &Window, step: Step) -> Option<Result<Option<JobWatch>, CommandError>> {
    // The bar's Cancel: the socket's `job.status` request, which the window's door takes.
    if step.command == method::JOB_STATUS {
        let id = step.args["id"].as_u64().unwrap_or_default();
        window.job(id, step.args["cancel"] == true);
        return Some(Ok(None));
    }
    let response = window.execute(request(step))?;
    Some(match response.outcome {
        Outcome::Ok(done) if done.result["working"] == true => Ok(done.result["job"]
            .as_u64()
            .and_then(|id| window.job(id, false))),
        Outcome::Ok(_) => Ok(None),
        Outcome::Error(e) => Err(e),
    })
}

/// The window's loop: read input, send pen samples through `pen.input`, run the interface, run
/// what it asked for, then paint and present. Returns when the window closes or the time is up.
fn frames(
    platform: &mut Platform,
    surface: &mut WindowSurface,
    host: &Host,
    window: &Window,
    close_after: Option<f64>,
) -> Result<(), String> {
    let deadline = close_after.map(|s| platform.now_ns() + (s * 1e9) as u64);
    let mut wait = Duration::ZERO;
    let mut measured = 0;
    let mut mouse = MousePen::default();
    loop {
        let pumped = platform.pump(wait);
        // The memory the status line shows, about twice a second.
        if platform.now_ns().saturating_sub(measured) >= MEMORY_EVERY_NS {
            measured = platform.now_ns();
            host.lock().memory(crate::status::resident_mb());
        }
        let late = deadline.is_some_and(|d| platform.now_ns() >= d);
        if late || pumped.inputs.contains(&Input::Close) {
            return Ok(());
        }
        // The mouse's held buttons are the pen's (decision 0017 as amended).
        let inputs = mouse.convert(&pumped.inputs, platform.now_ns());
        let (samples, others) = split_pen(&inputs);
        let applied = if samples.is_empty() {
            None
        } else {
            let step = Step {
                command: "pen.input".into(),
                args: json!({ "samples": samples }),
            };
            report(host, None, call(window, step).ok_or("the engine stopped")?);
            Some(platform.now_ns())
        };
        let times = applied.zip(samples.last().map(|s| (s.t_ns, pumped.received_ns)));
        let pass = interface(platform, surface, host, &pumped, &others);
        // The interface's commands run before the frame is painted, so a navigation drag
        // shows in the frame it happened in.
        commands(host, window)?;
        let Some((ui, frame, begun)) = pass else {
            wait = IDLE;
            continue;
        };
        paint(platform, surface, host, &ui, times, begun)?;
        platform.set_text_input(frame.typing);
        // A still view that has not settled yet (decision 0019) wants its next sample soon.
        let settling = host.lock().dirty;
        wait = if frame.again || settling {
            ANIMATING
        } else {
            IDLE
        };
    }
}

/// One pass of the window loop's command path: runs what the interface asked for as the human
/// at the window. `Err` once the engine has stopped.
pub fn commands(host: &Host, window: &Window) -> Result<(), String> {
    // Taken first: the engine locks the studio.
    let steps = host.lock().shell.take_commands();
    for step in steps {
        let ran = call(window, step.clone()).ok_or("the engine stopped")?;
        report(host, Some(step), ran);
    }
    // The bar follows the window's job each pass, and goes once it ends.
    let mut studio = host.lock();
    if crate::status::follow(&mut studio.shell.status) {
        studio.dirty = true;
    }
    Ok(())
}

/// Shows a job a command became on the status line's bar, or a command's failure in the status
/// line, asking for the argument it names, if any (see [`Shell::refused`]). `step` is what
/// ran; `None` for the pen's samples.
fn report(host: &Host, step: Option<Step>, result: Result<Option<JobWatch>, CommandError>) {
    match result {
        Ok(Some(job)) => host.lock().shell.status.busy = Some(Busy::new(job)),
        Ok(None) => {}
        Err(e) => host.lock().shell.refused(step, &e),
    }
}

/// Takes in the time and this pump's input and, if anything changed or the interface asked to
/// draw again by now, runs the interface for a frame: the layer to paint, what the frame asks
/// of the window, and how long the pass took.
fn interface(
    platform: &Platform,
    surface: &mut WindowSurface,
    host: &Host,
    pumped: &Pumped,
    others: &[Input],
) -> Option<(UiLayer, Frame, u64)> {
    let mut studio = host.lock();
    if others.contains(&Input::Resized) {
        let (w, h) = platform.size_in_pixels();
        if let Some(render) = &studio.render {
            surface.resize(&render.renderer, w, h);
        }
        studio.resize([w, h], pumped.density);
    }
    studio.content_scale(pumped.content_scale);
    // The window never lays the interface out below 640 by 480 points at its scale.
    let scale = pumped.content_scale * studio.shell.layout.layout.ui_scale;
    platform.set_minimum_size([640.0, 480.0].map(|v| (v * scale).ceil() as u32));
    studio.at(platform.now_ns());
    studio.input(others, pumped.modifiers);
    if !studio.dirty {
        return None;
    }
    let begun = platform.now_ns();
    let (ui, frame) = studio.interface();
    Some((ui, frame, platform.now_ns() - begun))
}

/// Paints and presents a frame with the interface layer `ui`, which took `spent` nanoseconds to
/// build. `times` are the pen frame's applied, sampled and received times.
fn paint(
    platform: &Platform,
    surface: &mut WindowSurface,
    host: &Host,
    ui: &UiLayer,
    times: Option<(u64, (u64, u64))>,
    spent: u64,
) -> Result<(), String> {
    let mut studio = host.lock();
    let begun = platform.now_ns();
    studio.paint(ui)?;
    let submitted = platform.now_ns();
    if let Some(render) = &studio.render
        && let Some(target) = render.target()
    {
        surface.present(&render.renderer, target);
    }
    let presented = platform.now_ns();
    studio.perf.frame(spent + (submitted - begun));
    if let Some((applied, (sampled, received))) = times {
        studio.perf.pen(PenTimes {
            sampled,
            received,
            applied,
            submitted,
            presented,
        });
    }
    Ok(())
}
