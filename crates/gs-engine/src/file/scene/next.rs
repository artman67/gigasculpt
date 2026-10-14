//! `file.save_next` (decision 0016, section 16; for Part files, section 17, item 8): a save
//! under the next numbered name.

use std::path::{Path, PathBuf};

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::save::{FileSave, SavedFile};
use crate::errors;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// How many digits a name without a number gets: the catalog's Save Next suffix width.
const WIDTH: usize = 3;

/// Save the scene, or some Parts as a Part file, under the next numbered name, in the
/// background.
///
/// The number at the end of the name it counts from goes up by one and keeps its width
/// (`head_001.gss` becomes `head_002.gss`, `take7.gss` becomes `take8.gss`); a name without a
/// number gets `_001`. A name a file already has is passed over, so it never writes over a
/// file. It counts from `path` when given, else from the scene's own file, or with `parts` from
/// the Part file saved last in this session; with none of these it is refused naming `path`.
/// Otherwise it is `file.save` to the new name: a scene file becomes the scene's own, a Part
/// file leaves the scene's file as it was, and the reply says which name was chosen.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSaveNext {
    /// The file to count from. Left out, the scene's own file, or with `parts` the Part file
    /// saved last in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
    /// Write only these Parts, as a Part file, as `file.save` does (`@active` is the active
    /// Part). The scene keeps its own file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PartRef>>,
    /// Keep the open Reference Views in a scene file, as `file.save` does.
    #[serde(default = "crate::command::yes")]
    pub references: bool,
}

impl FileSaveNext {
    /// The file the next name counts from, if there is one.
    fn base(&self, cx: &Ctx) -> Option<PathBuf> {
        let own = match self.parts {
            Some(_) => &cx.file.parts_path,
            None => &cx.file.path,
        };
        self.path
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| own.clone())
    }
}

impl Command for FileSaveNext {
    const NAME: &'static str = "file.save_next";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    const BACKGROUND: bool = true;
    type Output = SavedFile;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "save the next numbered copy (refused here: the scene has no file yet)",
                json!({}),
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::raw(
                "save the active Part as the next Part file after arm.gss (refused here: the \
                 folder does not exist)",
                json!({ "parts": ["@active"], "path": "/nonexistent/folder/arm.gss" }),
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    /// Without a file to count from it is refused before a job starts, naming `path`, so the
    /// window asks for one (decision 0016, section 17).
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        if self.base(cx).is_some() {
            return Ok(());
        }
        let why = match self.parts {
            Some(_) => "no Part file saved yet in this session, so no name to count from",
            None => "the scene has no file yet, so no name to count from",
        };
        Err(Unavailable::new(why)
            .fix("give the path to count from")
            .asks("path"))
    }

    fn run(self, cx: &mut Ctx) -> Result<SavedFile, CommandError> {
        cx.settled()?;
        self.check(cx).map_err(errors::unavailable)?;
        let base = self.base(cx);
        let base = base.ok_or_else(|| errors::internal("checked: a file to count from"))?;
        let save = FileSave {
            path: Some(next_free(&base).display().to_string()),
            parts: self.parts,
            overwrite: false,
            backup: false,
            upgrade: false,
            references: self.references,
        };
        save.run(cx)
    }
}
register_command!(FileSaveNext, "Save next numbered");

/// The first numbered name after `path`'s that no file has.
fn next_free(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let head = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &stem[head.len()..];
    let (head, width, mut n) = match digits.parse::<u64>() {
        Ok(n) => (head.to_string(), digits.len(), n),
        Err(_) => (format!("{stem}_"), WIDTH, 0),
    };
    loop {
        n = n.saturating_add(1);
        let next = path.with_file_name(format!("{head}{n:0width$}.gss"));
        // A dangling link counts as taken: writing there would replace it.
        if std::fs::symlink_metadata(&next).is_err() || n == u64::MAX {
            return next;
        }
    }
}
