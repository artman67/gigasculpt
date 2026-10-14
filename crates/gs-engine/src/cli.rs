//! The `gigasculpt` command line. The binary's `main` is one call to `cli::main_with`, so tests
//! can run exactly the same code.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use crate::server::{ServeOptions, Server};
use crate::{EngineConfig, NoRenderer, RenderHost, StateFolder};

#[derive(Parser)]
#[command(
    name = "gigasculpt",
    version,
    about = "The GigaSculpt app and headless engine."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(Subcommand)]
enum Sub {
    /// Run a session engine that clients (gs, MCP, tests) connect to over a Unix socket.
    Serve(ServeArgs),
}

#[derive(Args)]
struct ServeArgs {
    /// Run without a window. Required until the window exists.
    #[arg(long)]
    headless: bool,
    /// Session name: letters, digits, '-' and '_'.
    #[arg(long, default_value = "main")]
    session: String,
    /// Seed for every random number in the session.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Undo history budget in MiB. The newest step is always kept.
    #[arg(long, default_value_t = 2048)]
    history_mib: u64,
    /// Socket to listen on. Default: `$XDG_RUNTIME_DIR/gigasculpt/<session>.sock`.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Command log to write, never pruned. Default: `logs/<session>-<pid>.jsonl` in
    /// `$XDG_STATE_HOME/gigasculpt`, which survives a reboot; the oldest closed logs there are
    /// pruned. An older log at the path is kept under a numbered name.
    #[arg(long)]
    log: Option<PathBuf>,
    /// Write the scene hash after every logged call, so `gs run` can name the first call that
    /// replays differently.
    #[arg(long)]
    verify: bool,
    /// The key an agent must give to stop the session (`gs spawn` makes one). Without it, no
    /// agent can stop the session.
    #[arg(long)]
    stop_key: Option<String>,
    /// The longest request line accepted, in MiB.
    #[arg(long, default_value_t = 256)]
    max_line_mib: usize,
    /// The device offscreen looks render on: lavapipe (the default, which draws the same
    /// bytes every run), nvidia or radv. Opened at the first look.
    #[arg(long)]
    adapter: Option<String>,
    /// Turn autosave and crash recovery on: a copy of the scene goes to
    /// `$XDG_STATE_HOME/gigasculpt/autosave/` 10 minutes after the last save and after 2
    /// minutes without a call, and `file.recovery` offers crashed sessions' work back. Off by
    /// default here, so agents' throwaway sessions leave nothing behind; the window has it on.
    #[arg(long)]
    autosave: bool,
}

/// Makes a headless session's offscreen renderer from the `--adapter` name.
pub type MakeRenderer = fn(Option<&str>) -> Result<Box<dyn RenderHost>, String>;

/// Runs the `gigasculpt` command line with these arguments (the first is the program name), in
/// a session that cannot render (`view.snapshot` says so).
pub fn main(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    main_with(args, |_| Ok(Box::new(NoRenderer)))
}

/// Runs the `gigasculpt` command line with these arguments (the first is the program name); a
/// headless session renders its offscreen looks with what `renderer` makes.
pub fn main_with(args: impl IntoIterator<Item = OsString>, renderer: MakeRenderer) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            err.print().ok();
            return ExitCode::from(if err.use_stderr() { 2 } else { 0 });
        }
    };
    let Some(Sub::Serve(args)) = cli.command.filter(|Sub::Serve(args)| args.headless) else {
        eprintln!("The window is not built yet. Run: gigasculpt serve --headless --session NAME");
        return ExitCode::from(2);
    };
    match run_serve(args, renderer) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gigasculpt: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run_serve(args: ServeArgs, renderer: MakeRenderer) -> Result<(), String> {
    gs_protocol::check_session_name(&args.session)?;
    let render = renderer(args.adapter.as_deref())?;
    let socket = match args.socket {
        Some(socket) => socket,
        None => gs_protocol::socket_path(&args.session)?,
    };
    let config = EngineConfig {
        session: args.session,
        seed: args.seed,
        history_budget_bytes: args.history_mib << 20,
        verify: args.verify,
        autosave: args.autosave.then(StateFolder::standard).flatten(),
        ..EngineConfig::default()
    }
    .with_log(args.log)?;
    let options = ServeOptions {
        socket,
        stop_key: args.stop_key,
        max_line_bytes: args.max_line_mib.saturating_mul(1 << 20),
    };
    let server = Server::start_with(config, &options, |engine| engine.attach_renderer(render));
    server.and_then(Server::wait).map_err(|e| e.to_string())
}
