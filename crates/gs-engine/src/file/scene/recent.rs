//! The recent files (decision 0016, section 16): `file.recent`, and the list behind it and
//! behind `file.open`'s `recent`. The list is `recent.json` in the state folder, beside the
//! autosave folders, so it lives only in sessions with autosave on; tests point that folder at
//! a temporary one.

use std::path::{Path, PathBuf};

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::error;
use crate::{Command, Ctx, Example, register_command};

/// How many files the list keeps: the catalog's recent count.
pub(crate) const KEPT: usize = 10;

/// The list as it is written: paths, newest first.
#[derive(Default, Serialize, Deserialize)]
struct Stored {
    files: Vec<String>,
}

/// The list's file name in the state folder.
const LIST: &str = "recent.json";

/// The list's file in the state folder `state`.
fn list_file(state: &Path) -> PathBuf {
    state.join(LIST)
}

/// The paths in the list, newest first. An error names the file when it does not read.
pub(crate) fn read(state: &Path) -> Result<Vec<String>, String> {
    let file = list_file(state);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|e| format!("{} does not read as a recent list: {e}", file.display()))?;
    Ok(stored.files)
}

/// Puts `path` at the top of the list in `state`, once, and keeps the newest [`KEPT`]. A list
/// that does not read is started afresh. Written under a temporary name and renamed, so a
/// crash never leaves half a list.
pub(crate) fn record(state: &Path, path: &Path) -> std::io::Result<()> {
    let path = std::path::absolute(path)?.display().to_string();
    let mut files = read(state).unwrap_or_default();
    files.retain(|f| *f != path);
    files.insert(0, path);
    files.truncate(KEPT);
    let text = serde_json::to_string_pretty(&Stored { files }).map_err(std::io::Error::other)?;
    super::write_whole(state, LIST, &text)
}

/// The state folder that holds the list: autosave's (decision 0016, section 10). `None` in a
/// session with autosave off, which keeps no list.
pub(crate) fn state_folder(cx: &Ctx) -> Option<PathBuf> {
    cx.config.autosave.as_ref().map(|a| a.state_dir.clone())
}

/// The refusal for a session that keeps no list.
fn off() -> CommandError {
    CommandError {
        hint: Some("the window keeps one; gigasculpt serve keeps one with --autosave".into()),
        ..error(
            ErrorCode::Precondition,
            "autosave is off in this session, so it keeps no recent files",
        )
    }
}

/// The paths in this session's list, newest first; refused where there is none or it does not
/// read.
pub(crate) fn listed(cx: &Ctx) -> Result<Vec<String>, CommandError> {
    let state = state_folder(cx).ok_or_else(off)?;
    read(&state).map_err(|why| CommandError {
        hint: Some("delete it; the next file opened or saved starts a new list".into()),
        ..error(ErrorCode::Damaged, why)
    })
}

/// List the scene files opened or saved lately, newest first, at most 10.
///
/// `file.open`, `file.revert` and every save of the scene (`file.save` without `parts`,
/// `file.save_next`) put their file at the top when they land; a file already listed moves up.
/// A file that has since been moved or deleted stays listed with `gone` set, until newer files
/// push it off the end. `file.open` with `recent` opens one by its place here, 1 the newest.
/// The list sits in the state folder beside the autosaves and is shared by every session that
/// has autosave on; a session with autosave off keeps none and is refused. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRecent {}

/// The recent files.
#[derive(Serialize, JsonSchema)]
pub struct RecentFiles {
    /// Newest first.
    pub files: Vec<RecentFile>,
    /// How many the list keeps.
    pub kept: usize,
}

/// One recent file.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct RecentFile {
    /// Its place in the list, 1 the newest: what `file.open`'s `recent` takes.
    pub recent: usize,
    pub path: String,
    /// The file's name without its folder and extension.
    pub name: String,
    /// The name of the folder it is in, which tells apart two files of one name.
    pub folder: String,
    /// True when no file is at `path` now: it was moved or deleted.
    pub gone: bool,
}

/// `paths` as the list shows them, newest first.
fn files(paths: Vec<String>) -> Vec<RecentFile> {
    let name =
        |p: Option<&std::ffi::OsStr>| p.map_or_else(String::new, |s| s.to_string_lossy().into());
    (paths.into_iter().enumerate())
        .map(|(i, path)| {
            let at = Path::new(&path);
            RecentFile {
                recent: i + 1,
                name: name(at.file_stem()),
                folder: name(at.parent().and_then(Path::file_name)),
                gone: !at.exists(),
                path,
            }
        })
        .collect()
}

/// The list in the state folder `state`, as `session.state`'s `file.recent` gives it with every
/// call for the window's Open a recent scene (section 17, item 7); empty when it does not read,
/// which `file.recent` says.
pub(crate) fn shown_recent(state: &Path) -> Vec<RecentFile> {
    files(read(state).unwrap_or_default())
}

impl Command for FileRecent {
    const NAME: &'static str = "file.recent";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = RecentFiles;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "the recent files (refused here: autosave is off)",
                json!({}),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<RecentFiles, CommandError> {
        let files = files(listed(cx)?);
        Ok(RecentFiles { files, kept: KEPT })
    }
}
register_command!(FileRecent, "List recent scenes");
