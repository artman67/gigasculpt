//! `file.save` and `file.quick_save` (decision 0016, sections 6, 7 and 10). Both are
//! background queries: their job writes the session's copy of the scene (see `write`), so
//! sculpting goes on, and hands the engine thread a `Saved` landing.

use std::path::{Path, PathBuf};

use gs_io::scene::{self, FileKind, Written};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::autosave;
use super::write::{Target, scene_id, session, write};
use super::{Landing, SaveKind, Saved};
use crate::document::Document;
use crate::errors::{self, error};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Save the scene, or some Parts as a Part file, to a `.gss` file, in the background.
///
/// Left out, `path` is the scene's own file; the first save of a scene needs one. A file at
/// another path is replaced only with `overwrite`. Saving never risks the old file: the new one
/// is written beside it and renamed into place, and the old one is kept as
/// `<name>.backup.gss` unless `backup` is off. Every Part is stored exactly as it is held, so
/// opening the file gives the same scene hash and the same result for the next command. A scene
/// file also keeps the open Reference Views' setups as they are now, which `file.open` brings
/// back; they are not part of the scene. It keeps the background (`view.background`) too.
/// Runs as a job for every caller; the scene keeps its unsaved mark if it changed while saving.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent save switches, one per save option"
)]
pub struct FileSave {
    /// The file to write, ending in `.gss`. A relative path starts at the engine's working
    /// folder, not the caller's: give an absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
    /// Write only these Parts, as a Part file. The scene keeps its own file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PartRef>>,
    /// Replace a file at `path` that is not the scene's own file.
    #[serde(default)]
    pub overwrite: bool,
    /// Keep the file it replaces as `<name>.backup.gss`.
    #[serde(default = "crate::command::yes")]
    pub backup: bool,
    /// Replace the scene's own file even though it was written in an older format.
    #[serde(default)]
    pub upgrade: bool,
    /// Keep the open Reference Views in a scene file: each one's source, link, alignment, look,
    /// camera and lock, which one is active, and a parts reference's frozen copies. Off, the
    /// file holds none. A Part file never does.
    #[serde(default = "crate::command::yes")]
    pub references: bool,
}

/// What a save wrote.
#[derive(Serialize, JsonSchema)]
pub struct SavedFile {
    /// The file written.
    pub path: String,
    /// `scene` or `parts`.
    pub kind: String,
    /// The Parts in it, in order.
    pub parts: Vec<String>,
    /// The file's size.
    pub bytes: u64,
    /// The stored columns' size before compression.
    pub raw_bytes: u64,
    /// The scene revision the file holds.
    pub rev: u64,
    /// True if the file holds a preview picture.
    pub preview: bool,
    /// Where the replaced file was kept, if one was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

impl Command for FileSave {
    const NAME: &'static str = "file.save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    type Output = SavedFile;

    fn examples() -> Vec<Example<Self>> {
        let to = |path: &str| Self {
            path: Some(path.into()),
            parts: None,
            overwrite: false,
            backup: true,
            upgrade: false,
            references: true,
        };
        vec![
            Example::new(
                "save a sphere (refused here: the folder does not exist)",
                to("/nonexistent/folder/sphere.gss"),
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::raw("a scene never saved needs a path", json!({}))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    /// A first save without a path is refused before a job starts, naming `path`, so a dry run
    /// says so and the window asks for a name (decision 0016, section 17).
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        match (&self.path, &cx.file.path) {
            (None, None) => Err(Unavailable::new("the scene has no file yet; give a path")
                .fix("give a path ending in .gss")
                .asks("path")),
            _ => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<SavedFile, CommandError> {
        cx.settled()?;
        self.check(cx).map_err(errors::unavailable)?;
        let own = cx.file.path.clone();
        let path = self.path.as_ref().map(PathBuf::from);
        let Some(path) = path.or_else(|| own.clone()) else {
            return Err(errors::internal("checked: a path or the scene's own file"));
        };
        let is_own = own.as_deref() == Some(path.as_path());
        check_target(&path, is_own, self.overwrite)?;
        // Older programs that open the file now must still open it, or the caller says so.
        let older = cx.file.format.filter(|_| is_own && !self.upgrade);
        let doc = cx.doc.document();
        let names = chosen(doc, self.parts.as_deref())?;
        let kind = if self.parts.is_some() {
            FileKind::Parts
        } else {
            FileKind::Scene
        };
        let backup = self.backup.then(|| scene::backup_path(&path));
        let references = self.references && kind == FileKind::Scene;
        let session = session(cx, None, references);
        let target = Target {
            path: &path,
            backup: backup.as_deref(),
            older,
        };
        let written = write(cx, target, &names, kind, session, references)?;
        let id = scene_id(cx, &path);
        let save = if kind == FileKind::Scene {
            SaveKind::Scene
        } else {
            SaveKind::Parts
        };
        cx.land(Landing::Saved(Saved {
            path: path.clone(),
            kind: save,
            rev: cx.rev,
            palette_steps: cx.palette.steps,
            generation: cx.file.generation,
            id,
        }));
        let backup = backup.filter(|b| b.exists());
        Ok(saved_file(
            &path, kind, names, written.0, cx.rev, written.1, backup,
        ))
    }
}
register_command!(FileSave, "Save scene");

/// Save a numbered copy of the scene in its autosave folder, at once, in the background.
///
/// The folder is `autosave/<scene name>-<scene id>/` in the state folder, which `gigasculpt
/// serve --help` names on each system; the newest copies are kept (10 by default) and older
/// ones removed. Autosave writes the same copies on its own (decision 0016). The copy records
/// where it sits in the command log, so after a crash `file.recover` can bring the scene back
/// to the last command. A session with autosave off has no autosave folder.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileQuickSave {
    /// Why the copy is written: `quick` for a user's, `max_duration`, `rest` or `recovered` for
    /// autosave's.
    #[serde(default = "quick")]
    pub reason: String,
}

fn quick() -> String {
    "quick".into()
}

impl Command for FileQuickSave {
    const NAME: &'static str = "file.quick_save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    type Output = SavedFile;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("a quick save (refused here: autosave is off)", json!({}))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<SavedFile, CommandError> {
        cx.settled()?;
        let config = cx.config.autosave.clone().ok_or_else(|| {
            error(
                ErrorCode::Precondition,
                "autosave is off in this session, so it has no autosave folder",
            )
        })?;
        // An unsaved scene's id mixes in the session and process, so two sessions' untitled
        // scenes never share a folder.
        let untitled = format!("{}#{}", cx.config.session, std::process::id());
        let id = cx
            .file
            .id
            .clone()
            .unwrap_or_else(|| scene_id(cx, Path::new(&untitled)));
        let folder = autosave::folder(&config, &cx.file.name(), &id);
        let made = std::fs::create_dir_all(&folder);
        made.map_err(|e| crate::file::file_refused("write", &folder, &e))?;
        let path = autosave::next_file(&folder, &cx.file.name());
        let doc = cx.doc.document();
        let names = chosen(doc, None)?;
        let autosave = self.reason != quick();
        let session = session(cx, Some(self.reason), true);
        let target = Target {
            path: &path,
            backup: None,
            older: None,
        };
        let written = write(cx, target, &names, FileKind::Scene, session, true)?;
        autosave::prune(&folder, config.keep);
        cx.land(Landing::Saved(Saved {
            path: path.clone(),
            kind: SaveKind::Quick { autosave },
            rev: cx.rev,
            palette_steps: cx.palette.steps,
            generation: cx.file.generation,
            id,
        }));
        Ok(saved_file(
            &path,
            FileKind::Scene,
            names,
            written.0,
            cx.rev,
            written.1,
            None,
        ))
    }
}
register_command!(FileQuickSave, "Quick save");

fn saved_file(
    path: &Path,
    kind: FileKind,
    parts: Vec<String>,
    written: Written,
    rev: u64,
    preview: bool,
    backup: Option<PathBuf>,
) -> SavedFile {
    SavedFile {
        path: path.display().to_string(),
        kind: if kind == FileKind::Scene {
            "scene"
        } else {
            "parts"
        }
        .into(),
        parts,
        bytes: written.bytes,
        raw_bytes: written.raw,
        rev,
        preview,
        backup: backup.map(|b| b.display().to_string()),
    }
}

/// A `.gss` path in a folder that exists, and not a file the save may not replace. A refusal
/// quotes the path it got, so a stray prefix or space shows.
fn check_target(path: &Path, own: bool, overwrite: bool) -> Result<(), CommandError> {
    let gss = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(scene::EXTENSION));
    if !gss {
        return Err(CommandError {
            hint: Some("scene files end in .gss; file.export writes obj, ply and stl".into()),
            ..errors::args_invalid(
                "/path",
                format!("\"{}\" does not end in .gss", path.display()),
            )
        });
    }
    crate::file::has_folder(path)?;
    crate::file::not_a_folder(path)?;
    if path.exists() && !own && !overwrite {
        // Naming `overwrite`, so the window asks to replace it (review 0009, B2).
        return Err(CommandError {
            path: Some("/overwrite".into()),
            hint: Some("pass overwrite: true to replace it".into()),
            ..error(
                ErrorCode::Precondition,
                format!("a file exists at \"{}\"", path.display()),
            )
        });
    }
    Ok(())
}

/// The Parts to write, by name: the ones `parts` refers to (a name, `#id` or `@place`), or
/// all, in list order.
fn chosen(doc: &Document, parts: Option<&[PartRef]>) -> Result<Vec<String>, CommandError> {
    let Some(parts) = parts else {
        return Ok(doc.list().into_iter().map(|(n, _)| n.to_string()).collect());
    };
    if parts.is_empty() {
        return Err(errors::args_invalid("/parts", "name at least one part"));
    }
    let mut names: Vec<String> = Vec::new();
    for (i, PartRef(r)) in parts.iter().enumerate() {
        let Some(name) = doc.resolve(r) else {
            return Err(CommandError {
                path: Some(format!("/parts/{i}")),
                ..errors::not_found("part", r, doc.names())
            });
        };
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}
