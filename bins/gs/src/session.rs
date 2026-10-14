//! Finding, starting and stopping sessions.

use std::env::consts::EXE_SUFFIX;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use gs_protocol::{
    Role, ShutdownParams, UnixStream, check_session_name, method, session_dir, socket_path,
};
use serde_json::{Value, json};

use crate::connection::Connection;
use crate::failure::{Failure, because};

/// How often to look again while waiting for a session.
const POLL: Duration = Duration::from_millis(25);

/// What a session is started with.
pub struct Spawn {
    pub seed: u64,
    pub verify: bool,
    pub timeout: Duration,
}

/// The session to talk to: `-s NAME`, else `$GS_SESSION`, else the only one running.
pub fn pick(explicit: Option<String>) -> Result<String, Failure> {
    if let Some(name) = explicit.or_else(|| std::env::var("GS_SESSION").ok()) {
        return Ok(name);
    }
    let running = running()?;
    match running.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(Failure::Unreachable(
            "no session is running; start one with: gs spawn NAME".into(),
        )),
        several => Err(Failure::Usage(format!(
            "{} sessions are running ({}); pick one with -s NAME",
            several.len(),
            several.join(", ")
        ))),
    }
}

/// Connects to a session, waiting at most `patience` for each step.
pub fn connect(name: &str, role: Role, patience: Duration) -> Result<Connection, Failure> {
    Connection::open(&socket(name)?, role, patience)
}

/// True if session `name` accepts connections.
pub fn is_running(name: &str) -> Result<bool, Failure> {
    Ok(UnixStream::connect(socket(name)?).is_ok())
}

/// Names of the sessions accepting connections, sorted.
fn running() -> Result<Vec<String>, Failure> {
    let dir = session_dir().map_err(Failure::Unreachable)?;
    let mut names: Vec<String> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|e| e == "sock") && UnixStream::connect(path).is_ok()
        })
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    Ok(names)
}

/// The socket of a session. A name that is not a valid session name is a usage error, so it
/// can never point outside the session folder.
fn socket(name: &str) -> Result<PathBuf, Failure> {
    check_session_name(name).map_err(Failure::Usage)?;
    socket_path(name).map_err(Failure::Unreachable)
}

/// A fresh random stop key: 32 hex digits from the standard library's random hasher keys.
fn stop_key() -> String {
    let word = || {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.finish()
    };
    format!("{:016x}{:016x}", word(), word())
}

/// Starts a headless session in the background and waits until it answers. Returns what to
/// print: where it is, and the stop key only this spawner gets.
pub fn spawn(name: &str, how: &Spawn) -> Result<Vec<String>, Failure> {
    let socket = socket(name)?;
    if UnixStream::connect(&socket).is_ok() {
        return Err(Failure::Usage(format!(
            "session '{name}' is already running"
        )));
    }
    let dir = session_dir().map_err(Failure::Unreachable)?;
    fs::create_dir_all(&dir).map_err(|e| Failure::Unreachable(e.to_string()))?;
    let errors = dir.join(format!("{name}.stderr"));
    let stderr = File::create(&errors).map_err(|e| Failure::Unreachable(e.to_string()))?;
    let key = stop_key();
    let engine = engine_program();
    let seed = how.seed.to_string();
    let mut args = vec![
        "serve",
        "--headless",
        "--session",
        name,
        "--seed",
        &seed,
        "--stop-key",
        &key,
    ];
    if how.verify {
        args.push("--verify");
    }
    let mut child = Command::new(&engine)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .detached()
        .spawn()
        .map_err(because(
            Failure::Unreachable,
            format!("cannot start {}", engine.display()),
        ))?;
    for _ in 0..how.timeout.as_millis().div_ceil(POLL.as_millis()) {
        if let Ok(Some(status)) = child.try_wait() {
            let output = fs::read_to_string(&errors).unwrap_or_default();
            return Err(Failure::Unreachable(format!(
                "the engine exited ({status}): {}",
                output.trim()
            )));
        }
        if UnixStream::connect(&socket).is_ok() {
            return Ok(vec![
                format!("session '{name}' is ready at {}", socket.display()),
                format!("stop key: {key} (stop it with: gs -s {name} stop --key {key})"),
            ]);
        }
        thread::sleep(POLL);
    }
    child.kill().ok();
    child.wait().ok();
    Err(Failure::Timeout(format!(
        "the engine did not start within {} ms",
        how.timeout.as_millis()
    )))
}

/// The engine program: `$GS_ENGINE`, else `gigasculpt` next to this program, else `gigasculpt`
/// on the `PATH`.
pub fn engine_program() -> PathBuf {
    if let Some(engine) = std::env::var_os("GS_ENGINE") {
        return engine.into();
    }
    let sibling = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(format!("gigasculpt{EXE_SUFFIX}")));
    sibling
        .filter(|path| path.exists())
        .unwrap_or_else(|| "gigasculpt".into())
}

/// Starting the engine apart from this terminal, so the terminal's Ctrl-C reaches `gs` alone.
pub trait Detach {
    /// Unix: a process group of its own. Windows: a process group of its own and no console
    /// window.
    fn detached(&mut self) -> &mut Self;
}

impl Detach for Command {
    fn detached(&mut self) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(self, 0);
        // CREATE_NEW_PROCESS_GROUP and CREATE_NO_WINDOW.
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(self, 0x0000_0200 | 0x0800_0000);
        self
    }
}

/// Stops a session with the stop key `gs spawn` printed, and waits until its socket is gone.
pub fn stop(name: &str, key: Option<String>, patience: Duration) -> Result<String, Failure> {
    let socket = socket(name)?;
    let params = json!(ShutdownParams { key });
    connect(name, Role::Agent, patience)?.call::<Value>(method::SHUTDOWN, params)?;
    for _ in 0..patience.as_millis().div_ceil(POLL.as_millis()) {
        if !socket.exists() {
            return Ok(format!("session '{name}' stopped"));
        }
        thread::sleep(POLL);
    }
    Err(Failure::Timeout(format!(
        "session '{name}' did not stop within {} ms",
        patience.as_millis()
    )))
}
