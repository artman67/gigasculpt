//! A Part's detail levels in a scene file (decision 0016, section 15): chunks of kind `level`
//! beside the top level's own chunks, and a `levels` entry in the Part's manifest entry.
//!
//! The writer plans and encodes them from gs-core's [`StoredLevels`]. The reader checks every
//! size against the entry's node tables, and those against the top's, before it allocates;
//! decodes each chunk; and puts the pieces together as [`StoredLevels`] again, each array
//! covered once from start to end. Whether they make a stack is gs-core's check
//! (`Levels::from_file`).

use std::sync::Arc;

use gs_core::{
    AttrDef, AttrKind, AttrVec, Domain, NodeConfig, NodeId, Pending, Placement, StoredLevel,
    StoredLevels, Vec3,
};
use serde::{Deserialize, Serialize};

use super::codec::{self, Filter};
use super::manifest::{self as m, AttrEntry, ChunkEntry};

/// The base's faces and sharp edges, by the name their chunks give.
pub(crate) const BASE_FACES: &str = "base_faces";
pub(crate) const CREASES: &str = "creases";
/// A lower level's own UVs (eight words per face), when a step smoothed the UVs above it.
pub(crate) const UVS: &str = "uvs";

/// A Part's levels in its manifest entry: everything but the arrays, which are in chunks.
#[derive(Serialize, Deserialize)]
pub(crate) struct LevelsEntry {
    /// The active level, 0 for the base.
    pub active: u32,
    pub pin_corners: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corners: Vec<[String; 4]>,
    /// Every level's smooth flag, the base first.
    pub smooth: Vec<bool>,
    pub base_faces: u64,
    pub creases: u64,
    /// Per level below the top, the base first: per node, its vertex and face slots.
    pub lower: Vec<Vec<[u32; 2]>>,
    /// The levels below the top that keep their own UVs, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uvs: Vec<u32>,
    /// When the active level is below the top: what its store holds beyond the top's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingEntry>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PendingEntry {
    /// The active level's attribute definitions: the top's, then any it added.
    pub attributes: Vec<AttrEntry>,
    /// Per attribute, the nodes that dropped the column gathered from the top.
    pub dropped: Vec<Vec<u32>>,
}

/// The entry for `l`.
pub(crate) fn entry(l: &StoredLevels) -> LevelsEntry {
    LevelsEntry {
        active: l.active as u32,
        pin_corners: l.pin_corners,
        corners: l.corners.clone(),
        smooth: l.smooth.clone(),
        base_faces: l.base_faces.len() as u64,
        creases: l.creases.len() as u64,
        lower: l.lower.iter().map(|s| s.nodes.clone()).collect(),
        uvs: (0..l.lower.len() as u32)
            .filter(|&j| !l.lower[j as usize].uvs.is_empty())
            .collect(),
        pending: l.pending.as_ref().map(|p| PendingEntry {
            attributes: p.defs.iter().map(AttrEntry::of).collect(),
            dropped: p.dropped.clone(),
        }),
    }
}

/// What one level chunk holds.
pub(crate) enum Array {
    BaseFaces,
    Creases,
    /// A numbering array (by its name) of a level below the top.
    Numbering(usize, &'static str),
    /// A level's own UVs.
    Uvs(usize),
    /// The pending positions of a run's edited nodes: their places in `Pending::positions`.
    Positions(Vec<usize>),
    /// Attribute `a`'s pending columns of a run's edited nodes: their places in
    /// `Pending::columns[a]`.
    Attribute(usize, Vec<usize>),
}

/// Every level chunk of `l`, in table order, with what it covers: entries of an array in
/// frames of `frame`, pending columns per run of `run` nodes of the active level (only runs
/// with an edited node).
pub(crate) fn plan(l: &StoredLevels, frame: u64, run: u64) -> Vec<(Array, u64, u64)> {
    let mut out = Vec::new();
    let mut frames = |len: u64, what: &dyn Fn() -> Array| {
        for first in (0..len).step_by(frame as usize) {
            out.push((what(), first, frame.min(len - first)));
        }
    };
    frames(l.base_faces.len() as u64, &|| Array::BaseFaces);
    frames(l.creases.len() as u64, &|| Array::Creases);
    for (j, s) in l.lower.iter().enumerate() {
        let n = &s.numbering;
        frames(n.vertices.len() as u64, &|| {
            Array::Numbering(j, m::VERTICES)
        });
        frames(n.faces.len() as u64, &|| Array::Numbering(j, m::FACES));
        frames(n.first_corner.len() as u64, &|| {
            Array::Numbering(j, m::FIRST_CORNER)
        });
        frames(s.uvs.len() as u64, &|| Array::Uvs(j));
    }
    let (Some(p), Some(table)) = (&l.pending, l.lower.get(l.active)) else {
        return out;
    };
    let nodes = table.nodes.len() as u64;
    let runs = |edited: &mut dyn Iterator<Item = NodeId>| {
        let mut runs: Vec<(u64, Vec<usize>)> = Vec::new();
        for (i, k) in edited.enumerate() {
            let start = u64::from(k) / run * run;
            match runs.last_mut() {
                Some((s, list)) if *s == start => list.push(i),
                _ => runs.push((start, vec![i])),
            }
        }
        runs.into_iter()
            .map(move |(s, list)| (list, s, run.min(nodes.saturating_sub(s))))
    };
    for (list, first, count) in runs(&mut p.positions.iter().map(|x| x.0)) {
        out.push((Array::Positions(list), first, count));
    }
    for (a, cols) in p.columns.iter().enumerate() {
        for (list, first, count) in runs(&mut cols.iter().map(|x| x.0)) {
            out.push((Array::Attribute(a, list), first, count));
        }
    }
    out
}

/// A level chunk's array name, level, attribute and the nodes it holds, on its entry.
pub(crate) fn label(l: &StoredLevels, array: &Array, e: &mut ChunkEntry) {
    let held = |list: &[usize], node: &dyn Fn(usize) -> NodeId| {
        list.iter().map(|&i| u64::from(node(i))).collect()
    };
    let p = l.pending.as_ref();
    let (name, level, attribute, held): (&str, _, _, Vec<u64>) = match array {
        Array::BaseFaces => (BASE_FACES, None, None, Vec::new()),
        Array::Creases => (CREASES, None, None, Vec::new()),
        Array::Numbering(j, name) => (name, Some(*j as u32), None, Vec::new()),
        Array::Uvs(j) => (UVS, Some(*j as u32), None, Vec::new()),
        Array::Positions(list) => {
            let at = |i: usize| p.map_or(0, |p| p.positions[i].0);
            (m::POSITIONS, None, None, held(list, &at))
        }
        Array::Attribute(a, list) => {
            let at = |i: usize| p.map_or(0, |p| p.columns[*a][i].0);
            let name = p.map(|p| p.defs[*a].name.clone());
            (m::ATTRIBUTE, None, name, held(list, &at))
        }
    };
    (e.name, e.level, e.attribute, e.held) = (Some(name.into()), level, attribute, held);
}

/// The bytes a level chunk stores, before its filter.
pub(crate) fn bytes(l: &StoredLevels, array: &Array, first: u64, count: u64) -> (Vec<u8>, Filter) {
    let r = first as usize..(first + count) as usize;
    let p = l.pending.as_ref();
    match array {
        Array::BaseFaces => (codec::face_bytes(&l.base_faces[r]), Filter::Faces),
        Array::Creases => {
            let words = l.creases[r]
                .iter()
                .flat_map(|&(a, b, s)| [a, b, u32::from(s)]);
            (codec::word_bytes(words), Filter::Words)
        }
        Array::Numbering(j, name) => {
            let n = &l.lower[*j].numbering;
            match *name {
                m::VERTICES => (
                    codec::word_bytes(n.vertices[r].iter().copied()),
                    Filter::Words,
                ),
                m::FACES => (codec::word_bytes(n.faces[r].iter().copied()), Filter::Words),
                _ => (n.first_corner[r].to_vec(), Filter::Bytes),
            }
        }
        Array::Uvs(j) => {
            let words = l.lower[*j].uvs[r].iter().flatten().flatten();
            (codec::word_bytes(words.map(|x| x.to_bits())), Filter::Words)
        }
        Array::Positions(list) => {
            let points =
                |i: &usize| p.map_or(Vec::new(), |p| codec::point_bytes(&p.positions[*i].1));
            (list.iter().flat_map(points).collect(), Filter::Points)
        }
        Array::Attribute(a, list) => {
            let words = |i: &usize| p.map_or(Vec::new(), |p| p.columns[*a][*i].1.words());
            (
                codec::word_bytes(list.iter().flat_map(words)),
                Filter::Words,
            )
        }
    }
}

/// What the reader knows of a Part's levels before their chunks, checked so every chunk's
/// size is known to fit what the Part holds.
pub(crate) struct Shape<'a> {
    entry: &'a LevelsEntry,
    /// The active level's definitions, when it has pending edits.
    defs: Vec<AttrDef>,
    /// Per level below the top, its vertex and face slots.
    sums: Vec<[u64; 2]>,
}

/// The attribute definitions entries name.
pub(crate) fn definitions(attrs: &[AttrEntry]) -> Result<Vec<AttrDef>, String> {
    (attrs.iter())
        .map(|a| {
            let domain = Domain::from_name(&a.domain);
            let kind = AttrKind::from_name(&a.kind);
            match (domain, kind) {
                (Some(d), Some(k)) => Ok(AttrDef::new(&a.name, d, k)),
                _ => Err(format!(
                    "attribute {} has a domain or kind this version does not know ({}, {})",
                    a.name, a.domain, a.kind
                )),
            }
        })
        .collect()
}

/// The levels `e` describes, checked against the node configuration and the top's node table:
/// every lower level's node table within a node's capacity and no bigger than the top (a lower
/// level is the top's first points), the base as big as level 0 says, at most one crease per
/// base corner.
pub(crate) fn shape<'a>(
    e: &'a LevelsEntry,
    cfg: NodeConfig,
    top: &[[u32; 4]],
) -> Result<Shape<'a>, String> {
    let top_sum = |i: usize| top.iter().map(|r| u64::from(r[i])).sum::<u64>();
    let (tv, tf) = (top_sum(0), top_sum(1));
    let mut sums = Vec::with_capacity(e.lower.len());
    for (j, table) in e.lower.iter().enumerate() {
        let within = |r: &[u32; 2]| r[0] <= cfg.capacity() && r[1] <= cfg.face_capacity();
        let sum = |i: usize| table.iter().map(|r| u64::from(r[i])).sum::<u64>();
        if table.len() > cfg.max_nodes() || !table.iter().all(within) {
            return Err(format!("level {j}'s node table holds more than nodes can"));
        }
        if sum(0) > tv || sum(1) > tf {
            return Err(format!("level {j} is bigger than the top level"));
        }
        sums.push([sum(0), sum(1)]);
    }
    let base = sums
        .first()
        .ok_or("the levels list no level below the top")?;
    if e.base_faces != base[1] || e.creases > e.base_faces.saturating_mul(4) {
        return Err("the base's faces or creases do not fit level 0".into());
    }
    let defs = match &e.pending {
        Some(p) => definitions(&p.attributes)?,
        None => Vec::new(),
    };
    Ok(Shape {
        entry: e,
        defs,
        sums,
    })
}

/// What a level chunk's bytes become, once read.
pub(crate) enum Want {
    BaseFaces(u64),
    Creases(u64),
    Words(usize, usize, u64),
    Bytes(usize, u64),
    Positions(Vec<NodeId>),
    Attr(usize, Vec<NodeId>, AttrKind),
}

/// A level chunk, checked against `s`: its filter, its elements' counts, their width, and
/// what its bytes become.
pub(crate) fn want(s: &Shape, c: &ChunkEntry) -> Result<(Filter, Vec<usize>, usize, Want), String> {
    let [first, count] = c.range;
    let within = |len: u64| {
        if count == 0 || first.checked_add(count).is_none_or(|end| end > len) {
            Err("it covers entries the array does not have")
        } else {
            Ok(())
        }
    };
    let n = count as usize;
    let level = |j: Option<u32>| {
        let j = j.ok_or("it names no level")? as usize;
        s.sums
            .get(j)
            .map(|x| (j, *x))
            .ok_or("it names a level the Part does not have")
    };
    match c.name.as_deref() {
        Some(BASE_FACES) => {
            within(s.entry.base_faces)?;
            Ok((Filter::Faces, vec![n], 16, Want::BaseFaces(first)))
        }
        Some(CREASES) => {
            within(s.entry.creases)?;
            Ok((Filter::Words, vec![n * 3], 4, Want::Creases(first)))
        }
        Some(m::VERTICES) => {
            let (j, [v, _]) = level(c.level)?;
            within(v)?;
            Ok((Filter::Words, vec![n], 4, Want::Words(j, 0, first)))
        }
        Some(m::FACES) => {
            let (j, [_, f]) = level(c.level)?;
            within(f)?;
            Ok((Filter::Words, vec![n], 4, Want::Words(j, 1, first)))
        }
        Some(m::FIRST_CORNER) => {
            let (j, [_, f]) = level(c.level)?;
            within(f)?;
            Ok((Filter::Bytes, vec![n], 1, Want::Bytes(j, first)))
        }
        Some(UVS) => {
            let (j, [_, f]) = level(c.level)?;
            if !s.entry.uvs.contains(&(j as u32)) {
                return Err("it holds UVs of a level the entry does not list".into());
            }
            within(f)?;
            Ok((Filter::Words, vec![n * 8], 4, Want::Words(j, 2, first * 8)))
        }
        Some(m::POSITIONS) => {
            let (held, table) = edited(s, c)?;
            let lens = held
                .iter()
                .map(|&k| table[k as usize][0] as usize)
                .collect();
            Ok((Filter::Points, lens, 12, Want::Positions(held)))
        }
        Some(m::ATTRIBUTE) => {
            let (held, table) = edited(s, c)?;
            let a = (s.defs.iter())
                .position(|d| Some(&d.name) == c.attribute.as_ref())
                .ok_or("it names an attribute the active level does not have")?;
            let d = &s.defs[a];
            let col = usize::from(d.domain == Domain::Face);
            let lens = held
                .iter()
                .map(|&k| table[k as usize][col] as usize)
                .collect();
            Ok((Filter::Words, lens, 4, Want::Attr(a, held, d.kind)))
        }
        _ => Err("it names an unknown level array".into()),
    }
}

/// A pending chunk's edited nodes, inside its range of the active level's nodes, and the
/// active level's node table.
fn edited<'a>(s: &Shape<'a>, c: &ChunkEntry) -> Result<(Vec<NodeId>, &'a [[u32; 2]]), String> {
    let e = s.entry;
    let table = (e.pending.as_ref())
        .and_then(|_| e.lower.get(e.active as usize))
        .ok_or("it holds edits, but the active level has none")?;
    let [first, count] = c.range;
    let end = first.saturating_add(count);
    let inside = end <= table.len() as u64
        && (c.held.iter()).all(|&k| (first..end).contains(&k))
        && c.held.windows(2).all(|w| w[0] < w[1]);
    if c.held.is_empty() || !inside {
        return Err("its list of nodes is not inside its range".into());
    }
    Ok((c.held.iter().map(|&k| k as NodeId).collect(), table))
}

/// A level chunk's values.
pub(crate) enum Got {
    BaseFaces(u64, Vec<[u32; 4]>),
    Creases(u64, Vec<(u32, u32, u8)>),
    Words(usize, usize, u64, Vec<u32>),
    Bytes(usize, u64, Vec<u8>),
    Positions(Vec<(NodeId, Vec<Vec3>)>),
    Attr(usize, Vec<(NodeId, AttrVec)>),
}

impl Want {
    /// Cuts decoded bytes into values: `lens[i]` elements of `width` bytes each.
    pub(crate) fn split(self, bytes: &[u8], lens: &[usize], width: usize) -> Result<Got, String> {
        let mut at = 0;
        let mut next = |n: usize| {
            let s = &bytes[at..at + n * width];
            at += n * width;
            s
        };
        Ok(match self {
            Want::BaseFaces(first) => Got::BaseFaces(first, codec::faces_of(bytes)),
            Want::Creases(first) => {
                let words = codec::words_of(bytes);
                let creases = (words.as_chunks::<3>().0.iter())
                    .map(|w| u8::try_from(w[2]).map(|s| (w[0], w[1], s)))
                    .collect::<Result<_, _>>()
                    .map_err(|_| "a crease is sharper than 255")?;
                Got::Creases(first, creases)
            }
            Want::Words(j, array, first) => Got::Words(j, array, first, codec::words_of(bytes)),
            Want::Bytes(j, first) => Got::Bytes(j, first, bytes.to_vec()),
            Want::Positions(held) => Got::Positions(
                (held.into_iter().zip(lens))
                    .map(|(k, &n)| (k, codec::points_of(next(n))))
                    .collect(),
            ),
            Want::Attr(a, held, kind) => {
                let values = (held.into_iter().zip(lens))
                    .map(|(k, &n)| {
                        let column = AttrVec::of_words(kind, codec::words_of(next(n)));
                        column
                            .map(|c| (k, c))
                            .ok_or("a flag holds a value other than 0 or 1")
                    })
                    .collect::<Result<_, _>>()?;
                Got::Attr(a, values)
            }
        })
    }
}

/// Pieces of one array: each with where it starts.
type Pieces<T> = Vec<(u64, Vec<T>)>;

/// Pieces of one array joined: each starts where the last ended, and they end at `len`.
fn joined<T>(mut pieces: Pieces<T>, len: u64, what: &str) -> Result<Vec<T>, String> {
    pieces.sort_by_key(|p| p.0);
    let mut out = Vec::new();
    for (start, values) in pieces {
        if start != out.len() as u64 {
            return Err(format!("{what} has a gap or an overlap"));
        }
        out.extend(values);
    }
    if out.len() as u64 == len {
        Ok(out)
    } else {
        Err(format!("{what} is incomplete"))
    }
}

/// Edited columns by node, ascending, each node once.
fn by_node<T>(mut cols: Vec<(NodeId, T)>, what: &str) -> Result<Vec<(NodeId, Arc<T>)>, String> {
    cols.sort_by_key(|c| c.0);
    if cols.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(format!("{what} lists a node twice"));
    }
    Ok(cols.into_iter().map(|(k, c)| (k, Arc::new(c))).collect())
}

/// The levels from their chunks' values, every array whole.
pub(crate) fn assemble(s: &Shape, got: Vec<Got>) -> Result<StoredLevels, String> {
    let e = s.entry;
    let t = s.sums.len();
    let (mut faces, mut creases, mut points) = (Vec::new(), Vec::new(), Vec::new());
    // Per level: its vertices, faces and own UVs (by word, eight per face).
    let mut words: Vec<[Pieces<u32>; 3]> = (0..t).map(|_| Default::default()).collect();
    let mut corners: Vec<Pieces<u8>> = vec![Vec::new(); t];
    let mut cols: Vec<Vec<(NodeId, AttrVec)>> = vec![Vec::new(); s.defs.len()];
    for g in got {
        match g {
            Got::BaseFaces(at, v) => faces.push((at, v)),
            Got::Creases(at, v) => creases.push((at, v)),
            Got::Words(j, array, at, v) => words[j][array].push((at, v)),
            Got::Bytes(j, at, v) => corners[j].push((at, v)),
            Got::Positions(v) => points.extend(v),
            Got::Attr(a, v) => cols[a].extend(v.into_iter().map(|(k, c)| (k, s.defs[a].loaded(c)))),
        }
    }
    let mut lower = Vec::with_capacity(t);
    for (j, (([vertices, faces, uvs], first), table)) in
        (words.into_iter().zip(corners)).zip(&e.lower).enumerate()
    {
        let [v, f] = s.sums[j];
        let listed = e.uvs.contains(&(j as u32));
        let uvs = joined(
            uvs,
            if listed { f * 8 } else { 0 },
            &format!("level {j}'s UVs"),
        )?;
        lower.push(StoredLevel {
            nodes: table.clone(),
            numbering: Placement {
                vertices: joined(vertices, v, &format!("level {j}'s vertices"))?,
                faces: joined(faces, f, &format!("level {j}'s faces"))?,
                first_corner: joined(first, f, &format!("level {j}'s first corners"))?,
            },
            uvs: (uvs.as_chunks::<8>().0.iter())
                .map(|w| std::array::from_fn(|i| [w[2 * i], w[2 * i + 1]].map(f32::from_bits)))
                .collect(),
        });
    }
    let pending = match &e.pending {
        Some(p) => Some(Pending {
            defs: s.defs.clone(),
            positions: by_node(points, "the edited positions")?,
            columns: (cols.into_iter().zip(&s.defs))
                .map(|(c, d)| by_node(c, &format!("the edited {}", d.name)))
                .collect::<Result<_, _>>()?,
            dropped: p.dropped.clone(),
        }),
        None if points.is_empty() && cols.iter().all(Vec::is_empty) => None,
        None => return Err("it holds edits, but the active level has none".into()),
    };
    Ok(StoredLevels {
        active: e.active as usize,
        pin_corners: e.pin_corners,
        corners: e.corners.clone(),
        smooth: e.smooth.clone(),
        base_faces: joined(faces, e.base_faces, "the base's faces")?,
        creases: joined(creases, e.creases, "the base's creases")?,
        lower,
        pending,
    })
}
