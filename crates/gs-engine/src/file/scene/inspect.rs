//! `file.inspect`: what a scene file holds, without opening it (decision 0016, section 7).

use std::path::PathBuf;

use gs_io::scene::{self, FileKind};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::unreadable;
use super::open::exists;
use crate::{Command, Ctx, Example, register_command};

/// Describe a `.gss` scene or Part file without opening it: its format versions, kind, Parts,
/// chunks and their sizes, preview, how many materials its palette keeps, and the scene and
/// session sections.
///
/// With `verify`, every chunk is read and checked against its hash, and the damaged ones are
/// listed; without it only the header, the manifest and the trailer are read. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInspect {
    /// The file to describe. Give an absolute path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// Read every chunk and check its hash.
    #[serde(default)]
    pub verify: bool,
}

/// A Part in a file.
#[derive(Serialize, JsonSchema)]
pub struct InspectedPart {
    pub name: String,
    pub nodes: u64,
    pub vertex_slots: u64,
    pub face_slots: u64,
    pub free_vertex_slots: u64,
    pub free_face_slots: u64,
    /// `name (domain, kind)` of each attribute column.
    pub attributes: Vec<String>,
    /// `spatial` (the store's order) or `given` (the order it was made with).
    pub numbering: String,
    /// What the Part remembers besides its mesh.
    pub props: Value,
    /// Its detail levels, 1 for a Part without, and the active one, from 1 (the base), as
    /// `level.describe` counts them. The counts above are the top level's.
    pub levels: u64,
    pub active_level: u64,
}

/// The chunks of one kind.
#[derive(Serialize, JsonSchema)]
pub struct InspectedChunks {
    pub kind: String,
    pub count: u64,
    /// Bytes on disk.
    pub stored: u64,
    /// Bytes before compression.
    pub raw: u64,
}

/// What a file holds.
#[derive(Serialize, JsonSchema)]
pub struct Inspected {
    pub path: String,
    /// `scene` or `parts`.
    pub kind: String,
    /// The format version that wrote it, and the oldest that can read it.
    pub format: u32,
    pub oldest_reader: u32,
    /// The program that wrote it.
    pub writer: String,
    pub bytes: u64,
    pub parts: Vec<InspectedPart>,
    pub chunks: Vec<InspectedChunks>,
    /// The preview picture's size in bytes, if it has one.
    pub preview: Option<u64>,
    /// How many materials the scene's palette keeps; `null` for a file without one (format 4
    /// or older, or a Part file). The scene section's `palette` holds them, the session's
    /// `material` the active one's id (decision 0043, section 7).
    pub materials: Option<u64>,
    pub scene: Value,
    pub session: Value,
    /// Optional data this version does not know.
    pub skipped: Vec<String>,
    /// True if every chunk was checked.
    pub verified: bool,
    /// Chunks whose bytes do not match their hash.
    pub damaged: Vec<String>,
}

impl Command for FileInspect {
    const NAME: &'static str = "file.inspect";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    type Output = Inspected;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "check every chunk of a file (refused here: there is no such file)",
                Self {
                    path: "/nonexistent/scene.gss".into(),
                    verify: true,
                },
            )
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, _cx: &mut Ctx) -> Result<Inspected, CommandError> {
        let path = PathBuf::from(&self.path);
        exists(&path)?;
        let i = scene::inspect(&path, self.verify).map_err(|e| unreadable(&path, &e))?;
        let parts = (i.parts.into_iter())
            .map(|p| InspectedPart {
                name: p.name,
                nodes: p.nodes,
                vertex_slots: p.vertex_slots,
                face_slots: p.face_slots,
                free_vertex_slots: p.free_vertex_slots,
                free_face_slots: p.free_face_slots,
                attributes: p.attributes,
                numbering: p.numbering,
                props: p.props,
                levels: p.levels,
                active_level: p.active_level + 1,
            })
            .collect();
        let chunks = (i.chunks.into_iter())
            .map(|c| InspectedChunks {
                kind: c.kind,
                count: c.count,
                stored: c.stored,
                raw: c.raw,
            })
            .collect();
        Ok(Inspected {
            path: self.path,
            kind: json!(if i.header.kind == FileKind::Scene {
                "scene"
            } else {
                "parts"
            })
            .as_str()
            .unwrap_or_default()
            .into(),
            format: i.header.format,
            oldest_reader: i.header.oldest_reader,
            writer: i.writer,
            bytes: i.bytes,
            parts,
            chunks,
            preview: i.preview,
            materials: (i.scene.pointer("/palette/materials"))
                .and_then(Value::as_array)
                .map(|m| m.len() as u64),
            scene: i.scene,
            session: i.session,
            skipped: i.skipped,
            verified: self.verify,
            damaged: i.damaged,
        })
    }
}
register_command!(FileInspect, "Inspect file");
