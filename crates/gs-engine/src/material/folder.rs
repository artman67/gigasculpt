//! `material.folder` (decision 0043, step (d)): the user folder's look files read into user
//! materials. The window runs it once at start with its `materials` folder; it is logged, so a
//! replay reads the folder again.

use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use gs_core::material::{Library, Refused};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Outcome, Request, RunParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{PaletteEntry, RefusedMaterial, hand_over};
use crate::engine::Engine;
use crate::errors::error;
use crate::{Command, Ctx, Example, Origin, register_command};

/// What `material.folder` read.
#[derive(Serialize, JsonSchema)]
pub struct FolderRead {
    /// The folder, as given.
    pub folder: String,
    /// The materials read, in palette order.
    pub read: Vec<String>,
    /// The files refused, in name order, each with why. `material.list` lists them too.
    pub refused: Vec<RefusedMaterial>,
}

/// Read a folder's look files into user materials: every `.toml` file in it, in name order
/// (bytes), each a look file (`material.look-recipe`) named by its file name without `.toml`
/// and titled by its recipe's name. They go after the user materials already there and before
/// the built-ins shipped later, with new ids; other files and folders are skipped. A file that
/// cannot be read, is not a look file, or whose name is taken or not one a material may have is
/// refused and listed in `material.list`'s `refused`, and the rest are read. Reading is no step
/// of the palette's undo: it clears that history, as opening a scene does. The window reads its
/// `materials` folder (beside `layouts` and `themes`) this way once at start, and the engine
/// reads each folder read again after an open of a file without a palette.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialFolder {
    /// The folder: an absolute path, as relative ones start at the engine's folder.
    #[schemars(length(min = 1))]
    pub folder: String,
}

impl Command for MaterialFolder {
    const NAME: &'static str = "material.folder";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Short;
    type Output = FolderRead;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "read the window's materials folder (refused here: there is no such folder)",
                json!({ "folder": "/nonexistent/gigasculpt/materials" }),
            )
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<FolderRead, CommandError> {
        let files = look_files(Path::new(&self.folder))?;
        let mut refused = Vec::new();
        let mut texts = Vec::new();
        for file in files {
            match std::fs::read_to_string(Path::new(&self.folder).join(&file)) {
                Ok(text) => texts.push((file, text)),
                Err(e) => refused.push(Refused {
                    file,
                    error: format!("cannot read: {e}"),
                }),
            }
        }
        let pairs: Vec<(&str, &str)> = (texts.iter())
            .map(|(file, text)| (file.as_str(), text.as_str()))
            .collect();
        let library = Library::read(&pairs);
        refused.extend(library.refused);

        let before = cx.palette.clone();
        let mut read = Vec::new();
        for look in library.looks {
            let named =
                |f: &&(String, String)| Path::new(&f.0).file_stem() == Some(look.name.as_ref());
            let file = texts
                .iter()
                .find(named)
                .map(|f| f.0.clone())
                .unwrap_or_default();
            if let Err(e) = cx.palette.now.check_name(&look.name, None) {
                refused.push(Refused {
                    file,
                    error: e.message,
                });
                continue;
            }
            let at = cx.palette.now.user_end();
            let entry = PaletteEntry {
                id: cx.palette.next_id(),
                name: look.name.clone(),
                built_in: false,
                recipe: Arc::new(look.recipe),
            };
            cx.palette.now.materials.insert(at, entry);
            read.push(look.name);
        }
        refused.sort_by(|a, b| a.file.cmp(&b.file));
        cx.palette.now.refused.extend(refused.iter().cloned());
        cx.palette.forget();
        hand_over(cx, before)?;
        if !cx.palette.folders.contains(&self.folder) {
            cx.palette.folders.push(self.folder.clone());
        }
        let refused = (refused.into_iter())
            .map(|r| RefusedMaterial {
                file: r.file,
                error: r.error,
            })
            .collect();
        Ok(FolderRead {
            folder: self.folder,
            read,
            refused,
        })
    }
}
register_command!(MaterialFolder, "Read materials folder");

impl Engine {
    /// After an open that landed the shipped palette in a file's place, reads every user folder
    /// the session read into it again, each a logged `material.folder` run as `origin`, the
    /// open's caller, so a replay of the open's log run reads them too (decision 0043, section
    /// 7). A folder that no longer reads is named in the status line.
    pub(crate) fn read_folders_again(&mut self, origin: &Origin) {
        if !std::mem::take(&mut self.state.palette.bare) {
            return;
        }
        for folder in self.state.palette.folders.clone() {
            let request = Request::Run(RunParams::new(
                MaterialFolder::NAME,
                json!({ "folder": folder }),
            ));
            if let Outcome::Error(e) = self.execute(origin, request).outcome {
                let said = format!("{}: {}", MaterialFolder::NAME, e.message);
                self.state.sculpt.problem = Some(said);
            }
        }
    }
}

/// The names of the `.toml` files in `folder` (the extension in any case), in byte order;
/// folders and names that are not UTF-8 are skipped.
fn look_files(folder: &Path) -> Result<Vec<String>, CommandError> {
    let toml = |name: &String| {
        (Path::new(name).extension()).is_some_and(|x| x.eq_ignore_ascii_case("toml"))
    };
    let shown = folder.display();
    let entries = std::fs::read_dir(folder).map_err(|e| match e.kind() {
        ErrorKind::NotFound => CommandError {
            hint: Some("give an absolute path: relative ones start at the engine's folder".into()),
            ..error(
                ErrorCode::NotFound,
                format!("there is no folder at {shown}"),
            )
        },
        ErrorKind::NotADirectory => error(
            ErrorCode::Precondition,
            format!("{shown} is a file, not a folder"),
        ),
        _ => error(ErrorCode::Precondition, format!("cannot read {shown}: {e}")),
    })?;
    let mut names: Vec<String> = (entries.flatten())
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(toml)
        .collect();
    names.sort();
    Ok(names)
}
