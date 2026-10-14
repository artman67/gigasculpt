//! The tool object and its part list.
//!
//! A tool object (version 1004) holds plain fields that end with the selected part's type
//! number, then that part's name, settings and record objects, and 2 plain bytes. A record
//! (version 10, 11 or 13) holds 25 plain bytes, a 2-byte object, a rectangle, the mesh object
//! and, for polygon meshes and sphere armatures, its companion, then 3 plain bytes. The
//! selected part's record goes on with the list: for each part in list order a lead byte and a
//! type number; for every part but the selected one its name, settings and record objects;
//! then its list entry (flags, color, display name and fixed fields whose length depends on
//! the record version).

use std::collections::VecDeque;

use super::{Entry, PartInfo, RawSection, Settings, Tool};
use crate::ReadError;
use crate::container::chunk::{Cursor, Item, Obj, Piece, Source, Span, scan, walk};
use crate::container::name;
use crate::container::record::{latin1, parse_setting};

/// Plain bytes at the end of every record, before the selected record's list.
const RECORD_TAIL: usize = 3;
/// The version of the 2-byte object that starts a group of mesh objects.
const MARKER: u16 = 13;

/// Length of the fixed fields after an entry's two names, for each record version seen.
const ENTRY_REST: [(u16, usize); 3] = [(10, 47), (11, 4), (13, 12)];

fn entry_rest(version: u16) -> Option<usize> {
    ENTRY_REST
        .iter()
        .find(|&&(v, _)| v == version)
        .map(|&(_, len)| len)
}

/// A part's objects before its list entry.
struct PartObjects {
    name: String,
    settings: Settings,
    record_version: u16,
    record_head: Vec<u8>,
    mesh: Span,
    companion: Option<Span>,
    own_entry: Option<Entry>,
    extra: Vec<RawSection>,
    /// Plain bytes between the last mesh group and the list.
    tail: usize,
}

impl PartObjects {
    fn info(self, lead: u8, kind: u32, entry: Entry) -> PartInfo {
        PartInfo {
            name: self.name,
            kind,
            lead,
            entry,
            own_entry: self.own_entry,
            extra: self.extra,
            settings: self.settings,
            record_version: self.record_version,
            record_head: self.record_head,
            mesh: self.mesh,
            companion: self.companion,
        }
    }
}

/// Reads a tool object and its part list.
pub(crate) fn tool(
    src: &mut impl Source,
    span: Span,
    project_flag: Option<u8>,
) -> Result<Tool, ReadError> {
    let layout = |reason| ReadError::BadLayout {
        offset: span.offset,
        reason,
    };
    let items = walk(src, span.body_start(), span.body_end())?;
    let [
        Item::Raw { offset, len },
        Item::Obj(n),
        Item::Obj(s),
        Item::Obj(r),
        end @ ..,
    ] = &items[..]
    else {
        return Err(layout(
            "a tool holds plain fields, then name, settings and record objects",
        ));
    };
    if !matches!(end, [] | [Item::Raw { .. }]) {
        return Err(layout("a tool ends after its first record"));
    }
    let head = src.bytes(*offset, *len, "tool head")?;
    let (selected, rest) = part_objects(src, *n, *s, *r)?;
    let rest_len =
        entry_rest(selected.record_version).ok_or(layout("unknown part record version"))?;
    let list = list_of(src, rest, selected.tail, rest_len, r.offset, 0)?;
    let (lead, kind, entry) = list
        .own
        .ok_or(layout("no list entry for the selected part"))?;
    let mut parts = list.others;
    parts.insert(list.own_at, selected.info(lead, kind, entry));
    Ok(Tool {
        offset: span.offset,
        project_flag,
        head,
        parts,
        selected: list.own_at,
    })
}

/// What a record's list holds: the record's own entry (its lead byte, type number and entry,
/// stored without objects) and the other parts in list order, with the own entry's place.
struct ListOf {
    own: Option<(u8, u32, Entry)>,
    others: Vec<PartInfo>,
    own_at: usize,
}

/// Reads what follows a record's mesh objects: `tail` plain bytes, then the list. Only the
/// selected part's record (`depth` 0) lists other parts; a record deeper down may list itself.
fn list_of(
    src: &mut impl Source,
    items: Vec<Item>,
    tail: usize,
    rest_len: usize,
    at: usize,
    depth: u8,
) -> Result<ListOf, ReadError> {
    let layout = |reason| ReadError::BadLayout { offset: at, reason };
    let mut list = List::new(items);
    let mut out = ListOf {
        own: None,
        others: Vec::new(),
        own_at: 0,
    };
    list.take(src, tail, "record tail")?;
    for _ in 0..=list.budget {
        if list.done(src)? {
            break;
        }
        let lead = list.u8(src)?;
        let kind = list.u32(src)?;
        let objects = match list.objects(at)? {
            Some(_) if depth > 0 => {
                return Err(layout("a part's own list holds other parts"));
            }
            Some([n, s, r]) => Some(other_part_objects(src, n, s, r, rest_len)?),
            None => None,
        };
        let entry = entry(src, &mut list, rest_len)?;
        match objects {
            Some(objects) => out.others.push(objects.info(lead, kind, entry)),
            None if out.own.is_none() => {
                out.own = Some((lead, kind, entry));
                out.own_at = out.others.len();
            }
            None => return Err(layout("two list entries without a record")),
        }
    }
    Ok(out)
}

/// A part other than the selected one, with the entry its own record may keep for itself.
fn other_part_objects(
    src: &mut impl Source,
    name_at: Span,
    settings_at: Span,
    record_at: Span,
    rest_len: usize,
) -> Result<PartObjects, ReadError> {
    let (mut objects, rest) = part_objects(src, name_at, settings_at, record_at)?;
    let inner = list_of(src, rest, objects.tail, rest_len, record_at.offset, 1)?;
    objects.own_entry = inner.own.map(|(_, _, entry)| entry);
    Ok(objects)
}

/// A part's name, settings and record, and what follows the record's mesh objects.
fn part_objects(
    src: &mut impl Source,
    name_at: Span,
    settings_at: Span,
    record_at: Span,
) -> Result<(PartObjects, Vec<Item>), ReadError> {
    let name = with_object(src, name_at, "part name", name)?;
    let settings = settings(src, settings_at)?;
    let items = walk(src, record_at.body_start(), record_at.body_end())?;
    let [
        Item::Raw { offset, len },
        Item::Obj(_),
        Item::Obj(_),
        Item::Obj(mesh),
        rest @ ..,
    ] = &items[..]
    else {
        return Err(ReadError::BadLayout {
            offset: record_at.offset,
            reason: "a record holds plain fields, two small objects and a mesh",
        });
    };
    let (companion, mut rest) = match rest {
        [Item::Obj(companion), rest @ ..] => (Some(*companion), rest),
        _ => (None, rest),
    };
    // A second group of a 2-byte object, a rectangle, a mesh and maybe a companion, after 6
    // plain bytes; seen once (a sphere armature next to its preview mesh). Its record then
    // has one plain byte less before the list.
    let mut extra = Vec::new();
    let mut tail = RECORD_TAIL;
    while let [
        Item::Raw { len: 6, .. },
        Item::Obj(marker),
        Item::Obj(_),
        Item::Obj(m),
        more @ ..,
    ] = rest
        && marker.version == MARKER
    {
        extra.push(RawSection::of(*m));
        rest = match more {
            [Item::Obj(c), more @ ..] => {
                extra.push(RawSection::of(*c));
                more
            }
            _ => more,
        };
        tail = RECORD_TAIL - 1;
    }
    let objects = PartObjects {
        name,
        settings,
        record_version: record_at.version,
        record_head: src.bytes(*offset, *len, "record head")?,
        mesh: *mesh,
        companion,
        own_entry: None,
        extra,
        tail,
    };
    Ok((objects, rest.to_vec()))
}

/// The settings object: plain fields, then setting records with presence bytes between them.
fn settings(src: &mut impl Source, span: Span) -> Result<Settings, ReadError> {
    let len = span.body_end() - span.body_start();
    let body = src.bytes(span.body_start(), len, "part settings")?;
    let mut head = Vec::new();
    let mut records = Vec::new();
    for (i, piece) in scan(&body, span.body_start())?.into_iter().enumerate() {
        match piece {
            Piece::Raw { bytes, .. } if i == 0 => head = bytes.to_vec(),
            Piece::Obj(obj) if obj.version == 5 => records.push(parse_setting(obj, 0, true)?),
            Piece::Raw { .. } | Piece::Obj(_) => {}
        }
    }
    Ok(Settings { head, records })
}

/// Loads the object at `span` and hands it to `f`.
fn with_object<T>(
    src: &mut impl Source,
    span: Span,
    what: &'static str,
    f: impl FnOnce(Obj<'_>) -> Result<T, ReadError>,
) -> Result<T, ReadError> {
    let bytes = src.bytes(span.offset, span.len, what)?;
    f(Cursor::new(&bytes, span.offset).object(what)?)
}

/// One list entry: flags, color, the name and a second name (empty unless the part is in a
/// folder), each up to a zero byte, and `rest_len` more bytes.
fn entry(src: &mut impl Source, list: &mut List, rest_len: usize) -> Result<Entry, ReadError> {
    let flags = list.u32(src)?;
    let mut color = [0; 4];
    color.copy_from_slice(list.take(src, 4, "list entry")?);
    let name = latin1(list.text(src)?);
    let folder = Some(latin1(list.text(src)?)).filter(|f| !f.is_empty());
    let rest = list.take(src, rest_len, "list entry")?.to_vec();
    Ok(Entry {
        flags,
        color,
        name,
        folder,
        rest,
    })
}

/// Reads the plain fields of a list that objects interrupt.
struct List {
    items: VecDeque<Item>,
    raw: Vec<u8>,
    pos: usize,
    /// File offset of `raw[0]`.
    base: usize,
    /// The plain bytes in the whole list: a bound on its entries, since each takes some.
    budget: usize,
}

impl List {
    fn new(items: Vec<Item>) -> Self {
        let budget = items
            .iter()
            .map(|item| match item {
                Item::Raw { len, .. } => *len,
                Item::Obj(_) => 0,
            })
            .sum();
        Self {
            items: items.into(),
            raw: Vec::new(),
            pos: 0,
            base: 0,
            budget,
        }
    }

    /// Moves on to the next plain fields once the current ones are used up.
    fn refill(&mut self, src: &mut impl Source) -> Result<(), ReadError> {
        if self.pos == self.raw.len()
            && let Some(&Item::Raw { offset, len }) = self.items.front()
        {
            self.raw = src.bytes(offset, len, "part list")?;
            self.items.pop_front();
            (self.pos, self.base) = (0, offset);
        }
        Ok(())
    }

    fn done(&mut self, src: &mut impl Source) -> Result<bool, ReadError> {
        self.refill(src)?;
        Ok(self.pos == self.raw.len() && self.items.is_empty())
    }

    fn take(
        &mut self,
        src: &mut impl Source,
        n: usize,
        what: &'static str,
    ) -> Result<&[u8], ReadError> {
        self.refill(src)?;
        let at = self.pos;
        let bytes = self.raw.get(at..at.saturating_add(n));
        let bytes = bytes.ok_or(ReadError::Truncated {
            offset: self.base + at,
            what,
        })?;
        self.pos += n;
        Ok(bytes)
    }

    fn u8(&mut self, src: &mut impl Source) -> Result<u8, ReadError> {
        Ok(self.take(src, 1, "list entry")?[0])
    }

    fn u32(&mut self, src: &mut impl Source) -> Result<u32, ReadError> {
        let mut w = [0; 4];
        w.copy_from_slice(self.take(src, 4, "list entry")?);
        Ok(u32::from_le_bytes(w))
    }

    /// Text up to and without its zero byte.
    fn text(&mut self, src: &mut impl Source) -> Result<&[u8], ReadError> {
        self.refill(src)?;
        let rest = self.raw.get(self.pos..).unwrap_or_default();
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(ReadError::Truncated {
                offset: self.base + self.pos,
                what: "list entry name",
            })?;
        Ok(&self.take(src, len + 1, "list entry name")?[..len])
    }

    /// A part's name, settings and record objects, if the plain fields are used up and
    /// objects come next.
    fn objects(&mut self, tool: usize) -> Result<Option<[Span; 3]>, ReadError> {
        if self.pos != self.raw.len() {
            return Ok(None);
        }
        let mut spans = Vec::new();
        while let Some(&Item::Obj(span)) = self.items.front() {
            spans.push(span);
            self.items.pop_front();
        }
        if spans.is_empty() {
            return Ok(None);
        }
        <[Span; 3]>::try_from(spans)
            .map(Some)
            .map_err(|_| ReadError::BadLayout {
                offset: tool,
                reason: "a part in the list has a name, settings and record",
            })
    }
}
