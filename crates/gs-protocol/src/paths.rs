//! Where sessions live, what a session may be called, and how a file that would land on another
//! is renamed, so the engine and every client agree without talking first.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The systems whose per-user folders differ. Every Unix system but macOS follows Linux (the XDG
/// Base Directory rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Linux,
    MacOs,
    Windows,
}

impl System {
    /// The system this program was built for.
    pub const HERE: Self = if cfg!(target_os = "macos") {
        Self::MacOs
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };
}

/// An environment: the value of a variable by its name.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The folder holding every session's socket, from this process's environment.
pub fn session_dir() -> Result<PathBuf, String> {
    session_dir_in(System::HERE, &|name| std::env::var_os(name))
}

/// The session folder on `system` in `env`: `$XDG_RUNTIME_DIR/gigasculpt` wherever that is set
/// (desktop Linux sessions always set it; tests and sessions run side by side name a folder of
/// their own with it); else none on Linux, and the `sessions` folder in the app's own folder
/// ([`state_dir_in`]'s) on macOS and Windows.
pub fn session_dir_in(system: System, env: Env) -> Result<PathBuf, String> {
    if let Some(dir) = env("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir).join("gigasculpt"));
    }
    match system {
        System::Linux => Err("XDG_RUNTIME_DIR is not set; set it to a private folder".into()),
        System::MacOs | System::Windows => Ok(app_dir(system, env)?.join("sessions")),
    }
}

/// The folder for what must survive a reboot (autosaves, crash markers, command logs), from this
/// process's environment.
pub fn state_dir() -> Result<PathBuf, String> {
    state_dir_in(System::HERE, &|name| std::env::var_os(name))
}

/// The state folder on `system` in `env`: `$XDG_STATE_HOME/gigasculpt` wherever that is an
/// absolute path; else `$HOME/.local/state/gigasculpt` on Linux, and the app's own folder
/// elsewhere.
pub fn state_dir_in(system: System, env: Env) -> Result<PathBuf, String> {
    if let Some(dir) = xdg(env, "XDG_STATE_HOME") {
        return Ok(dir);
    }
    match system {
        System::Linux => in_home(env, ".local/state/gigasculpt", "XDG_STATE_HOME"),
        System::MacOs | System::Windows => app_dir(system, env),
    }
}

/// The folder for what can be made again (the Quad Remesh program), from this process's
/// environment.
pub fn cache_dir() -> Result<PathBuf, String> {
    cache_dir_in(System::HERE, &|name| std::env::var_os(name))
}

/// The cache folder on `system` in `env`: `$XDG_CACHE_HOME/gigasculpt` wherever that is an
/// absolute path; else `$HOME/.cache/gigasculpt` on Linux, `~/Library/Caches/GigaSculpt` on
/// macOS and `cache` in the app's own folder on Windows.
pub fn cache_dir_in(system: System, env: Env) -> Result<PathBuf, String> {
    if let Some(dir) = xdg(env, "XDG_CACHE_HOME") {
        return Ok(dir);
    }
    match system {
        System::Linux => in_home(env, ".cache/gigasculpt", "XDG_CACHE_HOME"),
        System::MacOs => in_home(env, "Library/Caches/GigaSculpt", "XDG_CACHE_HOME"),
        System::Windows => Ok(app_dir(system, env)?.join("cache")),
    }
}

/// The folder for the user's settings (keys, layouts, preferences, materials), from this
/// process's environment.
pub fn config_dir() -> Result<PathBuf, String> {
    config_dir_in(System::HERE, &|name| std::env::var_os(name))
}

/// The config folder on `system` in `env`: `$XDG_CONFIG_HOME/gigasculpt` wherever that is an
/// absolute path; else `$HOME/.config/gigasculpt` on Linux, the app's own folder on macOS and
/// `%APPDATA%\GigaSculpt` on Windows (the roaming folder, which follows the user to another
/// machine as settings should).
pub fn config_dir_in(system: System, env: Env) -> Result<PathBuf, String> {
    if let Some(dir) = xdg(env, "XDG_CONFIG_HOME") {
        return Ok(dir);
    }
    match system {
        System::Linux => in_home(env, ".config/gigasculpt", "XDG_CONFIG_HOME"),
        System::MacOs => app_dir(system, env),
        System::Windows => (env("APPDATA").map(|dir| PathBuf::from(dir).join("GigaSculpt")))
            .ok_or_else(|| "APPDATA is not set".into()),
    }
}

/// `$<var>/gigasculpt` when `var` is an absolute path: as the XDG Base Directory spec says, an
/// empty or relative one counts as unset.
fn xdg(env: Env, var: &str) -> Option<PathBuf> {
    let dir = env(var).map(PathBuf::from)?;
    dir.is_absolute().then(|| dir.join("gigasculpt"))
}

/// `sub` in the home folder; `var` is the variable that would have won.
fn in_home(env: Env, sub: &str, var: &str) -> Result<PathBuf, String> {
    (env("HOME").map(|home| PathBuf::from(home).join(sub)))
        .ok_or_else(|| format!("neither {var} nor HOME is set"))
}

/// The app's own per-user folder: `~/Library/Application Support/GigaSculpt` on macOS,
/// `%LOCALAPPDATA%\GigaSculpt` on Windows.
fn app_dir(system: System, env: Env) -> Result<PathBuf, String> {
    let base = if system == System::Windows {
        env("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or("LOCALAPPDATA is not set")
    } else {
        (env("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support")))
            .ok_or("HOME is not set")
    };
    base.map(|dir| dir.join("GigaSculpt")).map_err(String::from)
}

/// The socket of session `name`, from this process's environment (see [`socket_path_in`]).
pub fn socket_path(name: &str) -> Result<PathBuf, String> {
    socket_path_in(System::HERE, &|name| std::env::var_os(name), name)
}

/// The socket of session `name` on `system` in `env`: `<session folder>/<name>.sock`. Refuses
/// names that are not a valid session name, so no name can point outside the session folder,
/// and names whose socket path would not fit in the system's socket address, before any bind.
pub fn socket_path_in(system: System, env: Env, name: &str) -> Result<PathBuf, String> {
    check_session_name(name)?;
    let dir = session_dir_in(system, env)?;
    let path = dir.join(format!("{name}.sock"));
    let (bytes, limit) = (path.as_os_str().len(), crate::socket::longest_path(system));
    if bytes <= limit {
        return Ok(path);
    }
    let fits = match name.len().checked_sub(bytes - limit) {
        Some(n) if n > 0 => format!("names of up to {n} characters fit"),
        _ => "no name fits; set XDG_RUNTIME_DIR to a shorter folder".to_string(),
    };
    Err(format!(
        "session name '{name}' is too long: its socket {} would be {bytes} bytes, and sockets \
         take at most {limit} bytes here; in {}, {fits}",
        path.display(),
        dir.display()
    ))
}

/// A session name is one or more ASCII letters, digits, `-` and `_`.
pub fn check_session_name(name: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if name.is_empty() || !name.chars().all(allowed) {
        return Err(format!(
            "bad session name '{name}': use letters, digits, '-' and '_'"
        ));
    }
    Ok(())
}

/// `path` if nothing is there, else the first free `<stem>-<n>.<extension>` beside it, counting
/// from 1. A kept command log, a panic's bug bundle and `gs run --repro-out`'s bundle are all
/// named this way.
pub fn free_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    let extension = extension.unwrap_or_default();
    let mut n = 1_u64;
    loop {
        let candidate = path.with_file_name(format!("{stem}-{n}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}
