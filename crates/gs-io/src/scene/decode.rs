//! Turning one Part's chunks into its columns (decision 0016): the chunk table is checked to
//! give every node its columns once before any chunk is read, then each chunk is read, checked
//! by its hash and by the size the Part's node table gives it, decoded in parallel and put in its
//! node. A chunk that fails refuses only its Part, by name and place.

use gs_core::{AttrDef, AttrKind, AttrVec, Domain, ExactNode, NodeConfig, StoredLevels, Vec3};

use super::codec::{self, Filter};
use super::layout::Trailer;
use super::levels::{self, Got, Want};
use super::manifest::{self as m, ChunkEntry, NumberingEntry, PartEntry};
use super::read::{Source, part_error, placed};
use super::{NumberingIn, PartError, PartIn};

/// The node table fits the configuration: slot counts within capacity, free lists within slots.
fn check_table(cfg: NodeConfig, table: &[[u32; 4]]) -> Result<(), String> {
    if table.len() > cfg.max_nodes() {
        return Err(format!("{} nodes, more than ids can hold", table.len()));
    }
    let fits = |r: &[u32; 4]| {
        r[0] <= cfg.capacity() && r[1] <= cfg.face_capacity() && r[2] <= r[0] && r[3] <= r[1]
    };
    match table.iter().position(|r| !fits(r)) {
        Some(k) => Err(format!("node {k} has more slots than a node holds")),
        None => Ok(()),
    }
}

/// What one chunk decoded into.
enum Piece {
    Points(usize, Vec<Vec<Vec3>>),
    Faces(usize, Vec<Vec<[u32; 4]>>),
    Free(usize, Vec<(Vec<u32>, Vec<u32>)>),
    Attr(usize, Vec<(usize, AttrVec)>),
    Numbers(usize, u64, Numbers),
    Level(Got),
}

enum Numbers {
    Words(Vec<u32>),
    Bytes(Vec<u8>),
}

/// What a Part's chunks need to know about the Part.
struct Shape<'a> {
    table: &'a [[u32; 4]],
    attrs: &'a [AttrDef],
    /// The numbering arrays' lengths: vertices, faces, first corners; `None` for store order.
    numbering: Option<[u64; 3]>,
    levels: Option<levels::Shape<'a>>,
}

pub(super) fn read_part(
    src: &dyn Source,
    t: &Trailer,
    e: &PartEntry,
    chunks: &[&ChunkEntry],
) -> Result<PartIn, PartError> {
    let whole = |reason: String| part_error(e, None, reason);
    let config = NodeConfig::new(e.shift, e.fill).map_err(|x| whole(x.to_string()))?;
    let attrs = levels::definitions(&e.attributes).map_err(whole)?;
    check_table(config, &e.nodes).map_err(whole)?;
    let numbering = match e.numbering {
        NumberingEntry::Spatial => None,
        NumberingEntry::Given { vertices, faces } => {
            let slots = |i: usize| e.nodes.iter().map(|r| u64::from(r[i])).sum::<u64>();
            if vertices > slots(0) || faces > slots(1) {
                return Err(whole("the numbering is longer than the Part".into()));
            }
            Some([vertices, faces, faces])
        }
    };
    let levels = match &e.levels {
        Some(_) if numbering.is_none() => {
            return Err(whole("a Part with detail levels has no numbering".into()));
        }
        Some(l) => Some(levels::shape(l, config, &e.nodes).map_err(whole)?),
        None => None,
    };
    let shape = Shape {
        table: &e.nodes,
        attrs: &attrs,
        numbering,
        levels,
    };
    covered(e, chunks)?;
    let pieces = gs_par::map(chunks.len(), |i| {
        decode(src, t, &shape, chunks[i]).map_err(|r| part_error(e, Some(chunks[i]), r))
    });
    let pieces: Vec<Piece> = pieces.into_iter().collect::<Result<_, _>>()?;
    let (nodes, numbering, levels) = assemble(&shape, pieces).map_err(whole)?;
    Ok(PartIn {
        name: e.name.clone(),
        props: e.props.clone(),
        config,
        attrs,
        nodes,
        numbering,
        levels,
        frozen: None,
    })
}

/// Every node has its positions and its topology, and its free lists when it has free slots,
/// each from exactly one chunk: checked from the chunk table, before any chunk is read.
fn covered(e: &PartEntry, chunks: &[&ChunkEntry]) -> Result<(), PartError> {
    let mut have = vec![[false; 3]; e.nodes.len()];
    for c in chunks {
        let column = match c.kind.as_str() {
            m::POSITIONS => 0,
            m::TOPOLOGY => 1,
            m::FREE => 2,
            _ => continue,
        };
        let [first, count] = c.range;
        let end = first
            .checked_add(count)
            .filter(|&end| end <= have.len() as u64);
        let Some(end) = end.filter(|_| count > 0) else {
            let reason = "it covers nodes the Part does not have".into();
            return Err(part_error(e, Some(c), reason));
        };
        let span = &mut have[first as usize..end as usize];
        if let Some(i) = span
            .iter_mut()
            .position(|h| std::mem::replace(&mut h[column], true))
        {
            let k = first as usize + i;
            return Err(part_error(e, Some(c), format!("node {k} is covered twice")));
        }
    }
    let whole = |h: &[bool; 3], r: &[u32; 4]| h[0] && h[1] && (h[2] || r[2] == 0 && r[3] == 0);
    match (have.iter().zip(&e.nodes)).position(|(h, r)| !whole(h, r)) {
        Some(k) => Err(part_error(e, None, format!("node {k} is missing a chunk"))),
        None => Ok(()),
    }
}

/// Reads, checks and decodes one chunk.
fn decode(src: &dyn Source, t: &Trailer, shape: &Shape, c: &ChunkEntry) -> Result<Piece, String> {
    let [first, count] = c.range;
    let nodes = shape.table.len() as u64;
    let column = || -> Result<std::ops::Range<usize>, String> {
        if count == 0 || first.checked_add(count).is_none_or(|end| end > nodes) {
            return Err("it covers nodes the Part does not have".into());
        }
        Ok(first as usize..(first + count) as usize)
    };
    let rows = |r: std::ops::Range<usize>, i: usize| -> Vec<usize> {
        shape.table[r].iter().map(|x| x[i] as usize).collect()
    };
    let (filter, lens, width, piece): (Filter, Vec<usize>, usize, Kind) = match c.kind.as_str() {
        m::POSITIONS => (Filter::Points, rows(column()?, 0), 12, Kind::Points),
        m::TOPOLOGY => (Filter::Faces, rows(column()?, 1), 16, Kind::Faces),
        m::FREE => {
            let r = column()?;
            let lens = (shape.table[r].iter()).flat_map(|x| [x[2] as usize, x[3] as usize]);
            (Filter::Bytes, lens.collect(), 4, Kind::Free)
        }
        m::ATTRIBUTE => attribute(shape, c, column()?)?,
        m::NUMBERING => numbering(shape, c)?,
        m::LEVEL => {
            let l = (shape.levels.as_ref()).ok_or("the Part has no detail levels")?;
            let (filter, lens, width, want) = levels::want(l, c)?;
            (filter, lens, width, Kind::Level(want))
        }
        other => return Err(format!("unexpected kind {other}")),
    };
    if c.filter != filter.name() {
        return Err(format!("unexpected filter {}", c.filter));
    }
    let raw: usize = lens.iter().sum::<usize>() * width;
    if c.raw != raw as u64 {
        return Err(format!(
            "it holds {} bytes, not the {raw} its nodes need",
            c.raw
        ));
    }
    placed(t, c)?;
    let stored = src.at(c.offset, c.stored).map_err(|e| e.to_string())?;
    if codec::hash16(&stored) != c.hash {
        return Err("its hash does not match: the chunk is damaged".into());
    }
    let bytes = codec::unpack(&stored, raw, filter).ok_or("it does not decompress")?;
    piece.split(first as usize, &bytes, &lens, width)
}

/// What a chunk's bytes become.
enum Kind {
    Points,
    Faces,
    Free,
    Attr(usize, Vec<usize>, AttrKind),
    Numbers(usize, u64),
    Level(Want),
}

impl Kind {
    /// Cuts decoded bytes into per-node (or per-array) values, `lens[i]` elements of `width`.
    /// Fails only on values no column holds (a flag other than 0 or 1).
    fn split(
        self,
        first: usize,
        bytes: &[u8],
        lens: &[usize],
        width: usize,
    ) -> Result<Piece, String> {
        let mut at = 0;
        let mut next = |n: usize| {
            let s = &bytes[at..at + n * width];
            at += n * width;
            s
        };
        Ok(match self {
            Kind::Points => Piece::Points(
                first,
                lens.iter().map(|&n| codec::points_of(next(n))).collect(),
            ),
            Kind::Faces => {
                let mut all = codec::faces_of(bytes).into_iter();
                let per = lens
                    .iter()
                    .map(|&n| all.by_ref().take(n).collect())
                    .collect();
                Piece::Faces(first, per)
            }
            Kind::Free => Piece::Free(
                first,
                (lens.as_chunks::<2>().0.iter())
                    .map(|l| (codec::words_of(next(l[0])), codec::words_of(next(l[1]))))
                    .collect(),
            ),
            Kind::Attr(a, held, kind) => {
                let values = (held.into_iter().zip(lens))
                    .map(|(k, &n)| {
                        let column = AttrVec::of_words(kind, codec::words_of(next(n)));
                        column
                            .map(|c| (k, c))
                            .ok_or("a flag holds a value other than 0 or 1")
                    })
                    .collect::<Result<_, _>>()?;
                Piece::Attr(a, values)
            }
            Kind::Numbers(array, start) => Piece::Numbers(
                array,
                start,
                if width == 1 {
                    Numbers::Bytes(bytes.to_vec())
                } else {
                    Numbers::Words(codec::words_of(bytes))
                },
            ),
            Kind::Level(want) => Piece::Level(want.split(bytes, lens, width)?),
        })
    }
}

/// An attribute chunk: which attribute, and the nodes that hold it.
fn attribute(
    shape: &Shape,
    c: &ChunkEntry,
    r: std::ops::Range<usize>,
) -> Result<(Filter, Vec<usize>, usize, Kind), String> {
    let a = (shape.attrs.iter())
        .position(|d| Some(&d.name) == c.name.as_ref())
        .ok_or("it names an attribute the Part does not have")?;
    let held: Vec<usize> = c.held.iter().map(|&k| k as usize).collect();
    let inside = held.iter().all(|k| r.contains(k)) && held.windows(2).all(|w| w[0] < w[1]);
    if held.is_empty() || !inside {
        return Err("its list of nodes is not inside its range".into());
    }
    let d = &shape.attrs[a];
    let column = usize::from(d.domain == Domain::Face);
    let lens = held
        .iter()
        .map(|&k| shape.table[k][column] as usize)
        .collect();
    Ok((Filter::Words, lens, 4, Kind::Attr(a, held, d.kind)))
}

/// A numbering chunk: which array, and the entries it covers.
fn numbering(shape: &Shape, c: &ChunkEntry) -> Result<(Filter, Vec<usize>, usize, Kind), String> {
    let lens = shape
        .numbering
        .ok_or("the Part is in store order and has no numbering")?;
    let array = [m::VERTICES, m::FACES, m::FIRST_CORNER]
        .iter()
        .position(|n| c.name.as_deref() == Some(*n))
        .ok_or("it names an unknown numbering array")?;
    let [first, count] = c.range;
    if first.checked_add(count).is_none_or(|end| end > lens[array]) {
        return Err("it covers entries the array does not have".into());
    }
    let (filter, width) = if array == 2 {
        (Filter::Bytes, 1)
    } else {
        (Filter::Words, 4)
    };
    Ok((
        filter,
        vec![count as usize],
        width,
        Kind::Numbers(array, first),
    ))
}

/// What a Part's pieces make: the top's nodes, its numbering, its levels.
type Assembled = (Vec<ExactNode>, NumberingIn, Option<StoredLevels>);

/// Puts every piece in its node ([`covered`] has checked each node's columns come once).
fn assemble(shape: &Shape, pieces: Vec<Piece>) -> Result<Assembled, String> {
    let mut nodes: Vec<ExactNode> = (0..shape.table.len())
        .map(|_| ExactNode {
            attrs: vec![None; shape.attrs.len()],
            ..ExactNode::default()
        })
        .collect();
    let mut numbers: [Vec<(u64, Numbers)>; 3] = Default::default();
    let mut got = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Points(first, v) => place(&mut nodes, first, v, |n, x| n.pos = x),
            Piece::Faces(first, v) => place(&mut nodes, first, v, |n, x| n.faces = x),
            Piece::Free(first, v) => {
                place(&mut nodes, first, v, |n, (a, b)| {
                    (n.vfree, n.ffree) = (a, b);
                });
            }
            Piece::Attr(a, list) => {
                for (k, values) in list {
                    let values = shape.attrs[a].loaded(values);
                    if nodes[k].attrs[a].replace(values).is_some() {
                        return Err(format!("node {k} is covered twice"));
                    }
                }
            }
            Piece::Numbers(array, start, values) => numbers[array].push((start, values)),
            Piece::Level(g) => got.push(g),
        }
    }
    let levels = match &shape.levels {
        Some(l) => Some(levels::assemble(l, got)?),
        None => None,
    };
    Ok((nodes, numbering_of(shape, numbers)?, levels))
}

fn place<T>(
    nodes: &mut [ExactNode],
    first: usize,
    values: Vec<T>,
    put: impl Fn(&mut ExactNode, T),
) {
    for (node, v) in nodes[first..].iter_mut().zip(values) {
        put(node, v);
    }
}

/// The numbering arrays, from their pieces, each covered once from start to end.
fn numbering_of(
    shape: &Shape,
    mut numbers: [Vec<(u64, Numbers)>; 3],
) -> Result<NumberingIn, String> {
    let Some(lens) = shape.numbering else {
        return Ok(NumberingIn::Spatial);
    };
    let mut words: [Vec<u32>; 2] = Default::default();
    let mut bytes = Vec::new();
    for (array, pieces) in numbers.iter_mut().enumerate() {
        pieces.sort_by_key(|p| p.0);
        for (start, values) in pieces.drain(..) {
            let at = if array == 2 {
                bytes.len()
            } else {
                words[array].len()
            };
            if start != at as u64 {
                return Err("a numbering array has a gap or an overlap".into());
            }
            match values {
                Numbers::Words(w) if array < 2 => words[array].extend(w),
                Numbers::Bytes(b) if array == 2 => bytes.extend(b),
                _ => return Err("a numbering chunk holds the wrong kind of values".into()),
            }
        }
    }
    let [vertices, faces] = words;
    if [vertices.len(), faces.len(), bytes.len()].map(|l| l as u64) != lens {
        return Err("a numbering array is incomplete".into());
    }
    Ok(NumberingIn::Given {
        vertices,
        faces,
        first_corner: bytes,
    })
}
