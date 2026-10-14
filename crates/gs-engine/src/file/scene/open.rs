//! `file.open` and `file.revert` (decision 0016, sections 6, 7 and 16): background session
//! commands. Their job reads and checks the file into a new document while the old one stays on
//! screen; the engine thread only swaps it in when the call commits.

use std::path::{Path, PathBuf};

use gs_io::scene::FileKind;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::{Refused, load, read, unreadable};
use super::{Landing, Opened, recent};
use crate::errors::args_invalid;
use crate::file::{content_hash, io_error};
use crate::material::scene::opened;
use crate::view::saved::{ReopenedReferences, reopen};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Replace the scene with a `.gss` scene or Part file, in the background.
///
/// Refused while the scene has unsaved changes, unless `discard_changes` is on. Every Part
/// comes back exactly as it was saved: the same scene hash, and the same result for the next
/// command, random ones included (the file keeps the session seed and command counter). The
/// history starts empty and the command log starts a new run with this call, the file's content
/// hash pinned, so a replay refuses a changed file. A Part whose data is damaged is refused by
/// name and the rest open; then the scene gets no file of its own, so saving needs a path and
/// cannot overwrite the file that still holds that Part. The Reference Views a scene file keeps
/// open in place of the session's (a Part file keeps none, so they close); one whose file is
/// missing or changed opens empty with its setup kept, and `references` says why. They are not
/// part of the scene, so unsaved changes never count them. The background a scene file keeps
/// replaces the session's (`view.background`). So do its material palette and active
/// material, with the palette's undo empty: a file without them (format 4 or older, or a Part
/// file) opens with the shipped palette and MatCap Gray active, as does one whose palette this
/// version cannot read, which `skipped` names; then each user folder `material.folder` read
/// this session is read again into it, a logged call of its own. While the file loads, edits
/// wait with `E_BUSY`. Give `path`, or `recent` to open a file from `file.recent`'s list; either
/// way the log records the path, and the file goes to the top of that list.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileOpen {
    /// The file to open. A relative path starts at the engine's working folder, not the
    /// caller's: give an absolute path. Give this or `recent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
    /// Open a recent file instead: its place in `file.recent`'s list, 1 the newest. The engine
    /// puts its path in `path` before the open runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub recent: Option<usize>,
    /// Open even though the scene has unsaved changes, which are lost.
    #[serde(default)]
    pub discard_changes: bool,
    /// The BLAKE3 hash of the file's bytes, as 64 hex digits. Filled in by the engine, so the
    /// log records which content was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// What an open brought in.
#[derive(Serialize, JsonSchema)]
pub struct OpenedFile {
    /// The file opened.
    pub path: String,
    /// `scene` or `parts`.
    pub kind: String,
    /// The Parts opened, in order.
    pub parts: Vec<String>,
    /// Parts that could not be opened, and why.
    pub refused: Vec<Refused>,
    /// Optional data this version does not know or cannot read, and left out.
    pub skipped: Vec<String>,
    /// The format version that wrote the file.
    pub format: u32,
    /// The program that wrote it.
    pub writer: String,
    /// The scene's own file now; `null` when no file holds the scene as it is.
    pub file: Option<String>,
    /// The Reference Views open now: the file's own.
    pub references: ReopenedReferences,
}

impl Command for FileOpen {
    const NAME: &'static str = "file.open";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    const READS_FILE: Option<&'static str> = Some("path");
    type Output = OpenedFile;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "open a scene file (refused here: there is no such file)",
                json!({ "path": "/nonexistent/scene.gss" }),
            )
            .fails_with(ErrorCode::NotFound),
            Example::raw(
                "open the file opened or saved last (refused here: autosave is off, so there \
                 are no recent files)",
                json!({ "recent": 1 }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    /// Turns `recent` into the path it names. Only reads the small list and looks the file
    /// up, because the window dry-runs each File > Open recent row it draws after every call;
    /// the job hashes the file (`run`).
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let Some(n) = self.recent.take() else {
            return match self.path {
                Some(_) => Ok(()),
                None => Err(args_invalid("/path", "give path, or recent")),
            };
        };
        if self.path.is_some() {
            return Err(args_invalid("/recent", "give path or recent, not both"));
        }
        let files = recent::listed(cx)?;
        let path = (n.checked_sub(1).and_then(|i| files.get(i))).ok_or_else(|| {
            let held = files.len();
            CommandError {
                hint: Some("file.recent lists them".into()),
                ..crate::errors::error(
                    ErrorCode::NotFound,
                    format!("there is no recent file {n}: the list holds {held}"),
                )
            }
        })?;
        exists(Path::new(path))?;
        self.path = Some(path.clone());
        Ok(())
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::replaceable(cx, self.discard_changes)
    }

    fn run(self, cx: &mut Ctx) -> Result<OpenedFile, CommandError> {
        let path = self
            .path
            .ok_or_else(|| args_invalid("/path", "give path, or recent"))?;
        // A recent file is hashed here, off the engine thread, and logged with its hash.
        let log_as = match self.hash {
            Some(_) => None,
            None => Some(pinned_open(&path, self.discard_changes)?),
        };
        open_file(cx, &path, self.discard_changes, log_as)
    }
}
register_command!(FileOpen, "Open scene");

/// Reopen the scene's own file, in the background, dropping every change since it was saved.
///
/// The same as `file.open` of that file with `discard_changes`, and logged as exactly that
/// open with the file's content hash, so a replay needs only the file. The history starts
/// empty. It never asks first: the call is the decision, and the window asks before it runs
/// one. Refused when the scene has no file (never saved, or opened with a Part refused) and
/// when the file has gone.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRevert {}

impl Command for FileRevert {
    const NAME: &'static str = "file.revert";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    type Output = OpenedFile;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "go back to the saved file (refused here: the scene was never saved)",
                Self {},
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::has_file(
            cx,
            "the scene has no file to go back to",
            "save it first: a scene never saved, or opened with a part refused, has none",
        )
    }

    fn run(self, cx: &mut Ctx) -> Result<OpenedFile, CommandError> {
        let path = (cx.file.path.as_ref()).map(|p| p.display().to_string());
        let path = path.ok_or_else(|| {
            crate::errors::error(
                ErrorCode::Precondition,
                "the scene has no file to go back to",
            )
        })?;
        let log_as = pinned_open(&path, true)?;
        open_file(cx, &path, true, Some(log_as))
    }
}
register_command!(FileRevert, "Revert to saved");

/// The `file.open` a call amounts to, with the hash of the file at `path` as it is now.
fn pinned_open(path: &str, discard: bool) -> Result<(String, Value), CommandError> {
    exists(Path::new(path))?;
    let hash = content_hash(path).map_err(|e| io_error(path, &e))?;
    let args = json!({ "path": path, "discard_changes": discard, "hash": hash });
    Ok(("file.open".into(), args))
}

/// Reads the file at `path` into a new document and hands it over to land: `file.open`'s
/// work, and `file.revert`'s. `log_as` replaces the call's own log line.
fn open_file(
    cx: &mut Ctx,
    shown: &str,
    discard: bool,
    log_as: Option<(String, Value)>,
) -> Result<OpenedFile, CommandError> {
    let path = PathBuf::from(shown);
    exists(&path)?;
    cx.progress(None, format!("reading {}", path.display()));
    let file = read(&path).map_err(|e| unreadable(&path, &e))?;
    let (kind, format, writer, mut skipped) = (
        file.header.kind,
        file.header.format,
        file.writer.clone(),
        file.skipped.clone(),
    );
    cx.progress(None, "building the parts");
    let mut loaded = load(file);
    let saved = loaded.session.references.take();
    let (references, report) = reopen(cx, saved, &mut loaded.kept, &mut skipped);
    let (palette, unread) = opened(loaded.scene.palette.take(), loaded.session.material);
    skipped.extend(unread);
    let whole = kind == FileKind::Scene && loaded.refused.is_empty();
    let own = whole.then(|| path.clone());
    let session = (loaded.session.seed).zip(loaded.session.counter);
    let out = OpenedFile {
        path: shown.to_string(),
        kind: if kind == FileKind::Scene {
            "scene"
        } else {
            "parts"
        }
        .into(),
        parts: loaded.names,
        refused: loaded.refused,
        skipped,
        format,
        writer,
        file: own.as_ref().map(|p| p.display().to_string()),
        references: report,
    };
    cx.land(Landing::Opened(Box::new(Opened {
        doc: loaded.doc,
        path: own,
        clean: whole,
        forget: None,
        session: session.filter(|_| kind == FileKind::Scene),
        id: loaded.scene.id,
        format: Some(format),
        rev: cx.rev,
        discard,
        replay: Vec::new(),
        log_as,
        references,
        recent: Some(path),
        background: crate::Background::saved(loaded.scene.background),
        palette: Some(palette),
    })));
    Ok(out)
}

/// `E_NOT_FOUND` for a missing file, with the usual hint.
pub(crate) fn exists(path: &Path) -> Result<(), CommandError> {
    std::fs::metadata(path)
        .map(drop)
        .map_err(|e| io_error(&path.display().to_string(), &e))?;
    crate::file::not_a_folder(path)
}
