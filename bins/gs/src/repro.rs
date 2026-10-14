//! `gs repro` (decision 0031): replays a bug bundle in a fresh headless engine and says whether
//! its bug happens (`check`), shrinks it to the lines the bug needs (`minimize`), and keeps a
//! fixed bug's bundle as a test (`to-test`). `gs run --repro-out` writes a bundle when a verify
//! log diverges (`write_diverged`).
//!
//! Each replay starts its own engine, the program `gs spawn` starts, on its own socket, with its
//! log in a work folder beside the bundle: a panic bundle that engine writes then links the start
//! scene instead of copying it. The engine is killed and the folder removed when the replay ends.
//! Its working folder is the bundle folder, so the lines read the bundle's copies of their files
//! (`files/`), whose paths are relative to it.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use clap::Subcommand;
use gs_protocol::{
    BUNDLE_FORMAT, Bundle, BundleInfo, LogHeader, LogRecord, Predicate, Role, Source, Start,
    UnixStream, bundle_file, free_path, panic_message,
};
use serde_json::json;

use crate::connection::Connection;
use crate::failure::{Failure, because};
use crate::output::checked;
use crate::session::{Detach, engine_program};

/// How long a replay's engine may take to start: a busy machine can take well over 10 s.
const START: Duration = Duration::from_secs(180);
/// How long a line may take when the bug is not a hang.
const LINE: Duration = Duration::from_secs(600);
/// How often to look again while an engine starts.
const POLL: Duration = Duration::from_millis(25);

#[derive(Subcommand)]
pub enum Action {
    /// Replay a bundle in a fresh engine and say whether its bug happens: `happens`, `does not
    /// happen`, or `unknown` when a line fails some other way first.
    Check {
        /// The bundle folder.
        bundle: PathBuf,
    },
    /// Shrink a bundle to the command lines its bug needs, by delta debugging, into a new
    /// bundle.
    Minimize {
        /// The bundle folder.
        bundle: PathBuf,
        /// The new bundle folder. Default: `<name>-min.gsrepro` beside the bundle.
        #[arg(long)]
        out: Option<PathBuf>,
        /// How many replays run at once.
        #[arg(long, default_value_t = 4)]
        jobs: usize,
    },
    /// Keep a fixed bug's bundle as a test: check that its bug no longer happens and nothing
    /// else stops the replay, then copy it (without its screenshot and log tail) into the
    /// folder the `gs` tests replay.
    ToTest {
        /// The bundle folder.
        bundle: PathBuf,
        /// The test folder.
        #[arg(long, default_value = "tests/repros")]
        into: PathBuf,
    },
}

/// Runs `gs repro`.
pub fn run(action: Action, json: bool, out: &mut Vec<String>) -> Result<(), Failure> {
    match action {
        Action::Check { bundle } => {
            let found = read(&bundle)?;
            let verdict = replay(&bundle, &found, &found.lines)?;
            say(&found.info, &verdict, json, out);
            Ok(())
        }
        Action::Minimize {
            bundle,
            out: to,
            jobs,
        } => minimize(&bundle, to, jobs.max(1), out),
        Action::ToTest { bundle, into } => to_test(&bundle, &into, out),
    }
}

/// What one replay found.
struct Verdict {
    /// True if the predicate held.
    happens: bool,
    /// True if a line failed some other way than the predicate names, or the predicate could
    /// not be checked: the bug may be hidden rather than gone.
    blocked: bool,
    /// The line it held at, else the last line that ran (counting from 1).
    line: usize,
    /// What happened, in words.
    what: String,
    /// The bundle came from another engine or catalog, so the replay may differ.
    warning: Option<String>,
}

/// Reads a bundle that can be replayed.
fn read(folder: &Path) -> Result<Bundle, Failure> {
    let found = Bundle::read(folder).map_err(Failure::Usage)?;
    if let Start::Missing { why } = &found.info.start {
        return Err(Failure::Refused(format!(
            "{} cannot be replayed: its start scene is missing ({why})",
            folder.display()
        )));
    }
    Ok(found)
}

fn say(info: &BundleInfo, verdict: &Verdict, json: bool, out: &mut Vec<String>) {
    let predicate = info.predicate.describe();
    if json {
        let mut said = json!({
            "happens": verdict.happens,
            "blocked": verdict.blocked,
            "line": verdict.line,
            "what": verdict.what,
            "predicate": predicate,
        });
        if let Some(warning) = &verdict.warning {
            said["warning"] = json!(warning);
        }
        out.push(said.to_string());
        return;
    }
    let happens = match (verdict.happens, verdict.blocked) {
        (true, _) => "happens",
        (false, true) => "unknown",
        (false, false) => "does not happen",
    };
    out.push(format!("bug: {happens}"));
    out.push(format!("predicate: {predicate}"));
    out.push(format!("line: {}", verdict.line));
    out.push(format!("what: {}", verdict.what));
    out.extend(verdict.warning.iter().map(|w| format!("warning: {w}")));
}

/// One headless engine for one replay. Dropping it kills the engine and removes its work
/// folder.
struct Replayer {
    child: Child,
    work: PathBuf,
    socket: PathBuf,
}

impl Replayer {
    fn start(bundle: &Path, seed: u64) -> Result<Self, Failure> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let bundle = &std::path::absolute(bundle)
            .map_err(|e| Failure::Usage(format!("bad bundle path: {e}")))?;
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let name = bundle.file_name().unwrap_or_default().to_string_lossy();
        let work = bundle.with_file_name(format!(".{name}.replay-{pid}-{n}"));
        let unmade = |e| Failure::Unreachable(format!("cannot make {}: {e}", work.display()));
        fs::create_dir_all(&work).map_err(unmade)?;
        let stderr = File::create(work.join("engine.stderr")).map_err(unmade)?;
        // Short, so it stays under the 108-byte limit for socket paths.
        let socket = std::env::temp_dir().join(format!("gs-repro-{pid}-{n}.sock"));
        let engine = engine_program();
        let child = Command::new(&engine)
            .args(["serve", "--headless", "--session", &format!("repro-{n}")])
            .args(["--seed", &seed.to_string()])
            .arg("--socket")
            .arg(&socket)
            .arg("--log")
            .arg(work.join("replay.jsonl"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .detached()
            .current_dir(bundle)
            .spawn();
        let child = child.map_err(because(
            Failure::Unreachable,
            format!("cannot start {}", engine.display()),
        ))?;
        let mut me = Self {
            child,
            work,
            socket,
        };
        for _ in 0..START.as_millis().div_ceil(POLL.as_millis()) {
            if let Ok(Some(status)) = me.child.try_wait() {
                let said = fs::read_to_string(me.work.join("engine.stderr")).unwrap_or_default();
                return Err(Failure::Unreachable(format!(
                    "the replay's engine exited ({status}): {}",
                    said.trim()
                )));
            }
            if UnixStream::connect(&me.socket).is_ok() {
                return Ok(me);
            }
            thread::sleep(POLL);
        }
        Err(Failure::Timeout(format!(
            "the replay's engine did not start within {} s",
            START.as_secs()
        )))
    }
}

impl Drop for Replayer {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        fs::remove_file(&self.socket).ok();
        fs::remove_dir_all(&self.work).ok();
    }
}

/// Replays `lines` from the bundle's start in a fresh engine, stopping at the first line that
/// fails, and checks the bundle's predicate.
fn replay(folder: &Path, bundle: &Bundle, lines: &[String]) -> Result<Verdict, Failure> {
    let info = &bundle.info;
    let engine = Replayer::start(folder, info.seed)?;
    let patience = match info.predicate {
        Predicate::Hang { seconds } => Duration::from_secs(seconds),
        _ => LINE,
    };
    let mut connection = Connection::open(&engine.socket, Role::Agent, patience)?;
    let catalog = connection.catalog()?;
    let differs =
        catalog.engine_version != info.engine_version || catalog.hash != info.catalog_hash;
    let warning = differs.then(|| {
        format!(
            "the bundle was written by engine {} (catalog {}), the replay ran {} (catalog {})",
            info.engine_version, info.catalog_hash, catalog.engine_version, catalog.hash
        )
    });
    if let Start::File { hash } = &info.start {
        let start = std::path::absolute(folder.join(bundle_file::START))
            .map_err(|e| Failure::Usage(format!("bad bundle path: {e}")))?;
        let args = json!({ "path": start, "discard_changes": true, "hash": hash });
        checked(&connection.run("file.open", args)?)?;
    }
    let verdict = run_lines(&mut connection, &info.predicate, lines)?;
    Ok(Verdict { warning, ..verdict })
}

/// What a replay found: whether the predicate held, whether something else stopped it, at
/// which line, and what happened.
fn seen(happens: bool, blocked: bool, line: usize, what: String) -> Verdict {
    Verdict {
        happens,
        blocked,
        line,
        what,
        warning: None,
    }
}

/// Runs the lines and checks `predicate`.
fn run_lines(
    connection: &mut Connection,
    predicate: &Predicate,
    lines: &[String],
) -> Result<Verdict, Failure> {
    for (line, text) in (1..).zip(lines) {
        let record: LogRecord = serde_json::from_str(text).map_err(|e| {
            Failure::Usage(format!("line {line} of {}: {e}", bundle_file::COMMANDS))
        })?;
        let command = &record.command;
        let request = record.request().map_err(Failure::Usage)?;
        let response = match (connection.send(request), predicate) {
            (Ok(response), _) => response,
            (Err(Failure::Timeout(_)), Predicate::Hang { seconds }) => {
                let what = format!("line {line} ({command}) did not answer within {seconds} s");
                return Ok(seen(true, false, line, what));
            }
            (Err(failure), _) => {
                let why = crate::output::failure(&failure).join("; ");
                let what = format!("line {line} ({command}) got no answer: {why}");
                return Ok(seen(false, true, line, what));
            }
        };
        match checked(&response) {
            Err(Failure::Command(err)) => {
                let happens = match predicate {
                    Predicate::Panic { contains } => {
                        panic_message(err.code, &err.message).is_some_and(|m| m.contains(contains))
                    }
                    Predicate::Error { code, contains } => {
                        err.code == *code && err.message.contains(contains)
                    }
                    _ => false,
                };
                let what = format!("line {line} ({command}) failed: {}", err.message);
                return Ok(seen(happens, !happens, line, what));
            }
            Err(failure) => return Err(failure),
            Ok(()) => {}
        }
        if let Predicate::HashMismatch { line: at, expected } = predicate
            && *at == line as u64
        {
            let got = scene_hash(connection)?;
            let what = format!("the scene hash after line {line} is {got}, expected {expected}");
            return Ok(seen(&got != expected, false, line, what));
        }
    }
    let ran = lines.len();
    let (happens, blocked, what) = match predicate {
        Predicate::MeshDiff { a, b, over } => {
            let response = connection.run("mesh.diff", json!({ "a": a, "b": b }))?;
            let max = response["ok"]["result"]["displacement"]["max"].as_f64();
            match (checked(&response), max) {
                (Err(failure), _) => {
                    let why = crate::output::failure(&failure).join("; ");
                    (false, true, format!("mesh.diff failed: {why}"))
                }
                (Ok(()), Some(max)) => (
                    max > *over,
                    false,
                    format!("{a} and {b} differ by up to {max}"),
                ),
                (Ok(()), None) => (true, false, format!("{a} and {b} differ in topology")),
            }
        }
        Predicate::HashMismatch { line, .. } => (
            false,
            true,
            format!("the bundle has only {ran} lines, fewer than {line}"),
        ),
        _ => (false, false, format!("all {ran} lines ran")),
    };
    Ok(seen(happens, blocked, ran, what))
}

fn scene_hash(connection: &mut Connection) -> Result<String, Failure> {
    let response = connection.run("mesh.hash", json!({}))?;
    checked(&response)?;
    let hash = &response["ok"]["result"]["hash"];
    Ok(hash.as_str().unwrap_or_default().to_string())
}

/// `gs repro minimize`.
fn minimize(
    folder: &Path,
    to: Option<PathBuf>,
    jobs: usize,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let found = read(folder)?;
    let name = folder.file_stem().unwrap_or_default().to_string_lossy();
    let to = to.unwrap_or_else(|| folder.with_file_name(format!("{name}-min.gsrepro")));
    unused(&to)?;
    let first = replay(folder, &found, &found.lines)?;
    if !first.happens {
        return Err(Failure::Refused(format!(
            "the bug does not happen in this bundle ({}), so there is nothing to keep",
            first.what
        )));
    }
    // Lines after the one the bug shows at never ran. A hash mismatch needs every line before
    // its line, since the expected hash describes the scene they make.
    let mut lines = found.lines[..first.line].to_vec();
    let mut replays = 1;
    if !matches!(found.info.predicate, Predicate::HashMismatch { .. }) {
        lines = ddmin(folder, &found, lines, jobs, &mut replays)?;
    }
    let before = found.lines.len();
    let note = format!("minimized from {before} lines by gs repro minimize");
    let info = BundleInfo {
        note: [found.info.note.as_str(), &note]
            .iter()
            .filter(|n| !n.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("; "),
        ..found.info
    };
    let kept = lines.len();
    create(folder, &to, &Bundle { info, lines })?;
    out.push(format!("lines: {before} -> {kept}"));
    out.push(format!("replays: {replays}"));
    out.push(format!("wrote: {}", to.display()));
    Ok(())
}

/// Delta debugging (Zeller and Hildebrandt, 2002): split the lines into `n` chunks, keep any
/// chunk or complement whose replay still shows the bug, else split finer, until no single
/// line can go. Each round takes the first candidate in order that shows it, whatever order
/// the parallel replays finish in.
fn ddmin(
    folder: &Path,
    found: &Bundle,
    mut lines: Vec<String>,
    jobs: usize,
    replays: &mut usize,
) -> Result<Vec<String>, Failure> {
    let mut n = 2;
    while lines.len() >= 2 {
        let chunks = split(&lines, n);
        let mut candidates = chunks.clone();
        if n > 2 {
            candidates.extend((0..n).map(|skip| {
                let rest = chunks.iter().enumerate().filter(|(i, _)| *i != skip);
                rest.flat_map(|(_, chunk)| chunk.iter().cloned()).collect()
            }));
        }
        match first_showing(folder, found, &candidates, jobs, replays)? {
            Some((index, line)) => {
                lines = candidates.swap_remove(index);
                lines.truncate(line);
                n = if index < n { 2 } else { (n - 1).max(2) };
            }
            None if n >= lines.len() => break,
            None => n = (2 * n).min(lines.len()),
        }
    }
    Ok(lines)
}

/// `lines` in `n` runs of nearly equal length, in order.
fn split(lines: &[String], n: usize) -> Vec<Vec<String>> {
    let (each, extra) = (lines.len() / n, lines.len() % n);
    let mut rest = lines;
    (0..n)
        .map(|i| {
            let (chunk, after) = rest.split_at(each + usize::from(i < extra));
            rest = after;
            chunk.to_vec()
        })
        .collect()
}

/// The first candidate in order whose replay shows the bug, and the line it showed at,
/// replaying `jobs` at once.
fn first_showing(
    folder: &Path,
    found: &Bundle,
    candidates: &[Vec<String>],
    jobs: usize,
    replays: &mut usize,
) -> Result<Option<(usize, usize)>, Failure> {
    for (group, batch) in candidates.chunks(jobs).enumerate() {
        let verdicts: Vec<Result<Verdict, Failure>> = thread::scope(|scope| {
            let running: Vec<_> = (batch.iter())
                .map(|lines| scope.spawn(|| replay(folder, found, lines)))
                .collect();
            let lost = || Err(Failure::Protocol("a replay thread panicked".into()));
            running
                .into_iter()
                .map(|r| r.join().unwrap_or_else(|_| lost()))
                .collect()
        });
        *replays += batch.len();
        for (i, verdict) in verdicts.into_iter().enumerate() {
            let verdict = verdict?;
            if verdict.happens {
                return Ok(Some((group * jobs + i, verdict.line)));
            }
        }
    }
    Ok(None)
}

/// `gs repro to-test`.
fn to_test(folder: &Path, into: &Path, out: &mut Vec<String>) -> Result<(), Failure> {
    let found = read(folder)?;
    let verdict = replay(folder, &found, &found.lines)?;
    if verdict.happens {
        return Err(Failure::Refused(format!(
            "the bug still happens ({}); only a fixed bug's bundle becomes a test",
            verdict.what
        )));
    }
    if verdict.blocked {
        return Err(Failure::Refused(format!(
            "the replay stopped before it could tell ({}); such a bundle would test nothing",
            verdict.what
        )));
    }
    let name = folder.file_name().unwrap_or_default();
    fs::create_dir_all(into).map_err(because(
        Failure::Usage,
        format!("cannot make {}", into.display()),
    ))?;
    let to = into.join(name);
    unused(&to)?;
    create(folder, &to, &found)?;
    out.push(format!("wrote: {}", to.display()));
    Ok(())
}

/// A bundle folder name that is free: it ends in `.gsrepro` and does not exist.
fn unused(to: &Path) -> Result<(), Failure> {
    if to.extension().is_none_or(|x| x != "gsrepro") {
        return Err(Failure::Usage(format!(
            "{}: a bundle folder's name ends in .gsrepro",
            to.display()
        )));
    }
    if to.exists() {
        return Err(Failure::Usage(format!(
            "{} already exists; a bundle is never written over",
            to.display()
        )));
    }
    Ok(())
}

/// Writes `bundle` as the new folder `to`, with `from`'s start scene and the files its lines
/// read. Copies, not links: the new bundle may go into the source tree.
fn create(from: &Path, to: &Path, bundle: &Bundle) -> Result<(), Failure> {
    let failed = |e: std::io::Error| Failure::Usage(format!("cannot write {}: {e}", to.display()));
    fs::create_dir(to).map_err(failed)?;
    if matches!(bundle.info.start, Start::File { .. }) {
        fs::copy(from.join(bundle_file::START), to.join(bundle_file::START)).map_err(failed)?;
    }
    let read = from.join(bundle_file::FILES);
    if read.is_dir() {
        copy_folder(&read, &to.join(bundle_file::FILES)).map_err(failed)?;
    }
    bundle.write(to).map_err(failed)
}

/// Copies the folder `from`, and every folder in it, to the new folder `to`.
fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let (path, copy) = (entry.path(), to.join(entry.file_name()));
        if path.is_dir() {
            copy_folder(&path, &copy)?;
        } else {
            fs::copy(&path, &copy)?;
        }
    }
    Ok(())
}

/// Writes the bundle `gs run --repro-out DIR` makes when the last of `lines`, the records of
/// verify log `script` so far, replays to a scene hash other than `expected`: the lines up to
/// it, from the empty scene. Returns the new folder.
pub fn write_diverged(
    dir: &Path,
    script: &Path,
    header: Option<&LogHeader>,
    lines: &[String],
    expected: &str,
) -> Result<PathBuf, String> {
    let stem = script.file_stem().unwrap_or_default().to_string_lossy();
    let at = lines.len();
    let to = free_path(&dir.join(format!("{stem}-diverged-{at}.gsrepro")));
    let info = BundleInfo {
        gigasculpt_repro: BUNDLE_FORMAT,
        engine_version: header.map(|h| h.engine_version.clone()).unwrap_or_default(),
        catalog_hash: header.map(|h| h.catalog_hash.clone()).unwrap_or_default(),
        seed: header.map_or(0, |h| h.seed),
        start: Start::Empty,
        predicate: Predicate::HashMismatch {
            line: at as u64,
            expected: expected.into(),
        },
        source: Source::Verify,
        lines: 0,
        note: format!("gs run {} diverged at line {at}", script.display()),
    };
    let wrote = fs::create_dir(&to).and_then(|()| {
        Bundle {
            info,
            lines: lines.to_vec(),
        }
        .write(&to)
    });
    wrote.map_err(|e| format!("cannot write {}: {e}", to.display()))?;
    Ok(to)
}
