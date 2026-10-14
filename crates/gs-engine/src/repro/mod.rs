//! Bug bundles (decision 0031): writing a `.gsrepro` folder (the format is `gs_protocol`'s
//! `repro` module) from the flight recorder, the command log's last lines in memory.
//!
//! A bundle starts the way recovery does: from the newest autosave of the current log run and
//! the lines the run logged after it (`file::newest_autosave`, recovery's own code). Without
//! one it starts where the run started: the empty scene, or the file the run's first line
//! opened, with every line of the run, from the recorder when it holds the whole run, else
//! from the run's log file. Taking no snapshot of the scene costs no memory (decision 0031,
//! section 3). Two things write one: `repro.save`, and a command that panics in a session that
//! writes a log file (`Engine::bundle_panic`). The `gs` client replays, minimizes and checks
//! them. A bundle carries a copy of every file its lines read (`files`).

mod files;
mod save;

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use gs_protocol::{
    BUNDLE_FORMAT, Bundle, BundleInfo, Catalog, CommandError, ErrorCode, LogRecord, Predicate,
    Source, Start, bundle_file, free_path, method, panic_message,
};
use serde_json::json;

use crate::engine::{Call, EngineConfig, Reply};
use crate::file::{FileState, newest_autosave};
use crate::log::LogLine;
use crate::ui::WindowHost;
use crate::{Engine, file};

/// How a panic's message starts in the `E_INTERNAL` error `run_guarded` makes of it.
pub(crate) use gs_protocol::PANICKED;
/// How many of the recorder's lines `log-tail.json` keeps.
const TAIL_LINES: usize = 50;

/// What a bundle is made from.
pub(crate) struct Draft<'a> {
    /// The recorder's lines.
    pub recent: &'a VecDeque<LogLine>,
    /// The log's current run and how many records it holds.
    pub at: (u64, u64),
    /// Where the session's log file and autosaves go, and its seed.
    pub config: &'a EngineConfig,
    /// The scene's file, whose autosave folder may hold a start.
    pub file: &'a FileState,
    pub catalog: &'a Catalog,
    /// Calls to add after the recorded lines.
    pub then: Vec<LogRecord>,
    pub predicate: Predicate,
    pub source: Source,
    /// The window's picture, as PNG.
    pub screenshot: Option<Vec<u8>>,
}

/// Writes the bundle folder `folder`, which must not exist yet (its parent must), and returns
/// its header.
pub(crate) fn write(folder: &Path, draft: Draft) -> io::Result<BundleInfo> {
    fs::create_dir(folder)?;
    let (lines, start) = match from_autosave(&draft, folder)? {
        Some(found) => found,
        None => match run_lines(&draft) {
            (lines, Start::Empty) => opened_by(lines, folder)?,
            missing => missing,
        },
    };
    let then = (draft.then.iter()).map(|r| serde_json::to_string(r).unwrap_or_default());
    let lines: Vec<String> = lines.into_iter().chain(then).collect();
    let (lines, unread) = files::carry(lines, folder)?;
    let note = if unread.is_empty() {
        String::new()
    } else {
        let unread = unread.join(", ");
        format!("lines read files that could not be copied in, left at their paths: {unread}")
    };
    if let Some(png) = &draft.screenshot {
        fs::write(folder.join(bundle_file::SCREENSHOT), png)?;
    }
    let skip = draft.recent.len().saturating_sub(TAIL_LINES);
    let tail: Vec<&LogLine> = draft.recent.iter().skip(skip).collect();
    fs::write(folder.join(bundle_file::TAIL), json_text(&tail))?;
    let info = BundleInfo {
        gigasculpt_repro: BUNDLE_FORMAT,
        engine_version: draft.catalog.engine_version.clone(),
        catalog_hash: draft.catalog.hash.clone(),
        seed: draft.config.seed,
        start,
        predicate: draft.predicate,
        source: draft.source,
        lines: lines.len() as u64,
        note,
    };
    let bundle = Bundle { info, lines };
    bundle.write(folder)?;
    Ok(bundle.info)
}

/// The newest autosave of the current log run as `start.gss`, and the lines the run logged
/// after it, as recovery would replay them; `None` when the run has no autosave.
fn from_autosave(draft: &Draft, folder: &Path) -> io::Result<Option<(Vec<String>, Start)>> {
    let (Some(config), Some(log)) = (&draft.config.autosave, &draft.config.log_path) else {
        return Ok(None);
    };
    let Some((autosave, after)) = newest_autosave(config, draft.file, log, draft.at.0) else {
        return Ok(None);
    };
    let Ok(hash) = file::content_hash(&autosave.display().to_string()) else {
        return Ok(None);
    };
    link_or_copy(&autosave, &folder.join(bundle_file::START))?;
    let lines = (after.iter()).map(|r| serde_json::to_string(r).unwrap_or_default());
    Ok(Some((lines.collect(), Start::File { hash })))
}

/// The current run's lines as the log file has them, and `Start::Empty`, or the lines the
/// recorder still has and why they cannot be replayed.
fn run_lines(draft: &Draft) -> (Vec<String>, Start) {
    let (run, records) = draft.at;
    let held: Vec<String> = (draft.recent.iter())
        .filter(|line| line.run == run)
        .map(|line| line.record.to_string())
        .collect();
    if held.len() as u64 == records {
        return (held, Start::Empty);
    }
    // The run's file is its header, then every record of the run.
    let log_file = draft.config.log_path.as_deref();
    if let Some(text) = log_file.and_then(|path| fs::read_to_string(path).ok()) {
        let lines = text.lines().skip(1).filter(|l| !l.trim().is_empty());
        return (lines.map(ToString::to_string).collect(), Start::Empty);
    }
    let why = format!(
        "the log run has {records} lines, the flight recorder keeps the last {}, and the \
         session writes no log file",
        held.len()
    );
    (held, Start::Missing { why })
}

/// A run that began by opening a scene file: the file goes in as `start.gss` and its open line
/// goes, while the file still holds what the line opened.
fn opened_by(lines: Vec<String>, folder: &Path) -> io::Result<(Vec<String>, Start)> {
    let first = lines
        .first()
        .and_then(|l| serde_json::from_str::<LogRecord>(l).ok());
    let Some(open) = first.filter(|r| r.command == "file.open") else {
        return Ok((lines, Start::Empty));
    };
    let (Some(path), Some(hash)) = (open.args["path"].as_str(), open.args["hash"].as_str()) else {
        return Ok((lines, Start::Empty));
    };
    if file::content_hash(path).ok().as_deref() != Some(hash) {
        let why =
            format!("the log run opened {path}, and that file no longer holds what it opened");
        return Ok((lines, Start::Missing { why }));
    }
    link_or_copy(Path::new(path), &folder.join(bundle_file::START))?;
    let hash = hash.to_string();
    Ok((lines.into_iter().skip(1).collect(), Start::File { hash }))
}

/// A link costs nothing for a big scene, and is safe: saves and autosaves write a new file
/// rather than change one.
fn link_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    if fs::hard_link(from, to).is_err() {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// A call as a log line, the way the log would have written it.
pub(crate) fn as_record(command: &str, args: serde_json::Value) -> LogRecord {
    LogRecord {
        seq: None,
        client: None,
        command: command.into(),
        args,
        hash: None,
    }
}

fn json_text(value: &impl serde::Serialize) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.push('\n');
    text
}

/// The window's picture as PNG, or `None` without a window. A renderer that panics here gives
/// no picture rather than a second panic.
pub(crate) fn screenshot(
    take: impl FnOnce() -> Option<Result<crate::ui::Screenshot, String>>,
) -> Option<Vec<u8>> {
    let shot = catch_unwind(AssertUnwindSafe(take)).ok().flatten()?;
    shot.ok().map(|s| s.png)
}

impl Engine {
    /// A call whose command panicked writes a bug bundle beside the session's log file, ending
    /// with the call as it was sent, and its error's hint says where. A session that writes no
    /// log file writes no bundle: a bundle goes only where the caller already keeps the log.
    pub(crate) fn bundle_panic(&self, call: &Call, reply: Reply) -> Reply {
        let Err((seq, mut err)) = reply else {
            return reply;
        };
        let message = panic_message(err.code, &err.message);
        let (Some(message), Some(log)) = (message, self.config.log_path.as_deref()) else {
            return Err((seq, err));
        };
        let folder = panic_folder(log, seq);
        let failed = if call.batch {
            as_record(method::BATCH, json!({ "commands": call.steps }))
        } else {
            let step = &call.steps[0];
            as_record(&step.command, step.args.clone())
        };
        let draft = Draft {
            recent: self.log.recent(),
            at: self.log.position(),
            config: &self.config,
            file: &self.state.file,
            catalog: &self.registry.catalog,
            then: vec![failed],
            predicate: Predicate::Panic {
                contains: message.to_string(),
            },
            source: Source::Panic,
            screenshot: screenshot(|| self.state.window.with(WindowHost::screenshot)),
        };
        let said = match write(&folder, draft) {
            Ok(_) => format!(
                "a bug bundle was written to {}; `gs repro minimize` shrinks it",
                folder.display()
            ),
            Err(e) => format!("writing a bug bundle to {} failed: {e}", folder.display()),
        };
        err.hint = Some(match err.hint.take() {
            Some(hint) => format!("{hint}; {said}"),
            None => said,
        });
        Err((seq, err))
    }
}

/// `<log stem>-panic-<seq>.gsrepro` beside the log, numbered on if that exists.
fn panic_folder(log: &Path, seq: Option<u64>) -> PathBuf {
    let stem = log.file_stem().unwrap_or_default().to_string_lossy();
    let seq = seq.map_or_else(|| "call".into(), |s| s.to_string());
    free_path(&log.with_file_name(format!("{stem}-panic-{seq}.gsrepro")))
}

/// The error a bundle that cannot be written gives a command.
pub(crate) fn unwritable(folder: &Path, e: &io::Error) -> CommandError {
    crate::errors::error(
        ErrorCode::Precondition,
        format!("cannot write the bug bundle {}: {e}", folder.display()),
    )
}
