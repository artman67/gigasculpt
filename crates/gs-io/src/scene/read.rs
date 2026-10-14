//! Reading a scene file (decision 0016): header, trailer and manifest, then each Part's chunks
//! in parallel straight into node columns, its frozen levels, and the kept Parts asked for.
//! Nothing is trusted before it is checked: the manifest by its hash, every chunk by its hash and
//! by the size the Part's node table gives it, so no size read from the file is allocated before
//! it is known to fit what the file claims to hold. Before any chunk is decoded, the chunk table
//! itself is checked (section 2's limits): no two chunks share bytes, every node has its columns,
//! and what the Parts claim to hold fits a file of this size. A Part whose chunks are missing,
//! damaged or of a kind this version does not know is refused by name; the others read.
//!
//! This file opens a file and holds its budget, and reads the files nested in it (kept Parts and
//! frozen levels); `decode` turns one Part's chunks into its columns, and `inspect` reads a file
//! without decoding its Parts.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::path::Path;

use serde_json::Value;

use super::codec;
use super::decode::read_part;
use super::layout::{HEADER, Header, TRAILER, Trailer};
use super::manifest::{self as m, ChunkEntry, Manifest, PartEntry};
use super::{FORMAT, FileKind, PartError, PartIn, SceneError, SceneIn};

/// The largest manifest accepted, uncompressed. Measured 2026-10-11 with
/// `gs-engine/examples/scene_files.rs`: 0.50 MB for a part of 100 million triangles, 0.35 MB for
/// one of 50 million with five detail levels and every point of one an edit; about 5 KB per
/// million triangles, so this is room for some 3 billion.
const MAX_MANIFEST: u64 = 16 << 20;
/// How much larger than its JSON a manifest may be once read: a two-byte value such as `0,`
/// becomes a 32-byte `Value`. A manifest is charged at this much against the budget.
const GROWTH: u64 = (size_of::<Value>() / 2) as u64;
/// The largest preview accepted.
const MAX_PREVIEW: u64 = 16 << 20;
/// What a file's Parts, its kept Parts included, may hold uncompressed (decision 0016, section
/// 2): this much, plus [`RATIO`] times the file's size.
const FLOOR: u64 = 64 << 20;
/// Sculpts measure 4 to 6 to 1 and the most compressible file measured, a flat grid with eight
/// constant columns, 131 to 1 (decision 0016, "Measurements"); zstd alone reaches 10,000 to 1.
const RATIO: u64 = 1024;
/// How much of a chunk is read at a time to check its hash.
const PIECE: u64 = 16 << 20;
/// Chunk kinds this version reads.
pub(super) const KNOWN: [&str; 9] = [
    m::POSITIONS,
    m::TOPOLOGY,
    m::FREE,
    m::ATTRIBUTE,
    m::NUMBERING,
    m::PREVIEW,
    m::LEVEL,
    m::REFERENCE,
    m::FROZEN,
];

/// Where the bytes come from: a file read at offsets, or bytes in memory.
pub(crate) trait Source: Sync {
    fn size(&self) -> u64;
    fn at(&self, offset: u64, len: u64) -> Result<Vec<u8>, SceneError>;
}

/// Bytes in memory.
pub(super) struct Bytes<'a>(pub(super) &'a [u8]);

impl Source for Bytes<'_> {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn at(&self, offset: u64, len: u64) -> Result<Vec<u8>, SceneError> {
        let range = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(offset.saturating_add(len)).ok());
        range
            .and_then(|(a, b)| self.0.get(a..b))
            .map(<[u8]>::to_vec)
            .ok_or(SceneError::Incomplete {
                reason: format!("byte {offset} is past the end"),
            })
    }
}

/// Part of another source, read as a file of its own: a `reference` chunk.
struct Window<'a> {
    src: &'a dyn Source,
    at: u64,
    len: u64,
}

impl Source for Window<'_> {
    fn size(&self) -> u64 {
        self.len
    }
    fn at(&self, offset: u64, len: u64) -> Result<Vec<u8>, SceneError> {
        if offset.saturating_add(len) > self.len {
            return Err(SceneError::Incomplete {
                reason: format!("byte {offset} is past the end"),
            });
        }
        self.src.at(self.at + offset, len)
    }
}

pub(super) struct FileSource {
    file: File,
    size: u64,
}

impl Source for FileSource {
    fn size(&self) -> u64 {
        self.size
    }
    fn at(&self, offset: u64, len: u64) -> Result<Vec<u8>, SceneError> {
        if offset.saturating_add(len) > self.size {
            return Err(SceneError::Incomplete {
                reason: format!("byte {offset} is past the end"),
            });
        }
        let mut buf = vec![0u8; len as usize];
        read_at(&self.file, &mut buf, offset).map_err(|e| SceneError::Io {
            offset,
            message: e.to_string(),
        })?;
        Ok(buf)
    }
}

/// Fills `buf` from `file` at `offset` without a shared cursor, so many threads read one file at
/// once.
#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

/// Fills `buf` from `file` at `offset`. Windows' positional read also moves the file's cursor,
/// which nothing here uses, and may return fewer bytes than asked: read on until `buf` is full.
#[cfg(windows)]
fn read_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub(super) fn file(path: &Path) -> Result<FileSource, SceneError> {
    let io = |e: io::Error| SceneError::Io {
        offset: 0,
        message: e.to_string(),
    };
    let file = File::open(path).map_err(io)?;
    let size = file.metadata().map_err(io)?.len();
    Ok(FileSource { file, size })
}

/// The header, trailer and manifest of a file, checked.
pub(super) struct Opened {
    pub(super) header: Header,
    pub(super) trailer: Trailer,
    pub(super) manifest: Manifest,
}

/// Checks and reads the header, trailer and manifest, charging what the manifest may grow to
/// once read against `budget` before it is decompressed (review 0007, M1).
pub(super) fn open(src: &dyn Source, budget: &Cell<u64>) -> Result<Opened, SceneError> {
    let size = src.size();
    let start = src.at(0, HEADER.min(size as usize) as u64)?;
    let header = Header::read(&start)?;
    if header.oldest_reader > FORMAT {
        return Err(SceneError::TooNew {
            needs: header.oldest_reader,
        });
    }
    if size < (HEADER + TRAILER) as u64 {
        return Err(SceneError::Incomplete {
            reason: "the file ends before its trailer".into(),
        });
    }
    let trailer = Trailer::read(&src.at(size - TRAILER as u64, TRAILER as u64)?, size)?;
    let damaged = |reason: &str| SceneError::Damaged {
        offset: trailer.manifest_at,
        reason: reason.into(),
    };
    if trailer.manifest_raw > MAX_MANIFEST {
        return Err(damaged("the manifest claims to be larger than any scene's"));
    }
    let read = trailer.manifest_raw.saturating_mul(GROWTH);
    charge(budget, read, "its manifest takes", trailer.manifest_at)?;
    let stored = src.at(trailer.manifest_at, trailer.manifest_stored)?;
    if blake3::hash(&stored).as_bytes()[..16] != trailer.manifest_hash {
        return Err(damaged("the manifest's hash does not match"));
    }
    let json = codec::decompress(&stored, trailer.manifest_raw as usize)
        .ok_or_else(|| damaged("the manifest does not decompress"))?;
    let manifest: Manifest = serde_json::from_slice(&json)
        .map_err(|e| damaged(&format!("the manifest does not read: {e}")))?;
    if (manifest.format, manifest.oldest_reader, manifest.kind)
        != (header.format, header.oldest_reader, header.kind)
    {
        return Err(damaged("the header and the manifest disagree"));
    }
    let mut spans: Vec<(u64, u64)> = (manifest.chunks.iter())
        .filter(|c| c.stored > 0)
        .map(|c| (c.offset, c.offset.saturating_add(c.stored)))
        .collect();
    spans.sort_unstable();
    if let Some(w) = spans.windows(2).find(|w| w[1].0 < w[0].1) {
        return Err(SceneError::Damaged {
            offset: w[1].0,
            reason: "two chunks share bytes, which no writer does".into(),
        });
    }
    Ok(Opened {
        header,
        trailer,
        manifest,
    })
}

/// Which of the Parts a file keeps beside the scene (decision 0024, section 8) a read decodes.
/// The others are named in `skipped`.
#[derive(Clone, Copy)]
pub enum Kept<'a> {
    /// None of them.
    None,
    /// Those this function finds named in the file's session section.
    Named(&'a dyn Fn(&Value) -> Vec<String>),
}

/// Reads the file at `path`.
pub fn read(path: &Path, kept: Kept) -> Result<SceneIn, SceneError> {
    let src = file(path)?;
    read_from(&src, &budget(&src), Some(kept))
}

/// Reads a file held in memory.
pub fn read_bytes(bytes: &[u8], kept: Kept) -> Result<SceneIn, SceneError> {
    let src = Bytes(bytes);
    read_from(&src, &budget(&src), Some(kept))
}

/// What a file of this size may hold uncompressed, its manifest, Parts and kept Parts together.
pub(super) fn budget(src: &dyn Source) -> Cell<u64> {
    Cell::new(FLOOR.saturating_add(RATIO.saturating_mul(src.size())))
}

/// `budget`: what is left of what the outermost file may hold uncompressed, shared with the
/// Parts it keeps. `kept`: `None` when the file is a `reference` chunk's, which may keep no
/// Parts of its own (one level, so a crafted file cannot nest deeper).
fn read_from(
    src: &dyn Source,
    budget: &Cell<u64>,
    kept: Option<Kept>,
) -> Result<SceneIn, SceneError> {
    let o = open(src, budget)?;
    let parts = &o.manifest.parts;
    let mut skipped: Vec<String> = (o.manifest.unknown.keys())
        .map(|k| format!("manifest field '{k}'"))
        .collect();
    let mut by_part: Vec<Vec<&ChunkEntry>> = vec![Vec::new(); parts.len()];
    let mut frozen = by_part.clone();
    let mut refused: Vec<Option<String>> = vec![None; parts.len()];
    let mut by_name: BTreeMap<&str, Vec<&ChunkEntry>> = BTreeMap::new();
    for c in &o.manifest.chunks {
        let known = KNOWN.contains(&c.kind.as_str());
        let part = c.part.map(|p| p as usize).filter(|&p| p < parts.len());
        match (known, part) {
            (true, Some(p)) if c.kind == m::FROZEN => frozen[p].push(c),
            (true, Some(p)) => by_part[p].push(c),
            (true, None) if c.kind == m::PREVIEW => {}
            (true, None) if c.kind == m::REFERENCE => {
                let name = c.name.as_deref().unwrap_or_default();
                by_name.entry(name).or_default().push(c);
            }
            (false, Some(p)) if c.required => {
                refused[p] = Some(format!(
                    "it needs chunk kind '{}', which this version does not know",
                    c.kind
                ));
            }
            (false, None) if c.required => {
                return Err(SceneError::Damaged {
                    offset: c.offset,
                    reason: format!(
                        "the file needs chunk kind '{}', which this version does not know",
                        c.kind
                    ),
                });
            }
            (false, _) => skipped.push(format!("{} chunk", c.kind)),
            (true, _) => {
                return Err(SceneError::Damaged {
                    offset: c.offset,
                    reason: "a chunk names a part that does not exist".into(),
                });
            }
        }
    }
    let wanted: BTreeSet<String> = match kept {
        None => match by_name.values().flatten().next() {
            Some(c) => {
                return Err(SceneError::Damaged {
                    offset: c.offset,
                    reason: "a kept Part's file keeps Parts of its own".into(),
                });
            }
            None => BTreeSet::new(),
        },
        Some(Kept::None) => BTreeSet::new(),
        Some(Kept::Named(names)) => names(&o.manifest.session).into_iter().collect(),
    };
    let claimed = (by_part.iter().flatten()).fold(0u64, |sum, c| sum.saturating_add(c.raw));
    charge(budget, claimed, "its Parts claim", o.trailer.manifest_at)?;
    let read = (parts.iter().enumerate())
        .map(|(p, e)| match refused[p].take() {
            Some(reason) => Err(part_error(e, None, reason)),
            None => read_part(src, &o.trailer, e, &by_part[p]),
        })
        .collect();
    let read = attach(src, &o.trailer, read, &frozen, budget, kept.is_none())?;
    let references = kept_parts(src, &o.trailer, by_name, &wanted, budget, &mut skipped);
    Ok(SceneIn {
        preview: preview(src, &o),
        header: o.header,
        writer: o.manifest.writer,
        scene: o.manifest.scene,
        session: o.manifest.session,
        parts: read,
        skipped,
        references,
    })
}

/// Takes `claimed` bytes from what is left of the budget, or refuses the file, saying `what`
/// claims them.
fn charge(budget: &Cell<u64>, claimed: u64, what: &str, at: u64) -> Result<(), SceneError> {
    let left = budget.get();
    if claimed > left {
        return Err(SceneError::Damaged {
            offset: at,
            reason: format!(
                "{what} {claimed} bytes uncompressed, more than the {left} a file of its size \
                 may hold"
            ),
        });
    }
    budget.set(left - claimed);
    Ok(())
}

/// Reads the kept Parts named in `wanted`, each from its one chunk, and lists the others in
/// `skipped` without reading them.
fn kept_parts(
    src: &dyn Source,
    t: &Trailer,
    by_name: BTreeMap<&str, Vec<&ChunkEntry>>,
    wanted: &BTreeSet<String>,
    budget: &Cell<u64>,
    skipped: &mut Vec<String>,
) -> Vec<(String, Result<PartIn, PartError>)> {
    let mut references = Vec::new();
    for (name, copies) in by_name {
        let read = match copies[..] {
            _ if !wanted.contains(name) => {
                skipped.push(format!(
                    "kept Part '{name}', which no saved reference names"
                ));
                continue;
            }
            [c] => reference(src, t, c, budget),
            _ => Err(kept_error(
                copies[0],
                format!("the file keeps {} copies under this name", copies.len()),
            )),
        };
        references.push((name.to_string(), read));
    }
    references
}

/// A frozen Part's set-aside levels (decision 0030, section 7): its one `frozen_levels` chunk
/// holds a Part file of one Part, the level stack, checked and read as a `reference` chunk's is,
/// from the same budget.
///
/// Gives each Part read the frozen levels its chunks in `frozen` hold. A Part with more than
/// one such chunk, or whose chunk does not read, is refused by name. A file that is itself a
/// kept or frozen Part's (`nested`) may hold none: nothing nests deeper than one file.
fn attach(
    src: &dyn Source,
    t: &Trailer,
    parts: Vec<Result<PartIn, PartError>>,
    frozen: &[Vec<&ChunkEntry>],
    budget: &Cell<u64>,
    nested: bool,
) -> Result<Vec<Result<PartIn, PartError>>, SceneError> {
    if let Some(c) = frozen.iter().flatten().next().filter(|_| nested) {
        return Err(SceneError::Damaged {
            offset: c.offset,
            reason: "a kept or frozen Part's file keeps frozen levels of its own".into(),
        });
    }
    let read = |part: Result<PartIn, PartError>, chunks: &Vec<&ChunkEntry>| {
        let mut part = part?;
        let owned = |e: PartError| PartError {
            part: part.name.clone(),
            ..e
        };
        part.frozen = match chunks[..] {
            [] => None,
            [c] => Some(Box::new(reference(src, t, c, budget).map_err(owned)?)),
            [c, ..] => {
                return Err(owned(PartError {
                    part: String::new(),
                    chunk: Some("frozen levels".into()),
                    offset: Some(c.offset),
                    reason: format!("it has {} sets of frozen levels, not one", chunks.len()),
                }));
            }
        };
        Ok(part)
    };
    Ok(parts
        .into_iter()
        .zip(frozen)
        .map(|(p, c)| read(p, c))
        .collect())
}

/// A Part kept beside the scene: the Part file of one Part its `reference` chunk holds, checked
/// against the chunk's hash like any chunk, then read as a file of its own. A `frozen_levels`
/// chunk's the same.
fn reference(
    src: &dyn Source,
    t: &Trailer,
    c: &ChunkEntry,
    budget: &Cell<u64>,
) -> Result<PartIn, PartError> {
    let fail = |reason: String| kept_error(c, reason);
    placed(t, c).map_err(fail)?;
    if !hashed(src, c).map_err(|e| fail(e.to_string()))? {
        return Err(fail("its hash does not match: the chunk is damaged".into()));
    }
    let window = Window {
        src,
        at: c.offset,
        len: c.stored,
    };
    let file = read_from(&window, budget, None).map_err(|e| fail(e.to_string()))?;
    match <[_; 1]>::try_from(file.parts) {
        Ok([part]) if file.header.kind == FileKind::Parts => part.map_err(|e| fail(e.to_string())),
        _ => Err(fail("it does not hold a Part file of one Part".into())),
    }
}

/// The codec a chunk of `kind` is stored with; `None` for a kind this version does not know.
fn codec_of(kind: &str) -> Option<&'static str> {
    match kind {
        m::PREVIEW | m::REFERENCE | m::FROZEN => Some(m::STORED),
        k if KNOWN.contains(&k) => Some(m::ZSTD),
        _ => None,
    }
}

/// A chunk lies inside the file's chunks and has its kind's codec (a stored chunk holds as many
/// bytes as it stores). `file.open` and `file.inspect` check the same.
pub(super) fn placed(t: &Trailer, c: &ChunkEntry) -> Result<(), String> {
    if c.offset < HEADER as u64 || c.offset.saturating_add(c.stored) > t.manifest_at {
        return Err("it lies outside the file's chunks".into());
    }
    match codec_of(&c.kind) {
        Some(want) if c.codec != want => Err(format!("unexpected codec {}, not {want}", c.codec)),
        Some(m::STORED) if c.raw != c.stored => Err("its two sizes differ, stored as it is".into()),
        _ => Ok(()),
    }
}

/// Whether a placed chunk's bytes match its hash, read a piece at a time so a large one is
/// never held whole.
pub(super) fn hashed(src: &dyn Source, c: &ChunkEntry) -> Result<bool, SceneError> {
    let mut hash = blake3::Hasher::new();
    let mut at = 0;
    while at < c.stored {
        let len = PIECE.min(c.stored - at);
        hash.update(&src.at(c.offset + at, len)?);
        at += len;
    }
    Ok(c.hash == hash.finalize().to_hex()[..32])
}

/// Why the Part a `reference` chunk keeps could not be read.
fn kept_error(c: &ChunkEntry, reason: String) -> PartError {
    PartError {
        part: c.name.clone().unwrap_or_default(),
        chunk: Some(chunk_name(c)),
        offset: Some(c.offset),
        reason,
    }
}

/// The preview, if the header points at one that fits and its hash, when listed, matches.
fn preview(src: &dyn Source, o: &Opened) -> Option<Vec<u8>> {
    let (at, len) = o.header.preview;
    if len == 0 || len > MAX_PREVIEW || at.saturating_add(len) > o.trailer.manifest_at {
        return None;
    }
    let bytes = src.at(at, len).ok()?;
    let listed = (o.manifest.chunks.iter()).find(|c| c.kind == m::PREVIEW);
    listed
        .is_none_or(|c| c.hash == codec::hash16(&bytes))
        .then_some(bytes)
}

/// How a chunk is named in errors: its kind and what it covers.
pub(super) fn chunk_name(c: &ChunkEntry) -> String {
    let [first, count] = c.range;
    let last = first.saturating_add(count).saturating_sub(1);
    match (c.kind.as_str(), &c.name) {
        (m::NUMBERING, Some(name)) => format!("numbering {name} entries {first} to {last}"),
        (m::ATTRIBUTE, Some(name)) => format!("attribute {name} of nodes {first} to {last}"),
        (m::LEVEL, Some(name)) => {
            let of = match (c.level, &c.attribute) {
                (Some(j), _) => format!(" of level {j}"),
                (None, Some(a)) => format!(" of {a}"),
                (None, None) => String::new(),
            };
            format!("level {name}{of}, {first} to {last}")
        }
        (m::REFERENCE, Some(name)) => format!("reference {name}"),
        (m::FROZEN, _) => "frozen levels".into(),
        (kind, _) => format!("{kind} of nodes {first} to {last}"),
    }
}

pub(super) fn part_error(e: &PartEntry, chunk: Option<&ChunkEntry>, reason: String) -> PartError {
    PartError {
        part: e.name.clone(),
        chunk: chunk.map(chunk_name),
        offset: chunk.map(|c| c.offset),
        reason,
    }
}
