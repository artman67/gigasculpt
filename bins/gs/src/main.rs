//! Thin command-line client for GigaSculpt sessions, and the MCP server for agents (`gs mcp`).
//! It links only `gs-protocol`: it asks the running engine for its catalog and builds each
//! command's arguments from the schemas there, so it starts in milliseconds and is never out of
//! date with the engine. The async runtime MCP needs starts only inside `gs mcp`.

mod args;
mod calls;
mod connection;
mod failure;
mod help;
mod look;
mod mcp;
mod output;
mod repro;
mod script;
mod session;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use gs_protocol::Role;
use serde_json::json;

use crate::args::Parsed;
use crate::connection::{Connection, one};
use crate::failure::Failure;
use crate::output::{Surface, checked, report_of};

#[derive(Parser)]
#[command(
    name = "gs",
    version,
    about = "Drive a GigaSculpt session from the command line."
)]
struct Cli {
    /// Session to talk to. Default: `$GS_SESSION`, else the only running session.
    #[arg(short, long, global = true)]
    session: Option<String>,
    /// Print the full JSON response instead of short text.
    #[arg(long, global = true)]
    json: bool,
    /// Connect as an agent, or as an observer that may only run queries. (The human role
    /// belongs to the window.) Set `GS_CLIENT` to give this client its own name (default
    /// `gs`): steps are attributed to it, and an agent's plain undo only moves its own steps.
    #[arg(long, global = true, value_enum, default_value_t = ClientRole::Agent)]
    role: ClientRole,
    /// How long to wait for the session to connect and to answer, in milliseconds. A long
    /// command still running after this keeps running as a job (see `gs job`).
    #[arg(long, global = true, default_value_t = 120_000)]
    timeout_ms: u64,
    #[command(subcommand)]
    command: Sub,
}

/// The roles a `gs` client can have.
#[derive(Clone, Copy, ValueEnum)]
enum ClientRole {
    Agent,
    Observer,
}

#[derive(Subcommand)]
enum Sub {
    /// Run a command: `gs call primitive.sphere subdivisions=6`. `gs call NAME --help` shows
    /// its arguments.
    #[command(disable_help_flag = true)]
    Call {
        /// Command name.
        name: String,
        /// Arguments: --key value, key=value, @file.json or a JSON object.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run a query command, which never changes anything.
    #[command(disable_help_flag = true)]
    Query {
        /// Command name.
        name: String,
        /// Arguments, as for `gs call`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run a JSONL script, one {"command": ..., "args": ...} per line, stopping at the first
    /// error. A session's command log is such a script; replaying it into a fresh session
    /// rebuilds the scene, and a log written with --verify stops at the first line that differs.
    Run(script::RunArgs),
    /// List every command, or those matching words: `gs commands sphere`. An exact command
    /// name shows its docs, argument schema and examples: `gs commands primitive.sphere`.
    Commands {
        /// Words to search for, or one command name.
        query: Vec<String>,
    },
    /// Look at a long command running as a job: how far it got, or its result once it ended.
    Job {
        /// The job's id, as `gs call` printed it.
        id: u64,
        /// Wait this long for it to end first, in milliseconds.
        #[arg(long, default_value_t = 0)]
        wait_ms: u64,
        /// Cancel it. It ends at once and changes nothing.
        #[arg(long)]
        cancel: bool,
    },
    /// Render the scene offscreen to a PNG, never moving the window's camera: `gs look --views
    /// front,iso --out a.png`. Several views make one labelled contact sheet.
    Look {
        /// Views, comma separated: front, back, left, right, top, bottom, iso. Default: iso.
        #[arg(long, value_delimiter = ',')]
        views: Vec<String>,
        /// One more view from orbit angles: AZIMUTH,ELEVATION in degrees.
        #[arg(long, value_delimiter = ',', allow_negative_numbers = true)]
        orbit: Vec<f32>,
        /// Image width in pixels.
        #[arg(long, default_value_t = 1024)]
        size: u32,
        /// matcap, lit, clay, zebra or normals. Default: the session's (`view.display`; matcap
        /// until it is set).
        #[arg(long)]
        shading: Option<String>,
        /// Draw the edges over the surface.
        #[arg(long)]
        wireframe: bool,
        /// The PNG to write.
        #[arg(long)]
        out: PathBuf,
    },
    /// Bug bundles (`NAME.gsrepro` folders): replay one and say whether its bug happens, shrink
    /// it to the commands the bug needs, or keep a fixed one as a test.
    Repro {
        #[command(subcommand)]
        action: repro::Action,
    },
    /// Start a headless session in the background and wait until it is ready. Prints the stop
    /// key that `gs stop` needs.
    Spawn {
        /// Session name: letters, digits, '-' and '_'.
        name: String,
        /// Seed for every random number in the session.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Write the scene hash after every logged call, so `gs run` can check a replay.
        #[arg(long)]
        verify: bool,
        /// How long to wait for it, in milliseconds.
        #[arg(long, default_value_t = 10_000)]
        wait_ms: u64,
    },
    /// Stop a session you spawned, with the stop key `gs spawn` printed.
    Stop {
        /// The stop key.
        #[arg(long)]
        key: Option<String>,
    },
    /// Serve the session to an agent over MCP on stdin and stdout, with six tools: `gs_help`,
    /// `gs_run`, `gs_batch`, `gs_query`, `gs_look` and `gs_job`.
    Mcp {
        /// Start the session (`-s NAME`, else `$GS_SESSION`, else `main`) when it is not
        /// running. It keeps running after this server exits.
        #[arg(long)]
        spawn: bool,
    },
}

fn main() -> ExitCode {
    let mut out = Vec::new();
    let code = match run(Cli::parse(), &mut out) {
        Ok(()) => 0,
        Err(failure) => {
            write_lines(&mut io::stderr(), &output::failure(&failure));
            failure.exit_code()
        }
    };
    write_lines(&mut io::stdout(), &out);
    ExitCode::from(code)
}

fn run(cli: Cli, out: &mut Vec<String>) -> Result<(), Failure> {
    let role = match cli.role {
        ClientRole::Agent => Role::Agent,
        ClientRole::Observer => Role::Observer,
    };
    let patience = Duration::from_millis(cli.timeout_ms);
    let open = |session| session::connect(&session::pick(session)?, role, patience);
    match cli.command {
        Sub::Call { name, args } => {
            call(&mut open(cli.session)?, &name, args, false, cli.json, out)
        }
        Sub::Query { name, args } => {
            call(&mut open(cli.session)?, &name, args, true, cli.json, out)
        }
        Sub::Run(args) => script::run(&mut open(cli.session)?, &args, cli.json, out),
        Sub::Repro { action } => repro::run(action, cli.json, out),
        Sub::Commands { query } => commands(&mut open(cli.session)?, &query, out),
        Sub::Job {
            id,
            wait_ms,
            cancel,
        } => {
            let wait = Duration::from_millis(wait_ms);
            job(&mut open(cli.session)?, id, wait, cancel, cli.json, out)
        }
        Sub::Look {
            views,
            orbit,
            size,
            shading,
            wireframe,
            out: path,
        } => {
            let mut views: Vec<_> = views.into_iter().map(|name| json!(name)).collect();
            match orbit[..] {
                [] => {}
                [azimuth, elevation] => {
                    views.push(json!({ "azimuth_deg": azimuth, "elevation_deg": elevation }));
                }
                _ => return Err(Failure::Usage("give --orbit as AZIMUTH,ELEVATION".into())),
            }
            // The engine writes the file, from its own working folder.
            let path = std::path::absolute(&path)
                .map_err(|e| Failure::Usage(format!("bad --out {}: {e}", path.display())))?;
            let request = look::Look {
                views,
                size,
                shading,
                wireframe,
                path: Some(path),
            };
            look(&mut open(cli.session)?, &request, cli.json, out)
        }
        Sub::Spawn {
            name,
            seed,
            verify,
            wait_ms,
        } => {
            let how = session::Spawn {
                seed,
                verify,
                timeout: Duration::from_millis(wait_ms),
            };
            out.extend(session::spawn(&name, &how)?);
            Ok(())
        }
        Sub::Stop { key } => {
            out.push(session::stop(&session::pick(cli.session)?, key, patience)?);
            Ok(())
        }
        Sub::Mcp { spawn } => mcp::serve(mcp::Settings {
            session: cli.session,
            spawn,
            role,
            patience,
        }),
    }
}

/// `gs commands`: one exact command name is described; words search; nothing lists all.
fn commands(
    connection: &mut Connection,
    query: &[String],
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let catalog = connection.catalog()?;
    out.extend(match query {
        [name] if catalog.commands.iter().any(|c| &c.name == name) => {
            help::describe(connection, name)?
        }
        _ => help::list(connection, &query.join(" "))?,
    });
    Ok(())
}

/// `gs job`: the job's result once it ended, or how far it got.
fn job(
    connection: &mut Connection,
    id: u64,
    wait: Duration,
    cancel: bool,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let job = calls::status(connection, &json!(id), wait, cancel)?;
    match job.get("response") {
        Some(response) => {
            return report_of(
                job["command"].as_str().unwrap_or_default(),
                response,
                json,
                out,
            );
        }
        None if json => out.push(job.to_string()),
        None => out.extend(output::running(&job, Surface::Cli)),
    }
    Ok(())
}

/// `gs look`: renders the views to a file.
fn look(
    connection: &mut Connection,
    request: &look::Look,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let response = look::look(connection, request)?;
    if json {
        out.push(response.to_string());
    }
    checked(&response)?;
    if !json {
        let result = &response["ok"]["result"];
        out.push(look::summary(result));
        out.push(format!(
            "wrote {}",
            result["path"].as_str().unwrap_or_default()
        ));
    }
    Ok(())
}

/// Runs one command, building its arguments from the catalog's schema.
fn call(
    connection: &mut Connection,
    name: &str,
    words: Vec<String>,
    query_only: bool,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let catalog = connection.catalog()?;
    if query_only {
        calls::must_be_query(&catalog, name)?;
    }
    let (args, late_json) = match catalog.commands.iter().find(|info| info.name == name) {
        Some(info) => match args::parse(info, words)? {
            Parsed::Args(args, late_json) => (args, late_json),
            Parsed::Help(help) => {
                out.push(help);
                return Ok(());
            }
        },
        // Unknown names go to the engine, which answers with did-you-mean suggestions.
        None => (json!({}), false),
    };
    let response = connection.send(one(name, args))?;
    report_of(name, &response, json || late_json, out)
}

fn write_lines(stream: &mut impl Write, lines: &[String]) {
    for line in lines {
        if writeln!(stream, "{line}").is_err() {
            return;
        }
    }
}
