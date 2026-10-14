//! Writing the command log. The format, and why replaying it rebuilds the scene, is described
//! in `gs_protocol`'s `script` module (`LogHeader`, `LogRecord`).
//!
//! A log is never erased: when a session starts and its log file already has something in it,
//! the old file is renamed to the first free `<name>-1.jsonl`, `<name>-2.jsonl`... (oldest
//! first), and the new run starts a fresh file with its own header. One file is one run, so
//! every file replays on its own.
//!
//! The last [`RECORDER_LINES`] lines (at most [`RECORDER_BYTES`] of them) are also kept in
//! memory, with who ran them and their records as the log file gets them: `log.tail` shows the
//! newest of them, and a bug bundle (decision 0031) takes the current run's lines from them.
//! That works with or without a log file.
//!
//! Opening a scene file starts a new run the same way (decision 0016): the old file is kept
//! under a numbered name and the new one starts with its own header, so the open is its first
//! line. A save records how many lines the current run holds, so recovery knows which lines
//! came after it.
//!
//! While a run is written, its file is locked (an advisory lock), so pruning the standard log
//! folder (`standard`) can tell a live log from a closed one.

mod standard;
mod tail;

pub use standard::StandardLog;

use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use gs_protocol::{LogHeader, LogRecord, Role, free_path};
use schemars::JsonSchema;
use serde::Serialize;

/// How many recent lines the log keeps in memory: the flight recorder (decision 0031).
pub(crate) const RECORDER_LINES: usize = 2_000;
/// The most bytes of records the recorder keeps; older lines go first beyond it.
pub(crate) const RECORDER_BYTES: usize = 64 << 20;

/// Where the log goes, and its last lines.
pub(crate) struct CommandLog {
    file: Option<File>,
    recent: VecDeque<LogLine>,
    /// The bytes of the records in `recent`.
    recent_bytes: usize,
    /// Runs started, counting this one.
    run: u64,
    /// Records in the current run, the header not counted.
    records: u64,
    /// The brush presets the current run carries in full, by hash (decision 0042).
    carried: BTreeSet<String>,
}

/// One log line as `log.tail` shows it.
#[derive(Clone, Serialize, JsonSchema)]
pub struct LogLine {
    /// Sequence number of the call (its first step's, for a batch).
    pub seq: u64,
    /// Document revision after the call.
    pub rev: u64,
    /// The client that ran it, as `name#connection`.
    pub client: String,
    /// The client's role.
    pub role: Role,
    /// The commands it ran, in order: a batch lists its steps.
    pub commands: Vec<String>,
    /// `ok`. The log only records calls that succeeded (ARCHITECTURE 8).
    pub status: String,
    /// The run it belongs to (counting from 1).
    #[serde(skip)]
    pub(crate) run: u64,
    /// Its record as the log file has it, without the newline.
    #[serde(skip)]
    pub(crate) record: Arc<str>,
}

impl LogLine {
    /// The client's name, without `#connection`.
    fn client_name(&self) -> &str {
        self.client
            .rsplit_once('#')
            .map_or(self.client.as_str(), |(name, _)| name)
    }
}

impl CommandLog {
    /// Starts a new log at `path` with `header` as its first line, keeping any older log at
    /// that path under a numbered name. `None` keeps no log.
    pub fn start(path: Option<&Path>, header: &LogHeader) -> io::Result<Self> {
        let mut log = Self {
            file: None,
            recent: VecDeque::new(),
            recent_bytes: 0,
            run: 0,
            records: 0,
            carried: BTreeSet::new(),
        };
        log.restart(path, header)?;
        Ok(log)
    }

    /// Starts a new run at `path` with `header` as its first line, keeping the old file
    /// under a numbered name. The last lines stay in memory for `log.tail`.
    pub fn restart(&mut self, path: Option<&Path>, header: &LogHeader) -> io::Result<()> {
        self.run += 1;
        self.records = 0;
        self.carried.clear();
        self.file = None;
        let Some(path) = path else {
            return Ok(());
        };
        if path.exists() {
            fs::rename(path, free_path(path))?;
        }
        let file = File::create(path)?;
        // Held until the run ends; a file system without locks only makes pruning keep it.
        file.try_lock().ok();
        self.file = Some(file);
        let text = serde_json::to_string(header).unwrap_or_default();
        match self.write(&text) {
            Some(warning) => Err(io::Error::other(warning)),
            None => Ok(()),
        }
    }

    /// The current run (counting from 1) and the records in it.
    pub fn position(&self) -> (u64, u64) {
        (self.run, self.records)
    }

    /// Whether the current run has not carried the brush preset with `hash` yet; it counts as
    /// carried from now on (decision 0042).
    pub fn first_use(&mut self, hash: &str) -> bool {
        self.carried.insert(hash.to_string())
    }

    /// Appends one record, and remembers it with `line`, forgetting the oldest lines beyond
    /// [`RECORDER_LINES`] or [`RECORDER_BYTES`]. The record is turned into text once, here,
    /// for the file and the recorder both. Returns a warning if it could not be written.
    pub fn append(&mut self, record: &LogRecord, mut line: LogLine) -> Option<String> {
        let text = serde_json::to_string(record).unwrap_or_default();
        let warning = self.write(&text);
        self.recent_bytes += text.len();
        line.run = self.run;
        line.record = text.into();
        self.recent.push_back(line);
        while self.recent.len() > RECORDER_LINES || self.recent_bytes > RECORDER_BYTES {
            let Some(old) = self.recent.pop_front() else {
                break;
            };
            self.recent_bytes -= old.record.len();
        }
        self.records += 1;
        warning
    }

    /// The last lines, oldest first.
    pub fn recent(&self) -> &VecDeque<LogLine> {
        &self.recent
    }

    fn write(&mut self, text: &str) -> Option<String> {
        let file = self.file.as_mut()?;
        let mut line = String::with_capacity(text.len() + 1);
        line.push_str(text);
        line.push('\n');
        file.write_all(line.as_bytes())
            .err()
            .map(|e| format!("could not write the command log: {e}"))
    }
}
