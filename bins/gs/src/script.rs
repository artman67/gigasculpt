//! `gs run`: replays a script or a session's command log line by line, stopping at the first
//! failure and naming its line as `<script>:<line>:`.
//!
//! A batch line (`cmd.batch`) is sent as one batch, so it stays one undo step. A log header is
//! checked against the session (a different engine or catalog may replay differently). In a log
//! written in verify mode every line carries the scene hash after it; the replay compares after
//! each line and stops at the first one that differs; with `--repro-out` it writes a bug bundle
//! of the lines up to that one (decision 0031). With `--files DIR` a line that read a file reads
//! the file in DIR with the content it pinned instead, wherever the file was then.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gs_protocol::{LogHeader, LogRecord, ScriptLine, method};
use serde_json::{Value, json};

use crate::connection::Connection;
use crate::failure::{Failure, because};
use crate::output::{checked, report};

/// `gs run`'s arguments.
#[derive(clap::Args)]
pub struct RunArgs {
    /// The script.
    pub script: PathBuf,
    /// When a verify log's replay diverges, write a bug bundle of it into this folder
    /// (`gs repro` replays, shrinks and checks it).
    #[arg(long)]
    pub repro_out: Option<PathBuf>,
    /// A folder holding the files the log's lines read (`file.open`, `file.import`,
    /// `reference.open`, `view.environment`): each line reads the file here with the content
    /// hash it pinned, wherever the file was when the log was written.
    #[arg(long)]
    pub files: Option<PathBuf>,
}

/// The files in a folder by their content hash, for `gs run --files DIR`.
struct Files {
    dir: PathBuf,
    by_hash: BTreeMap<String, PathBuf>,
}

impl Files {
    /// Hashes every file in `dir` (not its subfolders), as the engine pins a file it reads: the
    /// BLAKE3 hash of its bytes.
    fn read(dir: &Path) -> Result<Self, Failure> {
        let context = format!("cannot read the folder {}", dir.display());
        let dir = std::path::absolute(dir).map_err(because(Failure::Usage, context.clone()))?;
        let entries = fs::read_dir(&dir).map_err(because(Failure::Usage, context))?;
        let mut by_hash = BTreeMap::new();
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            let mut hasher = blake3::Hasher::new();
            let read = fs::File::open(&path).and_then(|mut f| std::io::copy(&mut f, &mut hasher));
            if read.is_ok() {
                by_hash.insert(hasher.finalize().to_hex().to_string(), path);
            }
        }
        Ok(Self { dir, by_hash })
    }

    /// `record` reading every file it read from this folder: a call that read a file names it
    /// in `path` (`file` for `reference.open`) and pins its content in `hash`, and the file here
    /// with that content takes the name's place. A file the folder lacks is an error naming the
    /// hash and the path the line read it from.
    fn locate(&self, record: &mut LogRecord, out: &mut Vec<String>) -> Result<(), Failure> {
        if record.command != method::BATCH {
            return self.relocate(&mut record.args, out);
        }
        let steps = record
            .args
            .get_mut("commands")
            .and_then(Value::as_array_mut);
        for args in steps
            .into_iter()
            .flatten()
            .filter_map(|s| s.get_mut("args"))
        {
            self.relocate(args, out)?;
        }
        Ok(())
    }

    fn relocate(&self, args: &mut Value, out: &mut Vec<String>) -> Result<(), Failure> {
        let Some(hash) = args.get("hash").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some((key, logged)) = ["path", "file"]
            .into_iter()
            .find_map(|k| Some((k, args.get(k)?.as_str()?)))
        else {
            return Ok(());
        };
        let Some(found) = self.by_hash.get(hash) else {
            return Err(Failure::Usage(format!(
                "no file in {} has the content hash {hash}: the line read it from {logged}",
                self.dir.display()
            )));
        };
        out.push(format!("# reads {} for {logged}", found.display()));
        args[key] = found.display().to_string().into();
        Ok(())
    }
}

/// Runs a script's lines in order, stopping at the first failure; with `--files`, lines that read
/// a file read the one in that folder with the same content.
pub fn run(
    connection: &mut Connection,
    args: &RunArgs,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let (script, repro_out) = (args.script.as_path(), args.repro_out.as_deref());
    let files = args.files.as_deref().map(Files::read).transpose()?;
    let text = fs::read_to_string(script).map_err(because(
        Failure::Usage,
        format!("cannot read {}", script.display()),
    ))?;
    let lines = text
        .lines()
        .zip(1..)
        .filter(|(line, _)| !line.trim().is_empty());
    let mut header = None;
    let mut records = Vec::new();
    for (index, (line, number)) in lines.enumerate() {
        let failed = |failure| Failure::Script {
            line: format!("{}:{number}", script.display()),
            failure: Box::new(failure),
        };
        let parsed = ScriptLine::parse(line).map_err(|e| failed(Failure::Usage(e)))?;
        match parsed {
            ScriptLine::Header(found) if index == 0 => {
                check_header(connection, &found, out).map_err(failed)?;
                header = Some(found);
            }
            ScriptLine::Header(_) => {
                return Err(failed(Failure::Usage(
                    "a second log header: one log is one session run; replay each run into a \
                     fresh session"
                        .into(),
                )));
            }
            ScriptLine::Record(mut record) => {
                out.push(format!("# {}", record.command));
                if let Some(files) = &files {
                    files.locate(&mut record, out).map_err(failed)?;
                }
                records.push(line.to_string());
                let mut replayed = replay(connection, &record, json, out);
                if let (Err(Failure::Diverged(why)), Some(dir), Some(expected)) =
                    (&replayed, repro_out, &record.hash)
                {
                    let bundle = crate::repro::write_diverged(
                        dir,
                        script,
                        header.as_ref(),
                        &records,
                        expected,
                    );
                    let said = match bundle {
                        Ok(to) => format!("a bug bundle was written to {}", to.display()),
                        Err(e) => format!("no bug bundle was written: {e}"),
                    };
                    replayed = Err(Failure::Diverged(format!("{why}; {said}")));
                }
                replayed.map_err(failed)?;
            }
        }
    }
    Ok(())
}

/// Warns when the log was written by another engine version or catalog.
fn check_header(
    connection: &mut Connection,
    header: &LogHeader,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let catalog = connection.catalog()?;
    if catalog.engine_version != header.engine_version || catalog.hash != header.catalog_hash {
        out.push(format!(
            "warning: the log was written by engine {} (catalog {}), this session runs {} \
             (catalog {}); the replay may differ",
            header.engine_version, header.catalog_hash, catalog.engine_version, catalog.hash
        ));
    }
    Ok(())
}

/// Runs one line, and in a verify log checks the scene hash after it.
fn replay(
    connection: &mut Connection,
    record: &LogRecord,
    json: bool,
    out: &mut Vec<String>,
) -> Result<(), Failure> {
    let request = record.request().map_err(Failure::Usage)?;
    report(&connection.send(request)?, json, out)?;
    let Some(expected) = &record.hash else {
        return Ok(());
    };
    let response = connection.run("mesh.hash", json!({}))?;
    checked(&response)?;
    let got = response["ok"]["result"]["hash"]
        .as_str()
        .unwrap_or_default();
    if got == expected {
        return Ok(());
    }
    let seq = record
        .seq
        .map(|s| format!(" (seq {s} in the log)"))
        .unwrap_or_default();
    Err(Failure::Diverged(format!(
        "the replay diverged at {}{seq}: the scene hash is {got}, the log recorded {expected}",
        record.command
    )))
}
