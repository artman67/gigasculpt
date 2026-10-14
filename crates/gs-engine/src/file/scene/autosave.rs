//! Autosave (decision 0016, section 10): two triggers, as the catalog's `interface.autosave`
//! says. Max Duration fires when that long has passed since the last save of any kind; Rest
//! Duration when nobody has called the engine for that long, even right after a save. A
//! recovery that lands fires one at once (`recovered`), because it removes the crashed
//! session's marker and a second crash before either timer would leave nothing to recover
//! (review 0005, m6). Each writes `file.quick_save`'s numbered copy into the scene's autosave
//! folder, as a background job, unless an autosave already holds this revision and palette
//! (its step count, decision 0043, section 7), the scene came from a file unchanged, the pen is
//! down, a job holds the scene (one opening a scene, or the window's long edit), another
//! autosave is still running or the scene is empty.
//!
//! A running session with autosave on keeps a marker file naming its log and its scene's
//! autosave folder; a clean stop removes it. A marker whose process is gone means a crash,
//! which `file.recovery` offers to recover.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gs_protocol::{CmdResponse, Request, Role, RunParams};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::FileState;
use crate::Origin;
use crate::clock::Clock;
use crate::engine::Engine;
use crate::job::{Caller, JobCell, Start, Work};

/// The client name autosave's calls run as.
const AUTOSAVE: &str = "autosave";

/// The session's state folder, and when autosave writes into it. The folder holds the autosave
/// folders, the session markers, autosave's settings and the recent files list.
#[derive(Clone, Debug)]
pub struct StateFolder {
    /// The folder for the autosave folders (`autosave/`), the session markers (`sessions/`),
    /// autosave's settings (`autosave.json`) and the recent files list (`recent.json`):
    /// the state folder (`gs_protocol::state_dir`), never a temporary folder, which a reboot
    /// empties.
    pub state_dir: PathBuf,
    /// Max Duration: since the last save of any kind.
    pub max_duration: Duration,
    /// Rest Duration: without a call from anyone.
    pub rest_duration: Duration,
    /// Copies kept per scene (at most 100).
    pub keep: usize,
}

impl StateFolder {
    /// Autosave in `state_dir` with the settings `file.autosave` kept there, else the
    /// catalog's defaults (10 minutes, 2 minutes, 10 copies).
    pub fn in_folder(state_dir: PathBuf) -> Self {
        let kept = super::autosave_settings::kept(&state_dir);
        let defaults = Self {
            state_dir,
            max_duration: Duration::from_secs(600),
            rest_duration: Duration::from_secs(120),
            keep: 10,
        };
        match kept {
            Some(s) => defaults.with(s),
            None => defaults,
        }
    }

    /// The defaults in the state folder (`gs_protocol::state_dir`); `None` when there is none.
    pub fn standard() -> Option<Self> {
        gs_protocol::state_dir().ok().map(Self::in_folder)
    }
}

/// Autosave's bookkeeping. Its settings are the engine's (`EngineConfig::autosave`), which
/// `file.autosave` changes.
pub(crate) struct Autosave {
    /// Clock time of the last save of any kind, or of the last autosave attempt.
    last_save: Duration,
    /// Clock time of the last call from anyone but autosave.
    last_input: Duration,
    /// What the last autosave (or the file just opened) holds: the document's revision and the
    /// palette's step count (decision 0043, section 7).
    covered: Option<(u64, u64)>,
    /// The autosave job running, if one is.
    running: Option<Arc<JobCell>>,
    /// A recovery landed and no autosave has been tried since.
    recovered: bool,
}

impl Autosave {
    pub fn new(now: Duration) -> Self {
        Self {
            last_save: now,
            last_input: now,
            covered: None,
            running: None,
            recovered: false,
        }
    }

    /// A call from `origin` came in.
    pub fn activity(&mut self, now: Duration, origin: &Origin) {
        if origin.client != AUTOSAVE {
            self.last_input = now;
        }
    }

    /// A save of any kind landed: it restarts Max Duration. An autosave or an open that holds
    /// `covered` (revision, palette step count) also means no autosave is needed until the
    /// scene or its palette changes.
    pub fn saved(&mut self, now: Duration, covered: Option<(u64, u64)>) {
        self.last_save = now;
        if covered.is_some() {
            self.covered = covered;
        }
    }

    /// A recovery landed: autosave as soon as nothing is busy, even though the scene came from
    /// a file, so this session has a copy of its own before the crashed one's marker is gone
    /// for long.
    pub fn recovered(&mut self) {
        (self.recovered, self.covered) = (true, None);
    }

    /// Which trigger fires now with settings `config`, if one does; none with autosave off.
    fn trigger(&self, now: Duration, config: Option<&StateFolder>) -> Option<&'static str> {
        let c = config?;
        if self.recovered {
            Some("recovered")
        } else if now.saturating_sub(self.last_save) >= c.max_duration {
            Some("max_duration")
        } else if now.saturating_sub(self.last_input) >= c.rest_duration {
            Some("rest")
        } else {
            None
        }
    }
}

/// A scene's autosave folder: `<state>/autosave/<name>-<id>`.
pub(crate) fn folder(config: &StateFolder, name: &str, id: &str) -> PathBuf {
    let name: String = (name.chars())
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    config
        .state_dir
        .join("autosave")
        .join(format!("{name}-{id}"))
}

/// The numbered copies in a folder, oldest first.
pub(crate) fn copies(folder: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(u64, PathBuf)> = entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            let stem = path.file_stem()?.to_str()?;
            let gss = path
                .extension()
                .is_some_and(|x| x == gs_io::scene::EXTENSION);
            let n = stem.rsplit_once('-')?.1.parse().ok()?;
            gss.then_some((n, path))
        })
        .collect();
    found.sort();
    found
}

/// The next copy's name: `<name>-<n>.gss`, one past the highest number there.
pub(crate) fn next_file(folder: &Path, name: &str) -> PathBuf {
    let n = copies(folder).last().map_or(1, |c| c.0 + 1);
    folder.join(format!("{name}-{n:03}.{}", gs_io::scene::EXTENSION))
}

/// Removes all but the newest `keep` copies (at least one, at most 100).
pub(crate) fn prune(folder: &Path, keep: usize) {
    let all = copies(folder);
    let drop = all.len().saturating_sub(keep.clamp(1, 100));
    for (_, path) in &all[..drop] {
        fs::remove_file(path).ok();
    }
}

/// A running session, as its marker file says (`<state>/sessions/<session>-<pid>.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Marker {
    pub pid: u32,
    pub session: String,
    /// The session's command log, and which run of it is current (each open starts one).
    pub log: Option<String>,
    #[serde(default)]
    pub log_run: u64,
    /// The autosave folder of the scene it holds, once there is one.
    pub autosave: Option<String>,
}

/// The logs the session markers in state folder `state_dir` name, live or crashed: the runs
/// sessions are writing or left for recovery, which log pruning keeps (decision 0016,
/// section 14).
pub(crate) fn marked_logs(state_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(state_dir.join("sessions")) else {
        return Vec::new();
    };
    (entries.filter_map(|e| {
        let marker: Marker = serde_json::from_slice(&fs::read(e.ok()?.path()).ok()?).ok()?;
        marker.log.map(PathBuf::from)
    }))
    .collect()
}

/// The marker file of this process's session.
fn marker_path(config: &StateFolder, session: &str) -> PathBuf {
    let name = format!("{session}-{}.json", std::process::id());
    config.state_dir.join("sessions").join(name)
}

impl Engine {
    /// Gives the engine its clock (tests move one by hand). Autosave's timers start now.
    pub fn set_clock(&mut self, clock: Box<dyn Clock>) {
        self.clock = clock;
        let now = self.clock.now();
        (self.autosave.last_save, self.autosave.last_input) = (now, now);
    }

    /// The autosave due now, admitted as a job, if one is. The server runs it on a thread.
    pub(crate) fn autosave_due(&mut self) -> Option<Box<Work>> {
        let now = self.clock.now();
        let trigger = self.autosave.trigger(now, self.config.autosave.as_ref())?;
        let busy = self.state.control.held.is_some()
            || self.state.sculpt.live.is_some()
            || self.state.gizmos.unsettled()
            || self.autosave.running.as_ref().is_some_and(|c| c.working());
        let holds = (self.rev, self.state.palette.steps);
        if busy || self.state.doc.len() == 0 || self.autosave.covered == Some(holds) {
            return None;
        }
        // An attempt restarts both timers, so a failing one is not retried every second.
        (self.autosave.last_save, self.autosave.last_input) = (now, now);
        self.autosave.recovered = false;
        let origin = Origin {
            client_id: 0,
            client: AUTOSAVE.into(),
            role: Role::Human,
        };
        let request = Request::Run(RunParams::new(
            "file.quick_save",
            json!({ "reason": trigger }),
        ));
        // Not the window's: autosave shows no bar and waits for no other job.
        match self.start(&origin, request, Caller::InProcess) {
            Start::Job(work) => {
                self.autosave.running = Some(work.cell());
                Some(work)
            }
            Start::Now(_) => None,
        }
    }

    /// Lands every Live Boolean run, waiting for those still working, then checks the autosave
    /// triggers and, when one fires, runs the autosave on this thread and gives its answer. For
    /// in-process callers; the server does both itself.
    pub fn tick(&mut self) -> Option<CmdResponse> {
        self.settle_live();
        let work = self.autosave_due()?;
        Some(self.run_inline(*work))
    }

    /// Writes this session's marker, naming its log and its scene's autosave folder.
    pub(crate) fn mark_session(&self) {
        let Some(config) = &self.config.autosave else {
            return;
        };
        let file: &FileState = &self.state.file;
        let marker = Marker {
            pid: std::process::id(),
            session: self.config.session.clone(),
            log: self
                .config
                .log_path
                .as_ref()
                .map(|p| p.display().to_string()),
            log_run: self.log.position().0,
            autosave: (file.id.as_ref())
                .map(|id| folder(config, &file.name(), id).display().to_string()),
        };
        let path = marker_path(config, &self.config.session);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).ok();
        }
        let text = serde_json::to_string_pretty(&marker).unwrap_or_default();
        fs::write(path, text).ok();
    }

    /// A clean stop: removes this session's marker and, when the scene is saved with no
    /// changes since, its autosaves, which the saved file has overtaken.
    pub fn close(&mut self) {
        let Some(config) = self.config.autosave.clone() else {
            return;
        };
        fs::remove_file(marker_path(&config, &self.config.session)).ok();
        let file = &self.state.file;
        let unsaved = file.unsaved(self.rev, self.state.palette.steps);
        if let (Some(_), Some(id), false) = (&file.path, &file.id, unsaved) {
            let dir = folder(&config, &file.name(), id);
            for (_, path) in copies(&dir) {
                fs::remove_file(path).ok();
            }
            fs::remove_dir(dir).ok();
        }
    }
}
