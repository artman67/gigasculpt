//! The object container everything in a brush, tool or project file is stored in, and
//! bounds-checked readers.
//!
//! An object is a 4-byte start marker, a `u16` version, a `u32` total length (header and end
//! marker included), the body, and a 4-byte end marker. The version belongs to the kind of
//! object (a setting record is version 5, a value is version 1), so what an object is follows
//! from where it sits, not from a type tag. Between objects there can be plain fields.
//!
//! Small files are read from memory ([`Cursor`], [`scan`]); big ones through a [`Source`], which
//! reads only the bytes asked for, so a reader can walk a 2 GB file object by object.

use std::io::{Read, Seek, SeekFrom};

use crate::ReadError;

pub(crate) const START: [u8; 4] = [0x03, 0x80, 0x01, 0x00];
pub(crate) const END: [u8; 4] = [0x03, 0x88, 0x00, 0x00];
/// Start marker, version and length.
const HEAD: usize = 10;
/// How many bytes a search for the next start marker reads at a time.
const SEARCH_CHUNK: usize = 256;
/// How far the search window moves: windows overlap by a marker's length less one, so a
/// marker across a window edge is still seen. Any smaller step would work too.
const SEARCH_STEP: usize = 253;
const _: () = assert!(
    SEARCH_STEP + START.len() - 1 == SEARCH_CHUNK,
    "search windows must overlap by a marker less one byte"
);

/// Random access to a file's bytes by file offset.
pub(crate) trait Source {
    /// `n` bytes at `offset`, or `Truncated` naming `what` if the file ends first.
    fn bytes(&mut self, offset: usize, n: usize, what: &'static str) -> Result<Vec<u8>, ReadError>;
}

/// Bytes in memory that start at file offset `base`.
pub(crate) struct Slice<'a> {
    data: &'a [u8],
    base: usize,
}

impl<'a> Slice<'a> {
    pub fn new(data: &'a [u8], base: usize) -> Self {
        Self { data, base }
    }
}

impl Source for Slice<'_> {
    fn bytes(&mut self, offset: usize, n: usize, what: &'static str) -> Result<Vec<u8>, ReadError> {
        let found = offset
            .checked_sub(self.base)
            .and_then(|from| self.data.get(from..from.checked_add(n)?));
        found
            .map(<[u8]>::to_vec)
            .ok_or(ReadError::Truncated { offset, what })
    }
}

/// A file read on demand through `Read + Seek`.
pub(crate) struct Stream<R> {
    inner: R,
    end: usize,
}

impl<R: Read + Seek> Stream<R> {
    pub fn new(mut inner: R) -> Result<Self, ReadError> {
        let io = |kind| ReadError::Io { offset: 0, kind };
        let end = inner.seek(SeekFrom::End(0)).map_err(|e| io(e.kind()))?;
        let end = usize::try_from(end).map_err(|_| io(std::io::ErrorKind::FileTooLarge))?;
        Ok(Self { inner, end })
    }

    /// The offset just past the last byte.
    pub fn end(&self) -> usize {
        self.end
    }
}

impl<R: Read + Seek> Source for Stream<R> {
    fn bytes(&mut self, offset: usize, n: usize, what: &'static str) -> Result<Vec<u8>, ReadError> {
        if offset.checked_add(n).is_none_or(|end| end > self.end) {
            return Err(ReadError::Truncated { offset, what });
        }
        let io = |e: std::io::Error| ReadError::Io {
            offset,
            kind: e.kind(),
        };
        let mut buf = vec![0; n];
        self.inner
            .seek(SeekFrom::Start(offset as u64))
            .map_err(io)?;
        self.inner.read_exact(&mut buf).map_err(io)?;
        Ok(buf)
    }
}

/// Where an object sits: the offset of its start marker, its version and its total length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub offset: usize,
    pub version: u16,
    pub len: usize,
}

impl Span {
    /// File offset of the first body byte.
    pub fn body_start(&self) -> usize {
        self.offset + HEAD
    }

    /// File offset just past the last body byte.
    pub fn body_end(&self) -> usize {
        self.offset + self.len - END.len()
    }
}

/// What lies between two points of a file: objects and the plain fields between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Obj(Span),
    Raw { offset: usize, len: usize },
}

/// The object whose start marker is at `offset`; it must end by `limit`.
pub(crate) fn object_at(
    src: &mut impl Source,
    offset: usize,
    limit: usize,
    what: &'static str,
) -> Result<Span, ReadError> {
    let avail = limit.saturating_sub(offset);
    let head = src.bytes(offset, avail.min(HEAD), what)?;
    if !head.starts_with(&START) {
        return Err(ReadError::BadLayout {
            offset,
            reason: what,
        });
    }
    let mut c = Cursor::new(&head, offset);
    c.take(START.len(), what)?;
    let version = c.u16(what)?;
    let len = c.u32(what)? as usize;
    if len < HEAD + END.len() {
        return Err(ReadError::BadObject {
            offset,
            reason: "length shorter than the object header",
        });
    }
    if len > avail {
        return Err(ReadError::Truncated { offset, what });
    }
    if src.bytes(offset + len - END.len(), END.len(), what)? != END {
        return Err(ReadError::BadObject {
            offset,
            reason: "end marker missing",
        });
    }
    Ok(Span {
        offset,
        version,
        len,
    })
}

/// The first start marker that lies wholly in `from..end`.
fn find_start(src: &mut impl Source, from: usize, end: usize) -> Result<Option<usize>, ReadError> {
    let mut at = from;
    for _ in 0..end.saturating_sub(from).div_ceil(SEARCH_STEP) {
        let n = end.saturating_sub(at).min(SEARCH_CHUNK);
        let window = src.bytes(at, n, "plain fields")?;
        if let Some(i) = window.windows(START.len()).position(|w| w == START) {
            return Ok(Some(at + i));
        }
        at += SEARCH_STEP;
    }
    Ok(None)
}

/// Splits `start..end` into objects and the plain fields between them, reading only object
/// headers and the plain fields. Every start marker found must begin a well-formed object.
/// The walk takes at most one step per byte and each step moves forward, so it always ends.
pub(crate) fn walk(
    src: &mut impl Source,
    start: usize,
    end: usize,
) -> Result<Vec<Item>, ReadError> {
    let mut items = Vec::new();
    let mut pos = start;
    for _ in start..end {
        if pos >= end {
            break;
        }
        let item = match find_start(src, pos, end)? {
            Some(at) if at == pos => Item::Obj(object_at(src, pos, end, "object")?),
            found => Item::Raw {
                offset: pos,
                len: found.unwrap_or(end) - pos,
            },
        };
        pos += match item {
            Item::Obj(span) => span.len,
            Item::Raw { len, .. } => len,
        };
        items.push(item);
    }
    Ok(items)
}

/// One object in memory: its version, where its start marker is in the file, and its body.
#[derive(Clone, Copy)]
pub(crate) struct Obj<'a> {
    pub version: u16,
    pub offset: usize,
    pub body: &'a [u8],
}

impl<'a> Obj<'a> {
    /// A reader over the body, reporting offsets in file terms.
    pub fn reader(&self) -> Cursor<'a> {
        Cursor {
            data: self.body,
            pos: 0,
            base: self.offset + HEAD,
        }
    }
}

/// What lies between two points of bytes in memory: objects and the plain fields between them.
pub(crate) enum Piece<'a> {
    Obj(Obj<'a>),
    Raw { offset: usize, bytes: &'a [u8] },
}

/// Reads fields front to back; every read either returns the bytes or says what was cut off.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    /// File offset of `data[0]`.
    base: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8], base: usize) -> Self {
        Self { data, pos: 0, base }
    }

    /// File offset of the next byte.
    pub fn offset(&self) -> usize {
        self.base + self.pos
    }

    pub fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], ReadError> {
        let found = self
            .pos
            .checked_add(n)
            .and_then(|end| Some((end, self.data.get(self.pos..end)?)));
        let (end, bytes) = found.ok_or(ReadError::Truncated {
            offset: self.offset(),
            what,
        })?;
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], ReadError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N, what)?);
        Ok(out)
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8, ReadError> {
        Ok(self.array::<1>(what)?[0])
    }

    pub fn u16(&mut self, what: &'static str) -> Result<u16, ReadError> {
        self.array(what).map(u16::from_le_bytes)
    }

    pub fn u32(&mut self, what: &'static str) -> Result<u32, ReadError> {
        self.array(what).map(u32::from_le_bytes)
    }

    pub fn f32(&mut self, what: &'static str) -> Result<f32, ReadError> {
        self.array(what).map(f32::from_le_bytes)
    }

    /// Everything not read yet.
    pub fn rest(&mut self) -> &'a [u8] {
        let rest = self.peek();
        self.pos = self.data.len();
        rest
    }

    /// Everything not read yet, without reading it.
    fn peek(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or_default()
    }

    /// True if an object start marker comes next.
    pub fn at_object(&self) -> bool {
        self.peek().starts_with(&START)
    }

    /// The object that must come next.
    pub fn object(&mut self, what: &'static str) -> Result<Obj<'a>, ReadError> {
        let (offset, data) = (self.offset(), self.peek());
        let span = object_at(
            &mut Slice::new(data, offset),
            offset,
            offset + data.len(),
            what,
        )?;
        let whole = self.take(span.len, what)?;
        Ok(obj_in(whole, span))
    }
}

/// The object `span` whose bytes, start marker to end marker, are `whole`.
fn obj_in(whole: &[u8], span: Span) -> Obj<'_> {
    Obj {
        version: span.version,
        offset: span.offset,
        body: whole.get(HEAD..span.len - END.len()).unwrap_or_default(),
    }
}

/// Splits `data` (at file offset `base`) into objects and the plain fields between them.
/// Every start marker found must begin a well-formed object.
pub(crate) fn scan(data: &[u8], base: usize) -> Result<Vec<Piece<'_>>, ReadError> {
    let items = walk(&mut Slice::new(data, base), base, base + data.len())?;
    let at = |offset: usize, len: usize| data.get(offset - base..offset - base + len);
    Ok(items
        .into_iter()
        .map(|item| match item {
            Item::Obj(span) => {
                Piece::Obj(obj_in(at(span.offset, span.len).unwrap_or_default(), span))
            }
            Item::Raw { offset, len } => Piece::Raw {
                offset,
                bytes: at(offset, len).unwrap_or_default(),
            },
        })
        .collect())
}
