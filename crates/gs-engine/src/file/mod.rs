//! Files in and out: `file.import` (OBJ, PLY, STL, ZTL tools and ZPR projects) and
//! `file.export` (OBJ, PLY, STL), and our own scene files (`scene`: `file.save`, `file.open`,
//! autosave and recovery, decision 0016). The formats themselves live in gs-io; this folder
//! turns files into parts and parts into files, with the import and export options of the
//! catalog (`tool-misc.import-options`, `tool-misc.export-options`).
//!
//! Paths come from the caller. A relative path is taken from the engine's working folder, which
//! is not the caller's, so callers should give absolute paths. An import pins the file's content
//! hash before it runs (`Command::READS_FILE`), so a replay notices a changed file (ARCHITECTURE
//! 8). An export is a query: it changes no part, so it is not logged and a replay never writes
//! files. It refuses to replace a file unless asked.

mod clean;
mod export;
mod import;
mod scene;
mod ztl;

pub(crate) use export::save as save_file;
pub(crate) use import::FileImport;
pub(crate) use scene::{Autosave, FileState, Landing, marked_logs, newest_autosave};
pub use scene::{AutosaveSettings, FreeSpace, StateFolder};
pub(crate) use scene::{LowDisk, RecentFile, low_disk, shown_recent};

use std::fs::File;
use std::io;
use std::path::Path;

use gs_protocol::{CommandError, ErrorCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::{self, error};

/// A file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Wavefront OBJ.
    Obj,
    /// Stanford PLY.
    Ply,
    /// STL.
    Stl,
    /// A ZTL tool file (import only).
    Ztl,
    /// A ZPR project file (import only).
    Zpr,
}

impl Format {
    /// The format a file name's extension names, if any.
    fn of_path(path: &str) -> Option<Self> {
        let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "obj" => Self::Obj,
            "ply" => Self::Ply,
            "stl" => Self::Stl,
            "ztl" => Self::Ztl,
            "zpr" => Self::Zpr,
            _ => return None,
        })
    }

    /// The format given, or the one the extension names.
    fn resolve(given: Option<Self>, path: &str) -> Result<Self, CommandError> {
        given
            .or_else(|| Self::of_path(path))
            .ok_or_else(|| CommandError {
                hint: Some("name the format: obj, ply, stl, ztl or zpr".into()),
                ..errors::args_invalid(
                    "/format",
                    format!("the format of {path} is not known from its name"),
                )
            })
    }
}

/// Which way is up in a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpAxis {
    /// +Y is up, as in the scene.
    #[default]
    Y,
    /// +Z is up (CAD and most 3D printing software): the file's +Z is the scene's +Y and its
    /// -Y the scene's +Z.
    Z,
}

impl UpAxis {
    /// A file position in scene axes.
    fn to_scene(self, p: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Y => p,
            Self::Z => [p[0], p[2], -p[1]],
        }
    }

    /// A scene position in the file's axes.
    fn to_file(self, p: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Y => p,
            Self::Z => [p[0], -p[2], p[1]],
        }
    }
}

/// Negates the coordinates `flip` marks.
fn mirror(p: [f32; 3], flip: [bool; 3]) -> [f32; 3] {
    std::array::from_fn(|i| if flip[i] { -p[i] } else { p[i] })
}

/// The BLAKE3 hash of a file's bytes, as 64 hex digits.
pub(crate) fn content_hash(path: &str) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// A file that could not be opened or read, as a command error.
pub(crate) fn io_error(path: &str, e: &io::Error) -> CommandError {
    match e.kind() {
        io::ErrorKind::NotFound => CommandError {
            hint: Some("give an absolute path: relative ones start at the engine's folder".into()),
            ..error(ErrorCode::NotFound, format!("there is no file at {path}"))
        },
        _ => error(ErrorCode::Precondition, format!("cannot use {path}: {e}")),
    }
}
