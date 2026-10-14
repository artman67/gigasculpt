//! The files a bundle's lines read (`Command::READS_FILE`), copied into its `files/` folder, each
//! line's path rewritten to its copy, relative to the bundle the way `start.gss` is (review 0008,
//! m4). `gs repro` replays with the bundle folder as the engine's working folder, so the replay
//! reads the copies wherever the originals went. The line's pinned `hash` still guards a file
//! that changed after its line ran.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use gs_protocol::{LogRecord, bundle_file, method};
use serde_json::Value;

use super::link_or_copy;
use crate::command::registry;

/// Copies the files `lines` read into `folder` and points the lines at the copies. Returns the
/// lines, and the paths that could not be read, whose lines stay as they were.
pub(super) fn carry(lines: Vec<String>, folder: &Path) -> io::Result<(Vec<String>, Vec<String>)> {
    let mut files = Files {
        folder,
        readers: (registry().into_iter())
            .filter_map(|spec| Some((spec.name, spec.reads_file?)))
            .collect(),
        copied: BTreeMap::new(),
        unread: Vec::new(),
    };
    let mut out = Vec::with_capacity(lines.len());
    for text in lines {
        let Ok(mut record) = serde_json::from_str::<LogRecord>(&text) else {
            out.push(text);
            continue;
        };
        if files.call(&record.command.clone(), &mut record.args)? {
            out.push(serde_json::to_string(&record).unwrap_or(text));
        } else {
            out.push(text);
        }
    }
    Ok((out, files.unread))
}

struct Files<'a> {
    folder: &'a Path,
    /// Each command that reads a file, and the argument naming it.
    readers: BTreeMap<&'static str, &'static str>,
    /// Each path read, and its copy's path in the bundle.
    copied: BTreeMap<String, String>,
    unread: Vec<String>,
}

impl Files<'_> {
    /// Points one call at its copy, or each step of a batch at theirs. True if any changed.
    fn call(&mut self, command: &str, args: &mut Value) -> io::Result<bool> {
        if command == method::BATCH {
            let mut changed = false;
            let steps = args.get_mut("commands").and_then(Value::as_array_mut);
            for step in steps.into_iter().flatten() {
                let command = step.get("command").and_then(Value::as_str);
                let command = command.unwrap_or_default().to_string();
                if let Some(args) = step.get_mut("args") {
                    changed |= self.call(&command, args)?;
                }
            }
            return Ok(changed);
        }
        let Some(&key) = self.readers.get(command) else {
            return Ok(false);
        };
        let Some(Value::String(path)) = args.get_mut(key) else {
            return Ok(false);
        };
        let Some(copy) = self.copy(path)? else {
            return Ok(false);
        };
        *path = copy;
        Ok(true)
    }

    /// The bundle path of `path`'s copy, made on first use; `None` if it cannot be read.
    fn copy(&mut self, path: &str) -> io::Result<Option<String>> {
        if let Some(copy) = self.copied.get(path) {
            return Ok(Some(copy.clone()));
        }
        let from = Path::new(path);
        if !from.is_file() {
            if !self.unread.iter().any(|p| p == path) {
                self.unread.push(path.into());
            }
            return Ok(None);
        }
        // A folder of its own keeps the file's name, which commands read (`file.import` names
        // the part after it).
        let name = from.file_name().unwrap_or_default().to_string_lossy();
        let own = format!("{}/{}", bundle_file::FILES, self.copied.len() + 1);
        fs::create_dir_all(self.folder.join(&own))?;
        let copy = format!("{own}/{name}");
        link_or_copy(from, &self.folder.join(&copy))?;
        self.copied.insert(path.into(), copy.clone());
        Ok(Some(copy))
    }
}
