//! Our scene and Part files, `.gss` (decision 0016): the container, its codecs and manifest,
//! the writer, the reader, `inspect`, and the safe replace that never risks the last good file.
//!
//! ```text
//! header 64 bytes | preview PNG | chunks, each 8-byte aligned | manifest (zstd JSON) | trailer 64
//! ```
//!
//! A Part is written straight from its `MeshStore`, exactly as the engine holds it: every
//! node's slots, dead ones included, its free lists, every column bit for bit, and only the
//! attribute columns each node has. Reading gives the same columns back as `ExactNode`s, which
//! `MeshStore::from_exact` checks and turns into a store. What a Part remembers (`props`) and
//! the scene and session sections are JSON the engine writes; this module carries them as they
//! are.
//!
//! A Part with detail levels also stores its levels (section 15): the top level is its store,
//! and the rest is gs-core's `StoredLevels`, written as chunks of kind `level` and read back
//! the same, for `Levels::from_file` to check and rebuild.
//!
//! A file can also keep Parts beside the scene, not in it (a Reference View's frozen copies,
//! decision 0024 section 8): each is one optional chunk of kind `reference`, named, holding a
//! Part file of that one Part, written and read by this same module. They are not in the
//! manifest's `parts`, so a reader that does not know the kind skips them. The reader reads
//! only the ones the caller names (`Kept`).
//!
//! A frozen Part (decision 0030, section 7) keeps its set-aside level stack the same way, as a
//! Part file of one Part in a chunk of kind `frozen_levels`, but belonging to its Part and
//! required, and always read; a file with one needs format 3. A kept or frozen Part's file
//! keeps neither of its own, so nothing nests deeper than one file.
//!
//! The reader never panics on bad input: every size it allocates is first checked against the
//! Part's node table and the file's own size, every chunk against its hash, and every error
//! names the byte offset, the chunk or the Part. A damaged chunk refuses only its Part.

mod codec;
mod decode;
mod inspect;
mod layout;
mod levels;
mod manifest;
mod read;
mod replace;
mod write;

use std::fmt;

use gs_core::{AttrDef, ExactNode, MeshStore, NodeConfig, StoredLevels};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use inspect::{ChunkSummary, Inspection, PartSummary, inspect, inspect_bytes};
pub use layout::Header;
pub use manifest::RESERVED_KINDS;
pub use read::{Kept, read, read_bytes};
pub use replace::{Disk, DiskFile, RealDisk, backup_path, replace};
pub use write::{Written, write};

/// The file name extension.
pub const EXTENSION: &str = "gss";
/// The format version this build writes, and the newest it reads. 5 adds the material palette
/// to the scene section, optional data that leaves `oldest_reader` as it was.
pub const FORMAT: u32 = 5;

/// The oldest format version that reads a file of `parts` (decision 0016, section 3): 4 when
/// a level keeps its own UVs (decision 0014's smooth UVs), 3 when a Part is frozen (decision
/// 0030), 2 when one has detail levels, else 1.
pub fn oldest_reader(parts: &[PartOut]) -> u32 {
    let own_uvs = |p: &PartOut| {
        (p.levels
            .into_iter()
            .chain(p.frozen.as_ref().and_then(|f| f.levels)))
        .any(|l| l.lower.iter().any(|s| !s.uvs.is_empty()))
    };
    if parts.iter().any(own_uvs) {
        4
    } else if parts.iter().any(|p| p.frozen.is_some()) {
        3
    } else if parts.iter().any(|p| p.levels.is_some()) {
        2
    } else {
        1
    }
}

/// A whole scene, or only some Parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Scene,
    /// A Part file: one or more Parts and nothing else.
    Parts,
}

impl FileKind {
    fn code(self) -> u32 {
        match self {
            FileKind::Scene => 0,
            FileKind::Parts => 1,
        }
    }
    fn of_code(code: u32) -> Option<FileKind> {
        [FileKind::Scene, FileKind::Parts]
            .into_iter()
            .find(|k| k.code() == code)
    }
}

/// How a Part's vertices are numbered outside its store, to write.
#[derive(Clone, Copy, Debug)]
pub enum NumberingOut<'a> {
    /// The store's own order.
    Spatial,
    /// The order the Part was made with: each vertex's and face's index in store order, and
    /// which corner each stored face starts with.
    Given {
        vertices: &'a [u32],
        faces: &'a [u32],
        first_corner: &'a [u8],
    },
}

/// A Part's numbering, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberingIn {
    Spatial,
    Given {
        vertices: Vec<u32>,
        faces: Vec<u32>,
        first_corner: Vec<u8>,
    },
}

/// One Part to write.
pub struct PartOut<'a> {
    pub name: &'a str,
    pub props: Value,
    pub store: &'a MeshStore,
    pub numbering: NumberingOut<'a>,
    /// Its detail levels, with `store` the top level and `numbering` the top's (given).
    pub levels: Option<&'a StoredLevels>,
    /// The level stack `level.freeze` set aside, as a Part with levels (decision 0030).
    pub frozen: Option<Box<PartOut<'a>>>,
}

/// A scene or Part file to write.
pub struct SceneOut<'a> {
    pub kind: FileKind,
    /// The program writing it, with its version.
    pub writer: String,
    /// A PNG picture, stored as it is.
    pub preview: Option<&'a [u8]>,
    pub scene: Value,
    pub session: Value,
    pub parts: Vec<PartOut<'a>>,
    /// Parts kept beside the scene, each under the name its chunk gets: never in the scene.
    pub references: Vec<(String, PartOut<'a>)>,
}

/// One Part, read: its columns as the store held them, not yet checked as a store.
#[derive(Debug)]
pub struct PartIn {
    pub name: String,
    pub props: Value,
    pub config: NodeConfig,
    pub attrs: Vec<AttrDef>,
    pub nodes: Vec<ExactNode>,
    pub numbering: NumberingIn,
    /// Its detail levels, not yet checked as a stack; `nodes` are the top level's.
    pub levels: Option<StoredLevels>,
    /// Its frozen level stack (decision 0030), read as a Part with levels.
    pub frozen: Option<Box<PartIn>>,
}

/// A file, read.
#[derive(Debug)]
pub struct SceneIn {
    pub header: Header,
    pub writer: String,
    pub scene: Value,
    pub session: Value,
    /// Every Part in file order, or why it could not be read.
    pub parts: Vec<Result<PartIn, PartError>>,
    pub preview: Option<Vec<u8>>,
    /// Optional data this version does not know and left out, for the open report.
    pub skipped: Vec<String>,
    /// The Parts kept beside the scene, by their chunk's name, or why one could not be read.
    pub references: Vec<(String, Result<PartIn, PartError>)>,
}

/// Why a whole file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// The file does not start with the scene file signature.
    NotScene,
    /// The file needs a newer program: its oldest reader version is above ours.
    TooNew { needs: u32 },
    /// The file was cut short or never finished: no valid trailer.
    Incomplete { reason: String },
    /// The header or the manifest is damaged.
    Damaged { offset: u64, reason: String },
    /// The operating system could not read the file.
    Io { offset: u64, message: String },
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotScene => write!(f, "not a scene file: the signature is missing"),
            Self::TooNew { needs } => write!(
                f,
                "the file needs scene file format {needs} or newer; this program reads up to {FORMAT}"
            ),
            Self::Incomplete { reason } => write!(f, "the file is incomplete: {reason}"),
            Self::Damaged { offset, reason } => write!(f, "damaged at byte {offset}: {reason}"),
            Self::Io { offset, message } => write!(f, "could not read byte {offset}: {message}"),
        }
    }
}

impl std::error::Error for SceneError {}

/// Why one Part could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartError {
    pub part: String,
    /// The chunk, as `kind` and what it covers, when one chunk is the cause.
    pub chunk: Option<String>,
    /// Where that chunk starts.
    pub offset: Option<u64>,
    pub reason: String,
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "part '{}'", self.part)?;
        if let Some(chunk) = &self.chunk {
            write!(f, ", {chunk}")?;
        }
        if let Some(at) = self.offset {
            write!(f, " at byte {at}")?;
        }
        write!(f, ": {}", self.reason)
    }
}
