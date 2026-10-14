//! A part's geometry: the mesh object (vertex arrays) and its companion (faces, UVs, per
//! face data and the export transform).
//!
//! Both objects hold plain fields, then slots: one presence byte each, and after a present
//! slot's byte the objects that make it up. An array slot is a 2-byte object and a wrapper
//! (18 plain bytes and a planar array). The mesh object keeps a 260-byte object between its
//! plain fields and its first slot; the companion's last leading plain byte is its first
//! slot's presence byte, and it ends with 172 plain bytes that start with the export scale and
//! offset.
//!
//! Mesh slots: 0 holds the positions of the level the part was at when saved (12 bytes per
//! vertex), 1 its per vertex data (8 bytes), 3 and 4 the same for the top level when that is
//! higher. Companion slots are told apart by element size: 16 bytes per base face for the face
//! corners, 32 for the UVs of the four corners, 4 for a face group and flags.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, COLOR, GROUP, MASK, PlainMesh};

use super::RawSection;
use crate::ReadError;
use crate::container::chunk::{Item, Source, Span, walk};
use crate::container::planar::decode_elements;
use gs_core::math::arity;
use gs_core::{level_vertex_counts, subdivide_faces};

/// Version of the object between a mesh object's plain fields and its slots.
const MESH_SETTINGS: u16 = 2;
/// Version of a planar array object.
const PLANAR: u16 = 6;
/// Plain bytes at the end of a companion.
const COMPANION_TAIL: usize = 172;
/// The deepest subdivision level looked for.
const MAX_LEVEL: u32 = 12;
/// Mesh slots with the current level's positions and per vertex data, then the top level's.
const POSITIONS: u16 = 0;
const VERTEX_DATA: u16 = 1;
const TOP_POSITIONS: u16 = 3;
const TOP_VERTEX_DATA: u16 = 4;
/// Element sizes of the companion's arrays.
const FACE_BYTES: u16 = 16;
const UV_BYTES: u16 = 32;
const FACE_DATA_BYTES: u16 = 4;
/// The stored mask value of a vertex that is not masked.
const UNMASKED: u16 = u16::MAX;

/// Everything stored about one part's shape.
#[derive(Debug, Clone)]
pub struct Geometry {
    pub mesh_version: u16,
    /// `None` when the record holds no companion (parametric primitives).
    pub companion_version: Option<u16>,
    /// The mesh object's leading plain fields (384, 408 or 420 bytes seen).
    pub mesh_head: Vec<u8>,
    /// The companion's leading plain fields, without the first presence byte.
    pub companion_head: Vec<u8>,
    /// The companion's last 172 plain bytes.
    pub companion_tail: Vec<u8>,
    /// The polygon mesh; `None` when the companion holds no faces (sphere armatures).
    pub mesh: Option<MeshData>,
    /// Every slot not decoded, by location.
    pub other: Vec<Slot>,
}

/// A polygon mesh with its stored subdivision levels.
#[derive(Debug, Clone)]
pub struct MeshData {
    /// The base faces when level 0 is not stored (it holds them otherwise); see
    /// [`MeshData::base_faces`].
    base: Vec<[u32; 4]>,
    pub base_vertex_count: u32,
    /// How many subdivision levels the part has, the base included.
    pub level_count: u32,
    /// The level the part was at when saved (0 is the base).
    pub current_level: u32,
    /// The stored levels, lowest first: the current level and, when higher, the top level.
    pub levels: Vec<Level>,
    /// Per base face, the UVs of its four corners (the fourth unused on a triangle).
    pub uvs: Option<Vec<[[f32; 2]; 4]>>,
    /// Per base face: the face group in the low 16 bits, flags not understood yet above.
    pub face_data: Option<Vec<u32>>,
    /// The export scale and offset stored at the start of the companion's tail.
    pub export: Option<Export>,
}

impl MeshData {
    /// The base faces, quads or triangles (`NO_VERTEX` in the last slot). Kept once: in level
    /// 0's mesh when that level is stored.
    pub fn base_faces(&self) -> &[[u32; 4]] {
        match self.levels.first() {
            Some(level) if level.level == 0 => &level.mesh.faces,
            _ => &self.base,
        }
    }
}

/// One stored level.
#[derive(Debug, Clone)]
pub struct Level {
    pub level: u32,
    /// Positions in scene axes (Y up), faces for this level, and the attributes `group` (per
    /// face), `mask` and `color` (per vertex) when the file holds them.
    pub mesh: PlainMesh,
    /// The 8 stored bytes per vertex: flags, a byte not understood yet, the mask (`u16`,
    /// 65535 = not masked) and the color.
    pub vertex_data: Vec<[u8; 8]>,
}

/// The scale and offset an OBJ export applies: `obj = (position - offset) / scale` per axis,
/// with `position` in scene axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Export {
    pub scale: [f32; 3],
    pub offset: [f32; 3],
}

/// A slot kept by location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// True for a companion slot, false for a mesh slot.
    pub companion: bool,
    pub index: u16,
    pub sections: Vec<RawSection>,
    /// Element size and count when the slot is an array.
    pub array: Option<ArrayInfo>,
}

/// The shape of an array slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayInfo {
    pub element: u16,
    pub count: u32,
}

/// One slot's objects, and its planar array if it is an array slot.
struct RawSlot {
    index: u16,
    objects: Vec<Span>,
    array: Option<(ArrayInfo, Span)>,
}

/// Decodes the mesh object at `mesh` and its companion at `companion`, if any.
pub(crate) fn decode(
    src: &mut impl Source,
    mesh: Span,
    companion: Option<Span>,
) -> Result<Geometry, ReadError> {
    let items = walk(src, mesh.body_start(), mesh.body_end())?;
    let [Item::Raw { offset, len }, Item::Obj(settings), slots @ ..] = &items[..] else {
        return Err(bad(
            mesh,
            "a mesh holds plain fields and a settings object first",
        ));
    };
    if settings.version != MESH_SETTINGS {
        return Err(bad(
            mesh,
            "a mesh holds plain fields and a settings object first",
        ));
    }
    let mesh_head = src.bytes(*offset, *len, "mesh head")?;
    let mesh_slots = slots_of(src, slots, None)?;
    let items = match companion {
        Some(c) => walk(src, c.body_start(), c.body_end())?,
        None => Vec::new(),
    };
    let (head, tail, comp_slots) = match (companion, &items[..]) {
        (Some(c), [Item::Raw { offset, len }, rest @ ..]) => {
            let head = src.bytes(*offset, *len, "companion head")?;
            let (flag, head) = head.split_last().unwrap_or((&0, &[]));
            let (slots, tail) = companion_slots(src, c, rest, *flag)?;
            (head.to_vec(), tail, slots)
        }
        (Some(_), _) => (Vec::new(), Vec::new(), slots_of(src, &items, None)?),
        (None, _) => (Vec::new(), Vec::new(), Vec::new()),
    };
    let mut geometry = Geometry {
        mesh_version: mesh.version,
        companion_version: companion.map(|c| c.version),
        mesh_head,
        companion_head: head,
        companion_tail: tail,
        mesh: None,
        other: Vec::new(),
    };
    let mut mesh_slots: BTreeMap<u16, RawSlot> =
        mesh_slots.into_iter().map(|s| (s.index, s)).collect();
    let mut by_size: BTreeMap<u16, RawSlot> = BTreeMap::new();
    let mut other = Vec::new();
    for slot in comp_slots {
        match slot.array {
            Some((info, _)) if !by_size.contains_key(&info.element) => {
                by_size.insert(info.element, slot);
            }
            _ => other.push(kept(true, slot)),
        }
    }
    if let (Some(companion), Some(faces)) = (companion, by_size.remove(&FACE_BYTES)) {
        let parts = Parts {
            faces,
            uvs: by_size.remove(&UV_BYTES),
            face_data: by_size.remove(&FACE_DATA_BYTES),
            positions: mesh_slots.remove(&POSITIONS),
            vertex_data: mesh_slots.remove(&VERTEX_DATA),
            top_positions: mesh_slots.remove(&TOP_POSITIONS),
            top_vertex_data: mesh_slots.remove(&TOP_VERTEX_DATA),
        };
        geometry.mesh = Some(mesh_data(src, &geometry, companion, &parts)?);
    }
    other.extend(by_size.into_values().map(|s| kept(true, s)));
    let mut all: Vec<Slot> = mesh_slots.into_values().map(|s| kept(false, s)).collect();
    all.extend(other);
    geometry.other = all;
    Ok(geometry)
}

fn bad(span: Span, reason: &'static str) -> ReadError {
    ReadError::BadMesh {
        offset: span.offset,
        reason,
    }
}

fn kept(companion: bool, slot: RawSlot) -> Slot {
    Slot {
        companion,
        index: slot.index,
        sections: slot.objects.into_iter().map(RawSection::of).collect(),
        array: slot.array.map(|(info, _)| info),
    }
}

/// Splits the items after a companion's head into slots and its 172-byte tail. `flag` is the
/// presence byte of slot 0, stored as the head's last byte.
fn companion_slots(
    src: &mut impl Source,
    span: Span,
    items: &[Item],
    flag: u8,
) -> Result<(Vec<RawSlot>, Vec<u8>), ReadError> {
    let Some((Item::Raw { offset, len }, body)) = items.split_last() else {
        return Err(bad(span, "a companion ends with plain fields"));
    };
    let tail_at = len
        .checked_sub(COMPANION_TAIL)
        .ok_or(bad(span, "a companion ends with plain fields"))?;
    // Presence bytes before the tail announce nothing: no object follows them.
    let tail = src.bytes(offset + tail_at, COMPANION_TAIL, "companion tail")?;
    Ok((slots_of(src, body, Some(flag))?, tail))
}

/// Reads presence bytes and the objects after them. `first` is the presence byte of slot 0
/// when it was stored before `items`.
fn slots_of(
    src: &mut impl Source,
    items: &[Item],
    first: Option<u8>,
) -> Result<Vec<RawSlot>, ReadError> {
    let mut reader = Slots {
        found: Vec::new(),
        next: 0,
        open: false,
    };
    if let Some(b) = first {
        reader.flags(&[b]);
    }
    for item in items {
        match *item {
            Item::Raw { offset, len } => reader.flags(&src.bytes(offset, len, "presence bytes")?),
            Item::Obj(obj) => reader.object(obj),
        }
    }
    let mut slots = reader.found;
    for slot in &mut slots {
        slot.array = array_in(src, &slot.objects)?;
    }
    Ok(slots)
}

/// Collects slots from presence bytes and objects in file order. A plain byte other than 0 or
/// 1 is a plain field among the presence bytes (seen near the end of some mesh objects); it
/// still takes an index, and objects after a 0 byte are kept in a slot of their own.
struct Slots {
    found: Vec<RawSlot>,
    /// The index the next presence byte belongs to.
    next: u16,
    /// Whether the last presence byte was 1, so objects may follow.
    open: bool,
}

impl Slots {
    fn flags(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.open = b == 1;
            if self.open {
                self.start(self.next);
            }
            self.next = self.next.saturating_add(1);
        }
    }

    fn start(&mut self, index: u16) {
        self.found.push(RawSlot {
            index,
            objects: Vec::new(),
            array: None,
        });
    }

    fn object(&mut self, obj: Span) {
        if !self.open {
            self.start(self.next.saturating_sub(1));
            self.open = true;
        }
        if let Some(slot) = self.found.last_mut() {
            slot.objects.push(obj);
        }
    }
}

/// The planar array of an array slot: a 2-byte object, then a wrapper holding plain fields and
/// the array.
fn array_in(
    src: &mut impl Source,
    objects: &[Span],
) -> Result<Option<(ArrayInfo, Span)>, ReadError> {
    let [_, wrapper] = objects else {
        return Ok(None);
    };
    let inner = walk(src, wrapper.body_start(), wrapper.body_end())?;
    let [Item::Raw { .. }, Item::Obj(planar)] = inner[..] else {
        return Ok(None);
    };
    if planar.version != PLANAR {
        return Ok(None);
    }
    let head = src.bytes(planar.body_start(), 6, "array header")?;
    let size = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let element = u16::from_le_bytes([head[4], head[5]]);
    let count = size / u32::from(element.max(1));
    Ok(Some((ArrayInfo { element, count }, planar)))
}

/// The slots a polygon mesh is built from.
struct Parts {
    faces: RawSlot,
    uvs: Option<RawSlot>,
    face_data: Option<RawSlot>,
    positions: Option<RawSlot>,
    vertex_data: Option<RawSlot>,
    top_positions: Option<RawSlot>,
    top_vertex_data: Option<RawSlot>,
}

/// Decodes an array slot whose elements must be `N` bytes, `count` of them when given.
fn elements<const N: usize>(
    src: &mut impl Source,
    slot: &RawSlot,
    count: Option<usize>,
) -> Result<Vec<[u8; N]>, ReadError> {
    let Some((_, planar)) = slot.array else {
        return Err(ReadError::BadMesh {
            offset: slot.objects.first().map_or(0, |s| s.offset),
            reason: "an array slot holds no array",
        });
    };
    let items =
        decode_elements::<N>(src, planar)?.filter(|items| count.is_none_or(|c| c == items.len()));
    items.ok_or(ReadError::BadMesh {
        offset: planar.offset,
        reason: "an array's size does not match the mesh",
    })
}

/// The little-endian 4-byte words of `bytes`, as `M` values.
fn words<const N: usize, const M: usize, T>(bytes: &[u8; N], f: fn([u8; 4]) -> T) -> [T; M]
where
    T: Copy + Default,
{
    let mut out = [T::default(); M];
    for (dst, &w) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *dst = f(w);
    }
    out
}

/// A level's stored positions and per vertex data.
struct Stored {
    positions: Vec<[u8; 12]>,
    data: Vec<[u8; 8]>,
}

fn mesh_data(
    src: &mut impl Source,
    g: &Geometry,
    companion: Span,
    parts: &Parts,
) -> Result<MeshData, ReadError> {
    let (current_level, base_vertex_count) = level_and_count(&g.companion_head).ok_or(bad(
        companion,
        "a companion head holds the level and vertex count",
    ))?;
    let faces: Vec<[u32; 4]> = elements::<16>(src, &parts.faces, None)?
        .iter()
        .map(|f| words(f, u32::from_le_bytes))
        .collect();
    let face_count = Some(faces.len());
    let uvs = match &parts.uvs {
        Some(slot) => Some(
            elements::<32>(src, slot, face_count)?
                .iter()
                .map(|e| {
                    let v: [f32; 8] = words(e, f32::from_le_bytes);
                    [[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]]
                })
                .collect(),
        ),
        None => None,
    };
    let face_data = match &parts.face_data {
        Some(slot) => Some(
            elements::<4>(src, slot, face_count)?
                .into_iter()
                .map(u32::from_le_bytes)
                .collect::<Vec<u32>>(),
        ),
        None => None,
    };
    let positions = parts
        .positions
        .as_ref()
        .ok_or(bad(companion, "a mesh with faces stores positions"))?;
    let mut stored_levels = vec![stored(src, positions, parts.vertex_data.as_ref())?];
    if let Some(slot) = &parts.top_positions {
        stored_levels.push(stored(src, slot, parts.top_vertex_data.as_ref())?);
    }
    let found = level_numbers(&faces, base_vertex_count, current_level, &stored_levels);
    let numbers = found.map_err(|reason| bad(companion, reason))?;
    let group: Option<Vec<u32>> = face_data
        .as_ref()
        .map(|d| d.iter().map(|&v| v & 0xFFFF).collect());
    let mut levels: Vec<Level> = Vec::new();
    let mut vertices = base_vertex_count;
    let mut built = 0;
    for (&level, s) in numbers.iter().zip(stored_levels) {
        // Faces of the levels between, built from the last stored level above the base (its
        // faces moved into its mesh) or from the base.
        let mut here: Option<Vec<[u32; 4]>> = None;
        for _ in built..level {
            let stored_above = levels.last().filter(|l| l.level > 0);
            let from = here
                .as_deref()
                .or(stored_above.map(|l| l.mesh.faces.as_slice()))
                .unwrap_or(&faces);
            let (next, count) = subdivide_faces(from, vertices)
                .ok_or(bad(companion, "a subdivision level is too big"))?;
            (here, vertices) = (Some(next), count);
        }
        built = level;
        let here = here.unwrap_or_default();
        levels.push(level_mesh(level, &faces, here, s, group.as_deref()));
    }
    let base = match levels.first_mut() {
        Some(first) if first.level == 0 => {
            first.mesh.faces = faces;
            Vec::new()
        }
        _ => faces,
    };
    Ok(MeshData {
        base,
        base_vertex_count,
        level_count: built as u32 + 1,
        current_level,
        levels,
        uvs,
        face_data,
        export: export(&g.companion_tail),
    })
}

/// The current level and the base vertex count: the last two words of the companion head.
fn level_and_count(head: &[u8]) -> Option<(u32, u32)> {
    let words = head.last_chunk::<8>()?;
    let word =
        |at: usize| u32::from_le_bytes([words[at], words[at + 1], words[at + 2], words[at + 3]]);
    Some((word(0), word(4)))
}

/// The export scale and offset at the start of the companion tail; `None` when the scale is
/// zero (older files store zeros there, and their OBJ exports use no scale or offset).
fn export(tail: &[u8]) -> Option<Export> {
    let v: [f32; 6] = words(tail.first_chunk::<24>()?, f32::from_le_bytes);
    let scale = [v[0], v[1], v[2]];
    scale.iter().all(|s| s.is_normal()).then_some(Export {
        scale,
        offset: [v[3], v[4], v[5]],
    })
}

/// The level number of each stored level, from its vertex count: the first must be
/// `current`, each later one higher than the one before.
fn level_numbers(
    faces: &[[u32; 4]],
    base: u32,
    current: u32,
    stored: &[Stored],
) -> Result<Vec<usize>, &'static str> {
    // Every level has at least the base's vertices. Checking this first also keeps a corrupt
    // count from sizing the edge table past what the file holds.
    if stored.iter().any(|s| s.positions.len() < base as usize) {
        return Err("a stored level has fewer vertices than the base");
    }
    let counts = level_vertex_counts(faces, base, MAX_LEVEL)
        .ok_or("a face refers to a vertex past the base vertex count")?;
    let mut numbers = Vec::new();
    for s in stored {
        let level = counts
            .iter()
            .position(|&c| c == s.positions.len() as u64)
            .filter(|&l| numbers.last().is_none_or(|&last| l > last))
            .ok_or("the positions match no subdivision level")?;
        numbers.push(level);
    }
    if numbers.first() != Some(&(current as usize)) {
        return Err("the positions do not match the current level");
    }
    Ok(numbers)
}

/// A level's positions and per vertex data.
fn stored(
    src: &mut impl Source,
    positions: &RawSlot,
    data: Option<&RawSlot>,
) -> Result<Stored, ReadError> {
    let positions = elements::<12>(src, positions, None)?;
    let data = match data {
        Some(slot) => elements::<8>(src, slot, Some(positions.len()))?,
        None => Vec::new(),
    };
    Ok(Stored { positions, data })
}

/// One level as a plain mesh in scene axes: stored positions are Y down and Z toward the
/// back, so Y and Z change sign (a half turn about X, which keeps the winding).
fn level_mesh(
    level: usize,
    base: &[[u32; 4]],
    faces: Vec<[u32; 4]>,
    stored: Stored,
    group: Option<&[u32]>,
) -> Level {
    let Stored {
        positions,
        data: vertex_data,
    } = stored;
    let positions = positions
        .into_iter()
        .map(|p| {
            let [x, y, z]: [f32; 3] = words(&p, f32::from_le_bytes);
            [x, -y, -z]
        })
        .collect();
    let mut attributes = BTreeMap::new();
    if let Some(group) = group {
        let per_face = base.iter().zip(group).flat_map(|(f, &g)| {
            let children = if level == 0 {
                1
            } else {
                arity(*f) << (2 * (level - 1))
            };
            std::iter::repeat_n(g, children)
        });
        attributes.insert(
            GROUP.into(),
            column(AttrDomain::Face, AttrValues::U32(per_face.collect())),
        );
    }
    let data = &vertex_data;
    let mask = |d: &[u8; 8]| u16::from_le_bytes([d[2], d[3]]);
    if data.iter().any(|d| mask(d) != UNMASKED) {
        let values = data
            .iter()
            .map(|d| 1.0 - f32::from(mask(d)) / f32::from(UNMASKED));
        attributes.insert(
            MASK.into(),
            column(AttrDomain::Vertex, AttrValues::F32(values.collect())),
        );
    }
    if data.iter().any(|d| d[4..7] != [0, 0, 0]) {
        let values = data
            .iter()
            .map(|d| u32::from_le_bytes([d[6], d[5], d[4], 255]));
        attributes.insert(
            COLOR.into(),
            column(AttrDomain::Vertex, AttrValues::U32(values.collect())),
        );
    }
    Level {
        level: level as u32,
        mesh: PlainMesh {
            positions,
            faces,
            attributes,
        },
        vertex_data,
    }
}

fn column(domain: AttrDomain, values: AttrValues) -> Attribute {
    Attribute { domain, values }
}
