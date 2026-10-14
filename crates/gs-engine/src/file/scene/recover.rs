//! Crash recovery (decision 0016, section 11): `file.recovery` lists crashed sessions (a marker
//! whose lock nobody holds) and deletes one on request; `file.recover` opens a crashed
//! session's newest autosave and replays the log lines recorded after it, so the scene comes
//! back to the last command, not only to the last autosave.
//!
//! A replay is bit-safe on the same engine and catalog (ARCHITECTURE 8); when the log was
//! written by another, or is gone, the offer is the autosave alone and says why. A running
//! session holds a lock on the file beside its marker (`<session>-<pid>.lock`), which the
//! system lets go of when its process ends, however it ends; so a session counts as crashed
//! only once its program no longer runs, on every system, and a process number handed on to
//! another program changes nothing.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use gs_io::scene;
use gs_protocol::{Catalog, CommandError, Cost, ErrorCode, Kind, LogRecord, ScriptLine};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::autosave::{Marker, StateFolder, copies};
use super::model::{SessionSection, load, read, unreadable};
use super::{FileState, Landing, Opened};
use crate::errors::error;
use crate::view::saved::{ReopenedReferences, reopen};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// A crashed session that can be recovered.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Candidate {
    /// What `file.recover` and `file.recovery`'s `discard` call it: `<session>-<pid>`.
    pub id: String,
    pub session: String,
    /// The newest autosave, if it wrote one.
    pub autosave: Option<String>,
    /// How many autosaves of its scene there are.
    pub autosaves: usize,
    /// The scene's own file, if it had one.
    pub file: Option<String>,
    /// Log lines recorded after the newest autosave; `null` when they cannot be read.
    pub lines_after: Option<u64>,
    /// True if those lines replay bit for bit: the log is there, and this engine and catalog
    /// wrote it.
    pub replayable: bool,
    /// What `file.recover` does by default, in words.
    pub offer: String,
}

/// Crashed sessions.
#[derive(Serialize, JsonSchema)]
pub struct Recovery {
    pub candidates: Vec<Candidate>,
    /// The one `discard` deleted, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discarded: Option<String>,
}

/// List the sessions that crashed (killed, or the machine went down) with work to recover; or,
/// with `discard`, delete one's autosaves and marker first.
///
/// Each candidate names its newest autosave and how many command-log lines came after it;
/// `file.recover` brings the scene back from them. The window asks the same at start. Needs
/// autosave on (it is in the window and in `gigasculpt serve`).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRecovery {
    /// A candidate's id: delete its autosaves and marker, so it is no longer offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discard: Option<String>,
}

impl Command for FileRecovery {
    const NAME: &'static str = "file.recovery";
    const KIND: Kind = Kind::Query;
    type Output = Recovery;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "list crashed sessions (refused here: autosave is off)",
                json!({}),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Recovery, CommandError> {
        let config = autosave_on(cx)?;
        let mut discarded = None;
        if let Some(id) = &self.discard {
            let (marker_file, marker) = crashed(&config, id)?;
            if let Some(dir) = &marker.autosave {
                for (_, path) in copies(Path::new(dir)) {
                    fs::remove_file(path).ok();
                }
                fs::remove_dir(dir).ok();
            }
            forget(&marker_file);
            discarded = Some(id.clone());
        }
        let candidates = markers(&config)
            .into_iter()
            .map(|(file, m)| describe(&file, &m, cx.catalog))
            .collect();
        Ok(Recovery {
            candidates,
            discarded,
        })
    }
}
register_command!(FileRecovery, "List crashed sessions");

/// Recover a crashed session's scene: open its newest autosave, then replay the command-log
/// lines recorded after it, in the background.
///
/// Brings the scene back to its last command when the log is there and this engine wrote it
/// (`file.recovery` says); otherwise to the last autosave, or pass `replay: false` for that.
/// The replay stops at the first line that fails, for example one whose input file changed,
/// and keeps the scene as it was there. The scene keeps its own file name but counts as
/// unsaved. The log records this call as the open of the autosave. Refused while the scene has
/// unsaved changes, unless `discard_changes` is on.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRecover {
    /// The candidate to recover, as `file.recovery` names it.
    #[schemars(length(min = 1))]
    pub session: String,
    /// Replay the log after the autosave. Left out: when it replays bit for bit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<bool>,
    /// Recover even though the scene has unsaved changes, which are lost.
    #[serde(default)]
    pub discard_changes: bool,
}

/// What a recovery brought back.
#[derive(Serialize, JsonSchema)]
pub struct Recovered {
    /// The autosave opened.
    pub autosave: String,
    /// The Parts it holds.
    pub parts: Vec<String>,
    /// Optional data this version does not know or cannot read, and left out, as `file.open`
    /// reports it: a palette that does not read, for one (decision 0043, section 7).
    pub skipped: Vec<String>,
    /// Log lines it set out to replay after the autosave.
    pub lines: u64,
    /// How many ran (filled in when the replay ends).
    pub replayed: u64,
    /// Why the replay stopped early, if it did.
    pub replay_stopped: Option<String>,
    /// The Reference Views the autosave keeps, open now (before the replay).
    pub references: ReopenedReferences,
}

impl Command for FileRecover {
    const NAME: &'static str = "file.recover";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    type Output = Recovered;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "recover a crashed session (refused here: autosave is off)",
                json!({ "session": "main-12345" }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::replaceable(cx, self.discard_changes)
    }

    fn run(self, cx: &mut Ctx) -> Result<Recovered, CommandError> {
        let config = autosave_on(cx)?;
        let (marker_file, marker) = crashed(&config, &self.session)?;
        let c = describe(&marker_file, &marker, cx.catalog);
        let autosave = c.autosave.clone().map(PathBuf::from).ok_or_else(|| {
            let log = marker.log.as_deref().unwrap_or("its log");
            CommandError {
                hint: Some(format!("replay {log} into a fresh session with gs run")),
                ..error(
                    ErrorCode::Precondition,
                    format!("session {} wrote no autosave", c.id),
                )
            }
        })?;
        let replay = self.replay.unwrap_or(c.replayable);
        let lines = if replay {
            tail(&autosave, &marker)?
        } else {
            Vec::new()
        };
        cx.progress(None, format!("reading {}", autosave.display()));
        let file = read(&autosave).map_err(|e| unreadable(&autosave, &e))?;
        let mut skipped = file.skipped.clone();
        let mut loaded = load(file);
        if let Some(r) = loaded.refused.first() {
            return Err(error(
                ErrorCode::Damaged,
                format!(
                    "the autosave's part '{}' cannot be read: {}",
                    r.part, r.reason
                ),
            ));
        }
        let hash = crate::file::content_hash(&autosave.display().to_string())
            .map_err(|e| error(ErrorCode::Precondition, e.to_string()))?;
        let shown = autosave.display().to_string();
        let log_as = json!({ "path": shown, "discard_changes": true, "hash": hash });
        let session = (loaded.session.seed).zip(loaded.session.counter);
        let saved = loaded.session.references.take();
        let (references, report) = reopen(cx, saved, &mut loaded.kept, &mut skipped);
        let palette = loaded.scene.palette.take();
        let (palette, unread) = crate::material::scene::opened(palette, loaded.session.material);
        skipped.extend(unread);
        let out = Recovered {
            autosave: shown,
            parts: loaded.names,
            skipped,
            lines: lines.len() as u64,
            replayed: 0,
            replay_stopped: None,
            references: report,
        };
        cx.land(Landing::Opened(Box::new(Opened {
            doc: loaded.doc,
            path: loaded.session.file.map(PathBuf::from),
            clean: false,
            forget: Some(marker_file),
            session,
            id: loaded.scene.id,
            format: Some(scene::FORMAT),
            rev: cx.rev,
            discard: self.discard_changes,
            replay: lines,
            log_as: Some(("file.open".into(), log_as)),
            references,
            recent: None,
            background: crate::Background::saved(loaded.scene.background),
            palette: Some(palette),
        })));
        Ok(out)
    }
}
register_command!(FileRecover, "Recover session");

fn autosave_on(cx: &Ctx) -> Result<StateFolder, CommandError> {
    cx.config.autosave.clone().ok_or_else(|| {
        error(
            ErrorCode::Precondition,
            "autosave is off in this session, so it keeps no recovery files",
        )
    })
}

/// Takes the lock beside `marker` for as long as the file it gives lives, so the session
/// marked there counts as running; `None` when another holds it, or it cannot be made.
pub(crate) fn hold(marker: &Path) -> Option<File> {
    crate::log::take_lock(&marker.with_extension("lock"))
}

/// True once the session marked at `marker` has stopped: nobody holds the lock beside it, or
/// there is none.
fn stopped(marker: &Path) -> bool {
    !crate::log::lock_held(&marker.with_extension("lock"))
}

/// Removes `marker` and then the lock beside it, so no one sees the marker without its lock.
pub(crate) fn forget(marker: &Path) {
    fs::remove_file(marker).ok();
    fs::remove_file(marker.with_extension("lock")).ok();
}

/// Every crashed session's marker, by file name.
fn markers(config: &StateFolder) -> Vec<(PathBuf, Marker)> {
    let Ok(entries) = fs::read_dir(config.state_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Marker)> = entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            path.extension().filter(|x| *x == "json")?;
            let marker: Marker = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            stopped(&path).then_some((path, marker))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The crashed session `id`, or why there is none.
fn crashed(config: &StateFolder, id: &str) -> Result<(PathBuf, Marker), CommandError> {
    let all = markers(config);
    let names = || all.iter().map(|(f, _)| stem(f)).collect::<Vec<_>>();
    match all.iter().find(|(f, _)| stem(f) == id) {
        Some(found) => Ok(found.clone()),
        None => Err(CommandError {
            suggestions: names(),
            hint: Some("file.recovery lists the sessions to recover".into()),
            ..error(ErrorCode::NotFound, format!("no crashed session {id}"))
        }),
    }
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A crashed session as a candidate: its newest autosave and the log after it.
fn describe(file: &Path, m: &Marker, catalog: &Catalog) -> Candidate {
    let all = m
        .autosave
        .as_deref()
        .map(|d| copies(Path::new(d)))
        .unwrap_or_default();
    let newest = all.last().map(|c| c.1.clone());
    let session = newest.as_deref().and_then(session_of);
    let log = log_lines(m.log.as_ref(), m.log_run, session.as_ref());
    let same = log.as_ref().is_ok_and(|t| {
        t.header.engine_version == catalog.engine_version && t.header.catalog_hash == catalog.hash
    });
    let lines_after = log.as_ref().ok().map(|t| t.lines.len() as u64);
    let offer = match (&newest, &log, same) {
        (None, _, _) => "nothing to recover from: it wrote no autosave".to_string(),
        (Some(_), Ok(t), true) => {
            let cut = if t.cut {
                "; the last command was cut off when the session stopped, so it is left out"
            } else {
                ""
            };
            let n = t.lines.len();
            format!("the newest autosave plus the {n} commands after it{cut}")
        }
        (Some(_), Ok(_), false) => {
            "the newest autosave alone: another engine wrote the log, so a replay could differ"
                .to_string()
        }
        (Some(_), Err(why), _) => {
            format!("the newest autosave alone: the command log after it cannot be read: {why}")
        }
    };
    Candidate {
        id: stem(file),
        session: m.session.clone(),
        autosave: newest.map(|p| p.display().to_string()),
        autosaves: all.len(),
        file: session.and_then(|s| s.file),
        lines_after,
        replayable: same,
        offer,
    }
}

/// An autosave's session section.
fn session_of(autosave: &Path) -> Option<SessionSection> {
    let seen = scene::inspect(autosave, false).ok()?;
    serde_json::from_value::<SessionSection>(seen.session).ok()
}

/// What recovery reads of a command log.
struct LogTail {
    header: gs_protocol::LogHeader,
    /// The records after the autosave.
    lines: Vec<LogRecord>,
    /// Whether a last record that a crash cut off was left out.
    cut: bool,
}

/// The log's header and the lines it recorded after the autosave, if the log is there and is
/// the run the autosave was written in; else why not, with the system's reason when it could
/// not be read.
fn log_lines(
    log: Option<&String>,
    run: u64,
    session: Option<&SessionSection>,
) -> Result<LogTail, String> {
    let session = session.ok_or("the autosave's session section cannot be read")?;
    let at = session
        .log
        .clone()
        .ok_or("the session kept no command log")?;
    if Some(&at.path) != log || at.run != run {
        return Err(format!(
            "the autosave was not written in {}'s last run",
            at.path
        ));
    }
    let bytes = fs::read(&at.path).map_err(|e| format!("{}: {e}", at.path))?;
    let (whole, cut) = whole_records(&bytes);
    let text = std::str::from_utf8(whole).map_err(|e| format!("{}: {e}", at.path))?;
    let mut header = None;
    let mut records = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        match ScriptLine::parse(line).map_err(|e| format!("{} line {}: {e}", at.path, n + 1))? {
            ScriptLine::Header(h) if header.is_none() => header = Some(h),
            ScriptLine::Header(_) => return Err(format!("{} holds a second header", at.path)),
            ScriptLine::Record(r) => records.push(r),
        }
    }
    let header = header.ok_or_else(|| format!("{} has no header", at.path))?;
    let before = usize::try_from(at.records).unwrap_or(usize::MAX);
    let lines = records.split_off(before.min(records.len()));
    Ok(LogTail { header, lines, cut })
}

/// `bytes` without a last line that has no newline and does not parse, and whether there was
/// one: a record a crash or power loss cut off in the middle of its write. A record counts once
/// its newline is written (decision 0016, section 14), so that line alone is dropped, as
/// write-ahead logs (SQLite's, PostgreSQL's) drop a torn last record; a line that does not
/// parse anywhere else makes the log unreadable.
fn whole_records(bytes: &[u8]) -> (&[u8], bool) {
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let (head, last) = bytes.split_at(end);
    let last = std::str::from_utf8(last);
    if last.is_ok_and(|l| l.trim().is_empty() || ScriptLine::parse(l).is_ok()) {
        (bytes, false)
    } else {
        (head, true)
    }
}

/// The lines to replay after `autosave`, or why there are none to read.
fn tail(autosave: &Path, m: &Marker) -> Result<Vec<LogRecord>, CommandError> {
    let session = session_of(autosave);
    log_lines(m.log.as_ref(), m.log_run, session.as_ref())
        .map(|t| t.lines)
        .map_err(|why| CommandError {
            hint: Some("pass replay: false to recover the autosave alone".into()),
            ..error(
                ErrorCode::Precondition,
                format!("the command log after the autosave cannot be read: {why}"),
            )
        })
}

/// Where a bug bundle of scene `file` starts when the current run of log `log` has an
/// autosave (decision 0031, section 3), the way recovery starts: the newest autosave and the
/// lines the run logged after it.
pub(crate) fn newest_autosave(
    config: &StateFolder,
    file: &FileState,
    log: &Path,
    run: u64,
) -> Option<(PathBuf, Vec<LogRecord>)> {
    let folder = super::autosave::folder(config, &file.name(), file.id.as_ref()?);
    let (_, newest) = copies(&folder).pop()?;
    let log = log.display().to_string();
    let tail = log_lines(Some(&log), run, session_of(&newest).as_ref()).ok()?;
    Some((newest, tail.lines))
}
