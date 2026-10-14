//! Finding the program and running its two steps (decision 0038, sections 1 and 2): a program
//! folder counts only with the right stamp; each step runs as a child process, on Linux inside
//! bubblewrap when it is installed, on macOS and Windows with no sandbox, with the environment
//! cleared but for what Windows needs to start a program ([`KEPT`]), its output to a log in the
//! job folder, and checked every 20 ms for cancel and the time limit. A step ended by a signal
//! is refused, after one more run when the signal was SIGKILL (amended 2026-10-13).

use std::env::consts::EXE_SUFFIX;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use gs_protocol::System;

use super::{PIN, QuadError};

/// How often a running step is checked for cancel and the time limit.
const TICK: Duration = Duration::from_millis(20);
/// The time limit for both steps together, in ticks: 600 s. Counted, never read from the
/// clock, so a loaded machine only makes it longer.
pub const LIMIT_TICKS: u32 = 600 * 50;
/// The two programs a folder must hold.
const PROGRAMS: [&str; 2] = ["quadwild", "quad_from_patches"];
/// The step 2 settings the run copies, relative to the program folder.
pub(crate) const MAIN_CONFIG: &str = "config/main_config/flow_noalign_lemon.txt";
/// The variables a step keeps when its environment is cleared. Windows needs `SystemRoot` to
/// start a program at all: without it the loader cannot find side-by-side assemblies and
/// Winsock fails (Python's `subprocess` documentation says so too). Linux and macOS need none.
const KEPT: &[&str] = if cfg!(windows) { &["SystemRoot"] } else { &[] };
/// The one signal a step is run again after.
pub(crate) const SIGKILL: i32 = 9;
/// Linux's names for signals 1 to 15.
const SIGNALS: [&str; 15] = [
    "SIGHUP", "SIGINT", "SIGQUIT", "SIGILL", "SIGTRAP", "SIGABRT", "SIGBUS", "SIGFPE", "SIGKILL",
    "SIGUSR1", "SIGSEGV", "SIGUSR2", "SIGPIPE", "SIGALRM", "SIGTERM",
];

/// Linux's name for `signal`, when it is one of the first fifteen.
pub(crate) fn signal_name(signal: i32) -> Option<&'static str> {
    let at = usize::try_from(signal).ok()?.checked_sub(1)?;
    SIGNALS.get(at).copied()
}

/// A program folder with the stamp GigaSculpt was built for, and how its steps run.
#[derive(Clone, Debug)]
pub struct Program {
    dir: PathBuf,
    bwrap: Option<PathBuf>,
    limit: u32,
}

impl Program {
    /// The first of `places` holding the program for [`PIN`], run inside `bwrap` when that is
    /// given. Not found: why each place was passed over.
    pub fn find(places: &[PathBuf], bwrap: Option<PathBuf>) -> Result<Self, String> {
        let mut why = Vec::new();
        for dir in places {
            match check(dir) {
                Ok(()) => {
                    return Ok(Self {
                        dir: dir.clone(),
                        bwrap,
                        limit: LIMIT_TICKS,
                    });
                }
                Err(e) => why.push(format!("{}: {e}", dir.display())),
            }
        }
        if why.is_empty() {
            why.push("no folder to look in".into());
        }
        Err(why.join("; "))
    }

    /// The program folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the steps run inside bubblewrap.
    pub fn sandboxed(&self) -> bool {
        self.bwrap.is_some()
    }

    /// The same program with a time limit of `ticks` (20 ms each) for both steps together, in
    /// place of [`LIMIT_TICKS`]: for tests.
    #[must_use]
    pub fn limit(self, ticks: u32) -> Self {
        Self {
            limit: ticks,
            ..self
        }
    }
}

/// Where the program is looked for on `system` (decision 0038, section 1): only `set`
/// (`GS_QUADWILD`) when it is set; else `quadwild/` in `beside` (the running binary's folder,
/// where a release puts it), on macOS also `Resources/quadwild` beside that folder (a release
/// app's `Contents/Resources`), then `quadwild/PIN/program` in `cache` (`gs_protocol::cache_dir`).
pub fn places(
    system: System,
    set: Option<OsString>,
    beside: Option<&Path>,
    cache: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(dir) = set {
        return vec![PathBuf::from(dir)];
    }
    let app = beside
        .filter(|_| system == System::MacOs)
        .and_then(Path::parent);
    let cached = cache.map(|c| pinned(c).join("program"));
    (beside.map(|b| b.join("quadwild")).into_iter())
        .chain(app.map(|c| c.join("Resources").join("quadwild")))
        .chain(cached)
        .collect()
}

/// The folder for this pin's build in the cache folder `cache`.
fn pinned(cache: &Path) -> PathBuf {
    cache.join("quadwild").join(PIN)
}

/// How to get the program when it is missing on `system`, `cache` being the cache folder: a
/// download carries it; from source, the scripts build it there (from Git Bash on Windows).
pub fn fix(system: System, cache: Option<&Path>) -> String {
    let dir = cache.map_or_else(|| "a folder".into(), |d| pinned(d).display().to_string());
    let shell = if system == System::Windows {
        " from Git Bash"
    } else {
        ""
    };
    format!(
        "every GigaSculpt download carries it; from source, build it with \
         third_party/quadwild/fetch.sh DIR and third_party/quadwild/build.sh DIR{shell}, DIR \
         being {dir}, or set GS_QUADWILD to a program folder built for {PIN}"
    )
}

/// `bwrap` in one of the folders of `path` (`PATH`), if it is installed. Bubblewrap is Linux's
/// alone: elsewhere the steps run with no sandbox (decision 0038, section 2).
pub fn bwrap_in(path: Option<OsString>) -> Option<PathBuf> {
    let path = path.filter(|_| cfg!(target_os = "linux"))?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("bwrap"))
        .find(|b| b.is_absolute() && b.is_file())
}

/// The program `name` in `dir`, with this system's suffix (`.exe` on Windows).
fn program_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{EXE_SUFFIX}"))
}

/// Why `dir` is not a program folder for [`PIN`], if it is not.
fn check(dir: &Path) -> Result<(), String> {
    let stamp = fs::read_to_string(dir.join("stamp")).map_err(|_| "no stamp".to_string())?;
    if stamp.trim() != PIN {
        return Err(format!("built for {}, not {PIN}", stamp.trim()));
    }
    for p in PROGRAMS {
        if !program_in(dir, p).is_file() {
            return Err(format!("no {p}{EXE_SUFFIX}"));
        }
    }
    if !dir.join(MAIN_CONFIG).is_file() {
        return Err(format!("no {MAIN_CONFIG}"));
    }
    Ok(())
}

/// The signal that ended a process, if one did.
#[cfg(unix)]
fn killed_by(status: ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

/// Windows has no signals: a process ends with an exit code.
#[cfg(not(unix))]
fn killed_by(_: ExitStatus) -> Option<i32> {
    None
}

/// A job folder, made empty for one run and removed when dropped, whatever the outcome.
#[derive(Debug)]
pub struct Job {
    dir: PathBuf,
}

impl Job {
    /// Makes the folder `dir`, which must be absolute and not exist yet; its parents are made
    /// as needed.
    pub fn new(dir: PathBuf) -> Result<Self, QuadError> {
        if !dir.is_absolute() {
            return Err(QuadError::Io(format!(
                "job folder {} is not absolute",
                dir.display()
            )));
        }
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, &e))?;
        }
        fs::create_dir(&dir).map_err(|e| io(&dir, &e))?;
        Ok(Self { dir })
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Nothing to report to: a folder left behind is in the session's scratch.
        fs::remove_dir_all(&self.dir).unwrap_or_default();
    }
}

/// An input or output error on `path`.
pub(crate) fn io(path: &Path, e: &std::io::Error) -> QuadError {
    QuadError::Io(format!("{}: {e}", path.display()))
}

/// Runs steps in one job, counting ticks across them.
pub(crate) struct Runner<'a> {
    pub program: &'a Program,
    pub job: &'a Path,
    pub cancelled: &'a dyn Fn() -> bool,
    pub ticks: u32,
}

impl Runner<'_> {
    /// Runs step `n`, program `exe` with `args` in the job folder, its output to `log` there,
    /// until it exits, is cancelled or reaches the time limit. Its exit code means nothing (the
    /// program exits 0 on failure); the caller judges the files it left. Ended by a signal it is
    /// refused, after one more run when the signal was SIGKILL and neither cancel nor the time
    /// limit has come.
    pub(crate) fn step(
        &mut self,
        n: u8,
        exe: &str,
        args: &[&OsStr],
        log: &str,
    ) -> Result<(), QuadError> {
        let mut signal = self.run(exe, args, log)?;
        if signal == Some(SIGKILL) {
            self.halt()?;
            signal = self.run(exe, args, log)?;
        }
        match signal {
            None => Ok(()),
            Some(signal) => Err(QuadError::Killed {
                step: n,
                signal,
                tail: self.tail(log),
            }),
        }
    }

    /// One run of `exe`, as [`Runner::step`] says: the signal that ended it, if one did.
    fn run(&mut self, exe: &str, args: &[&OsStr], log: &str) -> Result<Option<i32>, QuadError> {
        let path = self.job.join(log);
        let out = File::create(&path).map_err(|e| io(&path, &e))?;
        let err = out.try_clone().map_err(|e| io(&path, &e))?;
        let program = program_in(&self.program.dir, exe);
        let mut command = if let Some(bwrap) = &self.program.bwrap {
            let mut c = Command::new(bwrap);
            c.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"])
                .args(["--tmpfs", "/tmp", "--bind"])
                .args([self.job, self.job])
                .args([
                    "--unshare-all",
                    "--die-with-parent",
                    "--new-session",
                    "--chdir",
                ])
                .arg(self.job)
                .arg(&program);
            c
        } else {
            let mut c = Command::new(&program);
            c.current_dir(self.job);
            c
        };
        let kept = KEPT.iter().filter_map(|&k| Some((k, std::env::var_os(k)?)));
        command
            .args(args)
            .env_clear()
            .envs(kept)
            .stdin(Stdio::null());
        let mut child = (command.stdout(out).stderr(err).spawn())
            .map_err(|e| QuadError::Io(format!("could not start {exe}: {e}")))?;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| io(&program, &e))? {
                return Ok(self.signal(status));
            }
            if let Err(halt) = self.halt() {
                child.kill().unwrap_or_default();
                child.wait().map(drop).unwrap_or_default();
                return Err(halt);
            }
            std::thread::sleep(TICK);
            self.ticks += 1;
        }
    }

    /// Cancel or the time limit, if either has come.
    fn halt(&self) -> Result<(), QuadError> {
        if (self.cancelled)() {
            Err(QuadError::Cancelled)
        } else if self.ticks >= self.program.limit {
            Err(QuadError::TimedOut)
        } else {
            Ok(())
        }
    }

    /// The signal that ended a run with `status`, if one did. Inside bubblewrap the program's
    /// death comes back as bubblewrap's exit code: 128 plus the signal.
    fn signal(&self, status: ExitStatus) -> Option<i32> {
        let code = status.code().filter(|_| self.program.sandboxed());
        killed_by(status).or(code.filter(|c| (129..=192).contains(c)).map(|c| c - 128))
    }

    /// The last lines a step printed to `log`, for a refusal.
    pub(crate) fn tail(&self, log: &str) -> String {
        let text = fs::read(self.job.join(log)).unwrap_or_default();
        let text = String::from_utf8_lossy(&text);
        let lines: Vec<&str> = (text.lines().map(str::trim))
            .filter(|l| !l.is_empty())
            .collect();
        let last = &lines[lines.len().saturating_sub(4)..];
        let short = last.iter().map(|l| l.chars().take(160).collect::<String>());
        short.collect::<Vec<_>>().join(" | ")
    }
}
