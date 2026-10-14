//! The standard log folder, and keeping it bounded (decision 0016, section 14). Whole closed
//! files go, oldest first; a file a session is still writing, or one a session marker names
//! (the run recovery would replay), always stays. A run file is never cut: recovery counts its
//! lines from the start, and every run replays on its own from its header.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// Where the standard command logs go, and how many of them are kept.
#[derive(Clone, Debug)]
pub struct StandardLog {
    /// The state folder. Logs go in its `logs/`; the session markers in its `sessions/` name
    /// the logs recovery may still read.
    pub state_dir: PathBuf,
    /// The most log files kept.
    pub files: usize,
    /// The most bytes of log files kept.
    pub bytes: u64,
}

impl StandardLog {
    /// Logs in `state_dir`, keeping the newest 50 files and at most 512 MiB.
    pub fn new(state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            files: 50,
            bytes: 512 << 20,
        }
    }

    /// The folder the logs are in: `<state>/logs`.
    pub fn folder(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    /// The log of session `session` in this process: `<state>/logs/<session>-<pid>.jsonl`.
    /// The process id keeps a new session of the same name from moving a crashed one's log.
    pub fn path(&self, session: &str) -> PathBuf {
        let name = format!("{session}-{}.jsonl", std::process::id());
        self.folder().join(name)
    }

    /// Deletes the oldest logs beyond the limits, keeping the newest by modification time.
    /// Never deletes a file named in `keep` or one a session holds the lock on; those still
    /// count against the limits, so the folder may stay over them while they exist.
    pub(crate) fn prune(&self, keep: &[PathBuf]) {
        let Ok(entries) = fs::read_dir(self.folder()) else {
            return;
        };
        let mut logs: Vec<_> = entries
            .filter_map(|e| {
                let (path, meta) = e.ok().and_then(|e| Some((e.path(), e.metadata().ok()?)))?;
                let log = meta.is_file() && path.extension().is_some_and(|x| x == "jsonl");
                log.then_some((meta.modified().ok()?, path, meta.len()))
            })
            .collect();
        logs.sort_by(|a, b| b.cmp(a));
        let (mut files, mut bytes) = (0, 0u64);
        for (_, path, len) in logs {
            let fits = files < self.files && bytes.saturating_add(len) <= self.bytes;
            if fits || named(keep, &path) || writing(&path) {
                files += 1;
                bytes = bytes.saturating_add(len);
            } else {
                fs::remove_file(&path).ok();
            }
        }
    }
}

/// True if `keep` names a file of `path`'s name. Names are compared, not whole paths, so a
/// path spelled another way still counts; at worst a file stays longer.
fn named(keep: &[PathBuf], path: &Path) -> bool {
    keep.iter().any(|k| k.file_name() == path.file_name())
}

/// True unless the lock on `path` can be taken: a session writing a log holds it.
fn writing(path: &Path) -> bool {
    !File::open(path).is_ok_and(|f| f.try_lock().is_ok())
}
