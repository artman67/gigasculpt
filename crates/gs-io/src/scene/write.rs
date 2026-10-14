//! Writing a scene file (decision 0016): plan the chunks, encode them a batch at a time in
//! parallel through `gs-par` (the caller chooses the pool), stream them out in table order, then
//! the manifest and the trailer. The writer only moves forward, and only one batch of encoded
//! chunks is held at a time.

use std::io::{self, Write};

use gs_core::{AttrVec, MeshStore, NodeId};

use super::codec::{self, Filter, Packed};
use super::layout::{HEADER, Header, Trailer, padding};
use super::levels::{self, Array};
use super::manifest::{self as m, AttrEntry, ChunkEntry, Manifest, NumberingEntry, PartEntry};
use super::{FORMAT, FileKind, NumberingOut, PartOut, SceneOut, oldest_reader};

/// Nodes per column chunk: grouping 8 nodes saves 5% on positions against one node per chunk
/// and leaves enough chunks to keep every thread busy (research 15, section 7.2).
const RUN: u64 = 8;
/// Entries per numbering chunk.
const FRAME: u64 = 1 << 18;
/// Chunks encoded at once, so memory stays bounded however big the scene.
const BATCH: usize = 64;

/// What a written file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// The file's length.
    pub bytes: u64,
    /// Chunks, the preview included.
    pub chunks: usize,
    /// The chunks' bytes before compression.
    pub raw: u64,
}

/// What one chunk holds.
enum What {
    Positions,
    Topology,
    Free,
    /// An attribute, by its index, for the nodes that hold it.
    Attribute(usize, Vec<u64>),
    Numbering(&'static str),
    /// A detail level array (section 15).
    Level(Array),
}

struct Plan {
    part: usize,
    what: What,
    first: u64,
    count: u64,
}

/// Counts what goes out, so every chunk knows its offset.
struct Out<'a> {
    out: &'a mut dyn Write,
    at: u64,
}

impl Out<'_> {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.at += bytes.len() as u64;
        Ok(())
    }
    fn align(&mut self) -> io::Result<()> {
        self.put(&[0u8; 8][..padding(self.at)])
    }
}

/// Writes through to `out`, hashing what passes: a chunk that is a whole file, streamed.
struct Hashing<'o, 'w> {
    out: &'o mut Out<'w>,
    hash: blake3::Hasher,
}

impl Write for Hashing<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.out.put(bytes)?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.out.flush()
    }
}

/// Writes `scene` to `out`, start to end. Refused, before anything is written, if a Part file
/// it would nest holds frozen levels of its own ([`one_deep`]).
pub fn write(out: &mut dyn Write, scene: &SceneOut) -> io::Result<Written> {
    one_deep(scene)?;
    let mut out = Out { out, at: 0 };
    let preview = scene.preview.unwrap_or_default();
    let oldest = oldest_reader(&scene.parts);
    let header = Header {
        format: FORMAT,
        oldest_reader: oldest,
        kind: scene.kind,
        preview: if preview.is_empty() {
            (0, 0)
        } else {
            (HEADER as u64, preview.len() as u64)
        },
    };
    out.put(&header.bytes())?;
    let mut chunks = Vec::new();
    if !preview.is_empty() {
        out.put(preview)?;
        let len = preview.len() as u64;
        chunks.push(ChunkEntry {
            kind: m::PREVIEW.into(),
            part: None,
            range: [0, 0],
            name: None,
            level: None,
            attribute: None,
            held: Vec::new(),
            filter: Filter::Bytes.name().into(),
            codec: m::STORED.into(),
            offset: HEADER as u64,
            stored: len,
            raw: len,
            hash: codec::hash16(preview),
            required: false,
        });
    }
    let plans = plan(scene);
    let mut raw = 0;
    for batch in plans.chunks(BATCH) {
        let packed = gs_par::map(batch.len(), |i| {
            let (bytes, filter) = bytes_of(scene, &batch[i]);
            codec::pack(&bytes, filter).map(|p| (p, filter))
        });
        for (plan, packed) in batch.iter().zip(packed) {
            let (packed, filter) = packed?;
            out.align()?;
            raw += packed.raw;
            let range = [plan.first, plan.count];
            let mut e = entry(Some(plan.part as u32), range, filter, out.at, &packed);
            label(scene, plan, &mut e);
            chunks.push(e);
            out.put(&packed.stored)?;
        }
    }
    let (nested, nested_raw) = nested_parts(&mut out, scene)?;
    raw += nested_raw;
    chunks.extend(nested);
    let manifest = Manifest {
        format: FORMAT,
        oldest_reader: oldest,
        writer: scene.writer.clone(),
        kind: scene.kind,
        scene: scene.scene.clone(),
        session: scene.session.clone(),
        parts: scene.parts.iter().map(part_entry).collect(),
        chunks,
        unknown: std::collections::BTreeMap::new(),
    };
    let count = manifest.chunks.len();
    finish(&mut out, &manifest)?;
    Ok(Written {
        bytes: out.at,
        chunks: count,
        raw,
    })
}

/// Refuses `scene` if a Part file it would nest, a frozen Part's levels or a kept Part, holds
/// frozen levels of its own: nothing nests deeper than one file (decision 0030, section 7), and
/// writing it without them would drop them without a word (review 0007, S3).
fn one_deep(scene: &SceneOut) -> io::Result<()> {
    let refuse = |what: String| {
        let why = format!("{what} holds frozen levels of its own, which no file keeps");
        Err(io::Error::new(io::ErrorKind::InvalidInput, why))
    };
    for part in &scene.parts {
        if part.frozen.as_ref().is_some_and(|r| r.frozen.is_some()) {
            return refuse(format!("part '{}': its frozen levels", part.name));
        }
    }
    match scene.references.iter().find(|(_, p)| p.frozen.is_some()) {
        Some((name, _)) => refuse(format!("kept Part '{name}'")),
        None => Ok(()),
    }
}

/// The Part files held in chunks: each frozen Part's levels (decision 0030), a required chunk
/// belonging to that Part, then each Part kept beside the scene, an optional one under its
/// name. Their entries, and what their inner files hold uncompressed.
fn nested_parts(out: &mut Out, scene: &SceneOut) -> io::Result<(Vec<ChunkEntry>, u64)> {
    let frozen = (scene.parts.iter().enumerate()).filter_map(|(p, part)| {
        let levels = part.frozen.as_deref()?;
        Some((m::FROZEN, Some(p as u32), None, levels))
    });
    let kept = (scene.references.iter()).map(|(name, part)| (m::REFERENCE, None, Some(name), part));
    let (mut entries, mut raw) = (Vec::new(), 0);
    for (kind, owner, name, part) in frozen.chain(kept) {
        out.align()?;
        let (mut e, written) = nested(out, &scene.writer, kind, part)?;
        (e.part, e.name, e.required) = (owner, name.cloned(), owner.is_some());
        raw += written.raw;
        entries.push(e);
    }
    Ok((entries, raw))
}

/// `part`, which holds no frozen levels ([`one_deep`]), written at `out`'s place as a Part file
/// of that one Part: its chunk entry of `kind`, belonging to no Part and optional, and what the
/// inner file holds.
fn nested(
    out: &mut Out,
    writer: &str,
    kind: &str,
    part: &PartOut,
) -> io::Result<(ChunkEntry, Written)> {
    let file = SceneOut {
        kind: FileKind::Parts,
        writer: writer.into(),
        preview: None,
        scene: serde_json::Value::Null,
        session: serde_json::Value::Null,
        parts: vec![PartOut {
            name: part.name,
            props: part.props.clone(),
            store: part.store,
            numbering: part.numbering,
            levels: part.levels,
            frozen: None,
        }],
        references: Vec::new(),
    };
    let offset = out.at;
    let mut sink = Hashing {
        out,
        hash: blake3::Hasher::new(),
    };
    let written = write(&mut sink, &file)?;
    let entry = ChunkEntry {
        kind: kind.into(),
        part: None,
        range: [0, 0],
        name: None,
        level: None,
        attribute: None,
        held: Vec::new(),
        filter: Filter::Bytes.name().into(),
        codec: m::STORED.into(),
        offset,
        stored: written.bytes,
        raw: written.bytes,
        hash: sink.hash.finalize().to_hex()[..32].to_string(),
        required: false,
    };
    Ok((entry, written))
}

/// The manifest, then the trailer.
fn finish(out: &mut Out, manifest: &Manifest) -> io::Result<()> {
    let json = serde_json::to_vec(manifest).map_err(io::Error::other)?;
    let stored = codec::compress(&json)?;
    out.align()?;
    let manifest_at = out.at;
    out.put(&stored)?;
    out.align()?;
    let mut manifest_hash = [0u8; 16];
    manifest_hash.copy_from_slice(&blake3::hash(&stored).as_bytes()[..16]);
    let trailer = Trailer {
        manifest_at,
        manifest_stored: stored.len() as u64,
        manifest_raw: json.len() as u64,
        manifest_hash,
        length: out.at + 64,
    };
    out.put(&trailer.bytes())
}

fn entry(
    part: Option<u32>,
    range: [u64; 2],
    filter: Filter,
    offset: u64,
    packed: &Packed,
) -> ChunkEntry {
    ChunkEntry {
        kind: String::new(),
        part,
        range,
        name: None,
        level: None,
        attribute: None,
        held: Vec::new(),
        filter: filter.name().into(),
        codec: m::ZSTD.into(),
        offset,
        stored: packed.stored.len() as u64,
        raw: packed.raw,
        hash: packed.hash.clone(),
        required: true,
    }
}

/// Every chunk of every Part, in table order: per run of nodes its positions, topology, free
/// lists (when it has any) and attributes, then the numbering arrays.
fn plan(scene: &SceneOut) -> Vec<Plan> {
    let mut plans = Vec::new();
    for (p, part) in scene.parts.iter().enumerate() {
        let s = part.store;
        let nodes = s.node_count() as u64;
        for first in (0..nodes).step_by(RUN as usize) {
            let count = RUN.min(nodes - first);
            let run = |what| Plan {
                part: p,
                what,
                first,
                count,
            };
            plans.push(run(What::Positions));
            plans.push(run(What::Topology));
            let range = first as NodeId..(first + count) as NodeId;
            if range.clone().any(|k| s.free_slots(k) != (&[][..], &[][..])) {
                plans.push(run(What::Free));
            }
            for (a, def) in s.attrs().iter().enumerate() {
                let id = s.attr_id(&def.name);
                let held: Vec<u64> = (range.clone())
                    .filter(|&k| id.is_some_and(|id| s.node(k).attr(id).is_some()))
                    .map(u64::from)
                    .collect();
                if !held.is_empty() {
                    plans.push(run(What::Attribute(a, held)));
                }
            }
        }
        if let NumberingOut::Given {
            vertices,
            faces,
            first_corner,
        } = part.numbering
        {
            let arrays = [
                (m::VERTICES, vertices.len()),
                (m::FACES, faces.len()),
                (m::FIRST_CORNER, first_corner.len()),
            ];
            for (name, len) in arrays {
                for first in (0..len as u64).step_by(FRAME as usize) {
                    let count = FRAME.min(len as u64 - first);
                    plans.push(Plan {
                        part: p,
                        what: What::Numbering(name),
                        first,
                        count,
                    });
                }
            }
        }
        for (array, first, count) in part
            .levels
            .map(|l| levels::plan(l, FRAME, RUN))
            .unwrap_or_default()
        {
            plans.push(Plan {
                part: p,
                what: What::Level(array),
                first,
                count,
            });
        }
    }
    plans
}

/// A plan's kind, its array or attribute name, and the nodes that hold it, on its entry.
fn label(scene: &SceneOut, plan: &Plan, e: &mut ChunkEntry) {
    let part = &scene.parts[plan.part];
    let kind = match &plan.what {
        What::Positions => m::POSITIONS,
        What::Topology => m::TOPOLOGY,
        What::Free => m::FREE,
        What::Attribute(a, held) => {
            e.name = Some(part.store.attrs()[*a].name.clone());
            e.held.clone_from(held);
            m::ATTRIBUTE
        }
        What::Numbering(name) => {
            e.name = Some((*name).into());
            m::NUMBERING
        }
        What::Level(array) => {
            if let Some(l) = part.levels {
                levels::label(l, array, e);
            }
            m::LEVEL
        }
    };
    e.kind = kind.into();
}

/// The bytes a plan stores, before its filter.
fn bytes_of(scene: &SceneOut, plan: &Plan) -> (Vec<u8>, Filter) {
    let part = &scene.parts[plan.part];
    let s = part.store;
    let nodes = || plan.first as NodeId..(plan.first + plan.count) as NodeId;
    match &plan.what {
        What::Positions => {
            let bytes = nodes()
                .flat_map(|k| codec::point_bytes(s.node(k).pos))
                .collect();
            (bytes, Filter::Points)
        }
        What::Topology => {
            let faces: Vec<[u32; 4]> = nodes().flat_map(|k| s.node(k).faces.to_vec()).collect();
            (codec::face_bytes(&faces), Filter::Faces)
        }
        What::Free => {
            let free = |k| {
                let (v, f) = s.free_slots(k);
                [v, f].concat()
            };
            (codec::word_bytes(nodes().flat_map(free)), Filter::Bytes)
        }
        What::Attribute(a, held) => {
            let id = s.attr_id(&s.attrs()[*a].name);
            let words = (held.iter())
                .filter_map(|&k| s.node(k as NodeId).attr(id?).map(AttrVec::words))
                .flatten();
            (codec::word_bytes(words), Filter::Words)
        }
        What::Numbering(name) => {
            numbering_bytes(part, name, plan.first as usize, plan.count as usize)
        }
        What::Level(array) => match part.levels {
            Some(l) => levels::bytes(l, array, plan.first, plan.count),
            None => (Vec::new(), Filter::Bytes),
        },
    }
}

fn numbering_bytes(part: &PartOut, name: &str, first: usize, count: usize) -> (Vec<u8>, Filter) {
    let NumberingOut::Given {
        vertices,
        faces,
        first_corner,
    } = part.numbering
    else {
        return (Vec::new(), Filter::Bytes);
    };
    let words = |a: &[u32]| codec::word_bytes(a[first..first + count].iter().copied());
    match name {
        m::VERTICES => (words(vertices), Filter::Words),
        m::FACES => (words(faces), Filter::Words),
        _ => (first_corner[first..first + count].to_vec(), Filter::Bytes),
    }
}

fn part_entry(part: &PartOut) -> PartEntry {
    let s: &MeshStore = part.store;
    let nodes = (0..s.node_count() as NodeId)
        .map(|k| {
            let (v, f) = s.free_slots(k);
            let n = s.node(k);
            [n.pos.len(), n.faces.len(), v.len(), f.len()].map(|x| x as u32)
        })
        .collect();
    let numbering = match part.numbering {
        NumberingOut::Spatial => NumberingEntry::Spatial,
        NumberingOut::Given {
            vertices, faces, ..
        } => NumberingEntry::Given {
            vertices: vertices.len() as u64,
            faces: faces.len() as u64,
        },
    };
    PartEntry {
        name: part.name.into(),
        props: part.props.clone(),
        shift: s.config().shift(),
        fill: s.config().fill(),
        attributes: s.attrs().iter().map(AttrEntry::of).collect(),
        nodes,
        numbering,
        levels: part.levels.map(levels::entry),
    }
}
