//! `file.inspect` (decision 0016): what a file holds, from its header, manifest and chunk
//! table, without decoding a Part; with `verify`, every chunk's hash is checked too.

use std::path::Path;

use serde_json::Value;

use super::SceneError;
use super::layout::Header;
use super::manifest::{NumberingEntry, PartEntry};
use super::read::{Bytes, KNOWN, Source, budget, chunk_name, file, hashed, open, placed};

/// A Part as `file.inspect` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartSummary {
    pub name: String,
    pub nodes: u64,
    pub vertex_slots: u64,
    pub face_slots: u64,
    pub free_vertex_slots: u64,
    pub free_face_slots: u64,
    /// `name (domain, kind)` of each attribute column.
    pub attributes: Vec<String>,
    /// `spatial` or `given`.
    pub numbering: String,
    pub props: Value,
    /// Its detail levels, 1 for a Part without, and the active one.
    pub levels: u64,
    pub active_level: u64,
}

/// The chunks of one kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkSummary {
    pub kind: String,
    pub count: u64,
    pub stored: u64,
    pub raw: u64,
}

/// What a file holds, read without opening it.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    pub header: Header,
    pub writer: String,
    pub bytes: u64,
    pub scene: Value,
    pub session: Value,
    pub parts: Vec<PartSummary>,
    pub chunks: Vec<ChunkSummary>,
    /// The preview's size in bytes.
    pub preview: Option<u64>,
    pub skipped: Vec<String>,
    /// With `verify`: every chunk whose bytes do not match its hash, lie outside the file's
    /// chunks, or have a codec its kind does not use: what an open would refuse it for.
    pub damaged: Vec<String>,
}

/// Reads the header, the manifest and the chunk table of the file at `path`; with `verify`,
/// also every chunk, checking its hash.
pub fn inspect(path: &Path, verify: bool) -> Result<Inspection, SceneError> {
    inspect_from(&file(path)?, verify)
}

/// [`inspect`] for a file held in memory.
pub fn inspect_bytes(bytes: &[u8], verify: bool) -> Result<Inspection, SceneError> {
    inspect_from(&Bytes(bytes), verify)
}

fn inspect_from(src: &dyn Source, verify: bool) -> Result<Inspection, SceneError> {
    let o = open(src, &budget(src))?;
    let man = &o.manifest;
    let mut kinds: std::collections::BTreeMap<&str, ChunkSummary> =
        std::collections::BTreeMap::new();
    for c in &man.chunks {
        let s = kinds.entry(&c.kind).or_insert_with(|| ChunkSummary {
            kind: c.kind.clone(),
            count: 0,
            stored: 0,
            raw: 0,
        });
        s.count += 1;
        s.stored = s.stored.saturating_add(c.stored);
        s.raw = s.raw.saturating_add(c.raw);
    }
    let chunks: Vec<ChunkSummary> = kinds.into_values().collect();
    let damaged = if verify {
        let bad = gs_par::map(man.chunks.len(), |i| {
            let c = &man.chunks[i];
            let ok = placed(&o.trailer, c).is_ok() && hashed(src, c).is_ok_and(|same| same);
            let part = (c.part.and_then(|p| man.parts.get(p as usize)))
                .map(|p| format!(" of part '{}'", p.name));
            (!ok).then(|| {
                format!(
                    "{}{} at byte {}",
                    chunk_name(c),
                    part.unwrap_or_default(),
                    c.offset
                )
            })
        });
        bad.into_iter().flatten().collect()
    } else {
        Vec::new()
    };
    let known = |k: &String| KNOWN.contains(&k.as_str());
    let mut skipped: Vec<String> = man
        .unknown
        .keys()
        .map(|k| format!("manifest field '{k}'"))
        .collect();
    skipped.extend(
        chunks
            .iter()
            .filter(|s| !known(&s.kind))
            .map(|s| format!("{} chunk", s.kind)),
    );
    Ok(Inspection {
        header: o.header,
        writer: man.writer.clone(),
        bytes: src.size(),
        scene: man.scene.clone(),
        session: man.session.clone(),
        parts: man.parts.iter().map(summary).collect(),
        preview: (o.header.preview.1 > 0).then_some(o.header.preview.1),
        chunks,
        skipped,
        damaged,
    })
}

fn summary(e: &PartEntry) -> PartSummary {
    let sum = |i: usize| e.nodes.iter().map(|r| u64::from(r[i])).sum();
    PartSummary {
        name: e.name.clone(),
        nodes: e.nodes.len() as u64,
        vertex_slots: sum(0),
        face_slots: sum(1),
        free_vertex_slots: sum(2),
        free_face_slots: sum(3),
        attributes: (e.attributes.iter())
            .map(|a| format!("{} ({}, {})", a.name, a.domain, a.kind))
            .collect(),
        numbering: match e.numbering {
            NumberingEntry::Spatial => "spatial".into(),
            NumberingEntry::Given { .. } => "given".into(),
        },
        props: e.props.clone(),
        levels: e.levels.as_ref().map_or(1, |l| l.lower.len() as u64 + 1),
        active_level: e.levels.as_ref().map_or(0, |l| u64::from(l.active)),
    }
}
