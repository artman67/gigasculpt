//! The manifest: zstd-compressed JSON at the end of the file, before the trailer (decision
//! 0016, section 1). JSON so `file.inspect` and agents can read it and a new field is a default
//! value, not a new layout. Field order is fixed by these structs, so the same scene always
//! writes the same bytes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::FileKind;
use super::levels::LevelsEntry;

/// Chunk kinds this version reads.
pub(crate) const POSITIONS: &str = "positions";
pub(crate) const TOPOLOGY: &str = "topology";
pub(crate) const FREE: &str = "free";
pub(crate) const ATTRIBUTE: &str = "attribute";
pub(crate) const NUMBERING: &str = "numbering";
pub(crate) const PREVIEW: &str = "preview";
/// A Part's detail levels (section 15).
pub(crate) const LEVEL: &str = "level";
/// A Part kept beside the scene, not in it (a Reference View's frozen copy, decision 0024
/// section 8): one chunk, named, holding a Part file of that one Part. It belongs to no entry of
/// `parts` and is optional, so a reader that does not know the kind skips it.
pub(crate) const REFERENCE: &str = "reference";
/// A frozen Part's set-aside levels (decision 0030, section 7): one chunk, belonging to its
/// Part, holding a Part file of the level stack, written and read as a `reference` chunk's.
/// Required: a reader that drops it would lose the levels.
pub(crate) const FROZEN: &str = "frozen_levels";
/// Kinds kept for later features (layers, morph targets, saved history, embedded files). A
/// file that needs one of them is refused by this version.
pub const RESERVED_KINDS: [&str; 4] = ["layer", "morph", "history", "blob"];

/// Codecs.
pub(crate) const ZSTD: &str = "zstd";
pub(crate) const STORED: &str = "stored";

/// The numbering arrays, by the name their chunks give.
pub(crate) const VERTICES: &str = "vertices";
pub(crate) const FACES: &str = "faces";
pub(crate) const FIRST_CORNER: &str = "first_corner";

#[derive(Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub format: u32,
    pub oldest_reader: u32,
    /// The program that wrote the file, with its version.
    pub writer: String,
    pub kind: FileKind,
    /// The document beyond its Parts, as the engine writes it.
    #[serde(default)]
    pub scene: Value,
    /// What a session restores but the scene hash does not cover, as the engine writes it.
    #[serde(default)]
    pub session: Value,
    pub parts: Vec<PartEntry>,
    pub chunks: Vec<ChunkEntry>,
    /// Keys this version does not know: optional data a newer writer added.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, Value>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PartEntry {
    pub name: String,
    /// What the Part remembers besides its mesh, as the engine writes it.
    #[serde(default)]
    pub props: Value,
    /// The node configuration.
    pub shift: u32,
    pub fill: u32,
    pub attributes: Vec<AttrEntry>,
    /// Per node: vertex slots, face slots, free vertex slots, free face slots.
    pub nodes: Vec<[u32; 4]>,
    pub numbering: NumberingEntry,
    /// Its detail levels, when it has them (format 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<LevelsEntry>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AttrEntry {
    pub name: String,
    pub domain: String,
    pub kind: String,
}

impl AttrEntry {
    pub fn of(d: &gs_core::AttrDef) -> AttrEntry {
        AttrEntry {
            name: d.name.clone(),
            domain: d.domain.name().into(),
            kind: d.kind.name().into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum NumberingEntry {
    /// The store's own order.
    Spatial,
    /// The order the Part was made with: its arrays' lengths.
    Given { vertices: u64, faces: u64 },
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ChunkEntry {
    pub kind: String,
    /// The Part it belongs to, by its place in `parts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<u32>,
    /// What it covers: `[first, count]` of nodes for column chunks, of entries for numbering.
    #[serde(default)]
    pub range: [u64; 2],
    /// The attribute or numbering array it holds; for a level chunk, its array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For a level chunk of a numbering array: the level below the top it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// For a level chunk of pending edits to an attribute: the attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    /// For an attribute chunk, or a level chunk of pending edits: the nodes of the range that
    /// hold the column, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<u64>,
    pub filter: String,
    pub codec: String,
    pub offset: u64,
    pub stored: u64,
    pub raw: u64,
    /// BLAKE3 of the stored bytes, first 16 bytes as hex.
    pub hash: String,
    /// A reader that does not know this kind must refuse what it belongs to.
    pub required: bool,
}
