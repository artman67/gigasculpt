//! The manifest (decision 0032, section 1): a short JSON description of the column files in an
//! exchange folder, written last by whichever side writes the folder.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use gs_protocol::{CommandError, ErrorCode};
use serde::{Deserialize, Serialize};

use super::Form;
use super::names::{Column, Domain, Elem, Place, TABLE};
use crate::errors::error;
use crate::file::{file_refused, io_error};

/// The manifest's file name in its folder.
pub const MANIFEST: &str = "manifest.json";
/// The format name every manifest carries.
pub const FORMAT: &str = "gigasculpt-exchange";
/// The format version this engine writes and reads.
pub const VERSION: u32 = 1;

/// A folder's manifest.
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// The program that wrote it, with its version.
    pub writer: String,
    pub parts: Vec<Entry>,
}

/// One part, or one Blender object.
#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    /// The part's stable id, as a string; none for an object GigaSculpt never sent.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// The part's place in the list (sent only).
    #[serde(default)]
    pub rank: u64,
    /// The part's eye is closed (sent only).
    #[serde(default)]
    pub hidden: bool,
    /// The part's paint is hidden (Show Paint off), so its colors go but the add-on's render
    /// switch `gigasculpt_paint` is 0 (sent only, and only while hidden).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paint_hidden: bool,
    pub form: Form,
    /// Detail levels, the base included. A part sent in the Mesh form gives its own count, which
    /// only the Levels form acts on; an object Blender sends in the Mesh form gives 1.
    pub levels: u32,
    pub transform: Place,
    pub points: u64,
    pub faces: u64,
    pub corners: u64,
    /// The base's creased edges in the Levels form: both ends, as indices of the first points,
    /// and Blender's crease value.
    #[serde(default)]
    pub creases: Vec<(u32, u32, f64)>,
    pub columns: Vec<ColumnFile>,
}

/// One column file.
#[derive(Debug, Serialize, Deserialize)]
pub struct ColumnFile {
    pub gs: String,
    pub blender: String,
    pub domain: Domain,
    #[serde(rename = "type")]
    pub elem: Elem,
    pub width: u32,
    pub file: String,
}

impl ColumnFile {
    pub fn of(c: Column, file: String) -> ColumnFile {
        ColumnFile {
            gs: c.gs.into(),
            blender: c.blender.into(),
            domain: c.domain,
            elem: c.elem,
            width: c.width,
            file,
        }
    }

    /// The table's row for this file, if both names, the domain, type and width agree with it.
    pub fn row(&self) -> Option<Column> {
        TABLE.into_iter().find(|c| {
            c.gs == self.gs
                && c.blender == self.blender
                && c.domain == self.domain
                && c.elem == self.elem
                && c.width == self.width
        })
    }
}

/// Whether `name` is a plain file name in the folder: letters, digits, `.`, `_` and `-`, not
/// starting with a dot, so a manifest can never name a file elsewhere.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && (name.bytes()).all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The files a manifest lists, in order. `None` if it names a file outside its folder.
pub fn listed(manifest: &serde_json::Value) -> Option<Vec<String>> {
    let parts = manifest.get("parts")?.as_array()?;
    let mut out = Vec::new();
    for part in parts {
        for column in part.get("columns")?.as_array()? {
            let file = column.get("file")?.as_str()?;
            if !plain_name(file) {
                return None;
            }
            out.push(file.to_string());
        }
    }
    Some(out)
}

/// One BLAKE3 hash of the manifest's bytes and then every file it lists, in order, as 64 hex
/// digits. `None` when the folder cannot be read as an exchange folder: the command reports why.
pub fn bundle_hash(folder: &Path) -> Option<String> {
    let bytes = fs::read(folder.join(MANIFEST)).ok()?;
    let files = listed(&serde_json::from_slice(&bytes).ok()?)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&bytes);
    for file in files {
        io::copy(&mut File::open(folder.join(file)).ok()?, &mut hasher).ok()?;
    }
    Some(hasher.finalize().to_hex().to_string())
}

/// A folder as one read: the manifest, every file it lists, and the hash of both as
/// [`bundle_hash`] works it out, from the same bytes the call decodes.
pub struct Bundle {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
    pub hash: String,
}

/// Reads an exchange folder: its manifest, checked for the format and version, and every file
/// it lists.
pub fn load(folder: &Path) -> Result<Bundle, CommandError> {
    let path = folder.join(MANIFEST);
    let shown = path.display().to_string();
    let bytes = fs::read(&path).map_err(|e| io_error(&shown, &e))?;
    let bad = |why: String| error(ErrorCode::Precondition, format!("{shown}: {why}"));
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| bad(e.to_string()))?;
    let files = listed(&value).ok_or_else(|| bad("it names a file outside its folder".into()))?;
    let manifest: Manifest = serde_json::from_value(value).map_err(|e| bad(e.to_string()))?;
    if manifest.format != FORMAT || manifest.version != VERSION {
        return Err(bad(format!(
            "it is {} version {}; this engine reads {FORMAT} version {VERSION}",
            manifest.format, manifest.version
        )));
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(&bytes);
    let mut read = BTreeMap::new();
    for file in files {
        let at = folder.join(&file);
        let data = fs::read(&at).map_err(|e| io_error(&at.display().to_string(), &e))?;
        hasher.update(&data);
        read.insert(file, data);
    }
    Ok(Bundle {
        manifest,
        files: read,
        hash: hasher.finalize().to_hex().to_string(),
    })
}

/// Writes the manifest into `folder` through a file beside it that then takes its place, so a
/// reader never sees half a manifest.
pub fn put_manifest(folder: &Path, manifest: &Manifest) -> Result<(), CommandError> {
    let path = folder.join(MANIFEST);
    let temp = folder.join(format!(".{MANIFEST}.{}.part", std::process::id()));
    let json = serde_json::to_vec_pretty(manifest)
        .map_err(|e| error(ErrorCode::Internal, format!("manifest: {e}")))?;
    let written = fs::write(&temp, json).and_then(|()| fs::rename(&temp, &path));
    written.map_err(|e| {
        fs::remove_file(&temp).ok();
        file_refused("write", &path, &e)
    })
}

/// Writes one column file of little-endian values.
pub fn put_column<T: Copy>(
    folder: &Path,
    name: &str,
    values: &[T],
    bytes: impl Fn(T) -> [u8; 4],
) -> Result<u64, CommandError> {
    let path = folder.join(name);
    let refused = |e: io::Error| file_refused("write", &path, &e);
    let mut out = BufWriter::new(File::create(&path).map_err(refused)?);
    for &v in values {
        out.write_all(&bytes(v)).map_err(refused)?;
    }
    out.flush().map_err(refused)?;
    Ok(4 * values.len() as u64)
}

/// Writes one column file of bytes.
pub fn put_bytes(folder: &Path, name: &str, values: &[u8]) -> Result<u64, CommandError> {
    let path = folder.join(name);
    fs::write(&path, values).map_err(|e| file_refused("write", &path, &e))?;
    Ok(values.len() as u64)
}
