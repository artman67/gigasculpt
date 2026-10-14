//! Planar arrays: how these files store images (the icon, the alpha, textures) and the arrays
//! inside meshes.
//!
//! A planar array (object version 6) holds `size` bytes of elements of `element` bytes each,
//! laid out in rows of `row` bytes. The bytes are cut into strips (128 KiB in every file seen);
//! a table gives each strip's stored length and ends at the first zero entry. A stored strip is
//! a `u32` encoding (1 = run-length) and the run-length data. Decoded, a strip holds its first
//! element byte for every element, then its second byte for every element, and so on: a 16-bit
//! image strip is all its low bytes, then all its high bytes.

use super::chunk::{Cursor, Obj, Piece, Source, Span, scan};
use super::types::Image;
use crate::ReadError;

/// How deep to look inside objects for planar arrays (a texture sits three levels down).
const SEARCH_DEPTH: u32 = 4;
/// Size, element size, row size, 8 fixed bytes and the strip count.
const FIXED_HEAD: usize = 20;

/// The fixed fields and strip table of a planar array.
struct Layout {
    size: usize,
    element: usize,
    row: usize,
    strips: Vec<usize>,
}

/// Reads the fixed fields and the strip table, and checks that the sizes fit together.
fn layout(c: &mut Cursor<'_>, offset: usize) -> Result<Layout, ReadError> {
    let size = c.u32("array size")? as usize;
    let element = usize::from(c.u16("array element size")?);
    let row = usize::from(c.u16("array row size")?);
    c.take(8, "array header")?;
    let strips = strip_table(c)?;
    let bad = |reason| ReadError::BadImage { offset, reason };
    if element == 0 || row == 0 {
        return Err(bad("zero element or row size"));
    }
    if !row.is_multiple_of(element) || !size.is_multiple_of(row) {
        return Err(bad("size is not a whole number of rows"));
    }
    Ok(Layout {
        size,
        element,
        row,
        strips,
    })
}

/// Stored strip lengths: the table entries before the first zero. Entries after it are
/// leftovers and are skipped.
fn strip_table(c: &mut Cursor<'_>) -> Result<Vec<usize>, ReadError> {
    let count = c.u32("strip count")? as usize;
    let table = c.take(count.saturating_mul(4), "strip table")?;
    let lengths = table
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&e| u32::from_le_bytes(e) as usize);
    Ok(lengths.take_while(|&len| len != 0).collect())
}

/// Decodes one stored strip into its byte planes.
fn unpack_strip(strip: &[u8], element: usize, offset: usize) -> Result<Vec<u8>, ReadError> {
    let bad = |reason| ReadError::BadImage { offset, reason };
    let (encoding, packed) = strip
        .split_at_checked(4)
        .ok_or(bad("strip shorter than its header"))?;
    if encoding != [1, 0, 0, 0] {
        return Err(bad("unknown strip encoding"));
    }
    let plain = unpack_runs(packed).ok_or(bad("run-length data cut off"))?;
    if !plain.len().is_multiple_of(element) {
        return Err(bad("strip is not a whole number of elements"));
    }
    Ok(plain)
}

/// The finished image, once every strip is in.
fn finish(l: &Layout, data: Vec<u8>, offset: usize) -> Result<Image, ReadError> {
    if data.len() != l.size {
        return Err(wrong_size(offset));
    }
    Ok(Image {
        width: (l.row / l.element) as u32,
        height: (l.size / l.row) as u32,
        bytes_per_pixel: l.element as u16,
        data,
    })
}

fn after_last_strip(offset: usize) -> ReadError {
    ReadError::BadImage {
        offset,
        reason: "data after the last strip",
    }
}

/// Decodes one planar array object in memory into an image with interleaved little-endian
/// pixels.
pub(crate) fn decode(obj: Obj<'_>) -> Result<Image, ReadError> {
    let offset = obj.offset;
    let mut c = obj.reader();
    let l = layout(&mut c, offset)?;
    let mut data = Vec::new();
    for &len in &l.strips {
        let plain = unpack_strip(c.take(len, "array strip")?, l.element, offset)?;
        interleave(&plain, l.element, &mut data);
    }
    if !c.rest().is_empty() {
        return Err(after_last_strip(offset));
    }
    finish(&l, data, offset)
}

/// The layout of the planar array object at `span` in a file, and where its strips start.
fn layout_at(src: &mut impl Source, span: Span) -> Result<(Layout, usize), ReadError> {
    let (body, end) = (span.body_start(), span.body_end());
    // Only the strip count is taken from here; the header is checked on the read below.
    let fixed = src.bytes(body, FIXED_HEAD, "array header")?;
    let count = fixed
        .get(16..FIXED_HEAD)
        .map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize);
    let head_len = count.saturating_mul(4).saturating_add(FIXED_HEAD);
    let head = src.bytes(body, head_len.min(end - body), "strip table")?;
    let l = layout(&mut Cursor::new(&head, body), span.offset)?;
    Ok((l, body + head.len()))
}

fn wrong_size(offset: usize) -> ReadError {
    ReadError::BadImage {
        offset,
        reason: "decoded size differs from the stated size",
    }
}

/// Decodes the planar array object at `span` from a file straight into elements of `N` bytes,
/// one stored strip at a time; `None` if its elements have another size. The output is
/// allocated zeroed at its stated size, which the system only commits as strips fill it.
pub(crate) fn decode_elements<const N: usize>(
    src: &mut impl Source,
    span: Span,
) -> Result<Option<Vec<[u8; N]>>, ReadError> {
    let (l, mut at) = layout_at(src, span)?;
    if l.element != N {
        return Ok(None);
    }
    let end = span.body_end();
    let mut out = vec![[0u8; N]; l.size / N];
    let mut done = 0;
    for &len in &l.strips {
        if len > end - at {
            return Err(ReadError::Truncated {
                offset: at,
                what: "array strip",
            });
        }
        let plain = unpack_strip(&src.bytes(at, len, "array strip")?, N, span.offset)?;
        let count = plain.len() / N;
        let slots = out
            .get_mut(done..done + count)
            .ok_or(wrong_size(span.offset))?;
        for (i, e) in slots.iter_mut().enumerate() {
            for (k, b) in e.iter_mut().enumerate() {
                *b = plain[k * count + i];
            }
        }
        done += count;
        at += len;
    }
    if at != end {
        return Err(after_last_strip(span.offset));
    }
    if done != out.len() {
        return Err(wrong_size(span.offset));
    }
    Ok(Some(out))
}

/// Run-length decoding: a control byte below 128 repeats the next byte that many times; any
/// other control byte `c` is followed by `256 - c` bytes copied as they are.
pub(crate) fn unpack_runs(packed: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = packed;
    while let Some((&control, tail)) = rest.split_first() {
        rest = if control < 128 {
            let (&byte, tail) = tail.split_first()?;
            out.extend(std::iter::repeat_n(byte, usize::from(control)));
            tail
        } else {
            let (literal, tail) = tail.split_at_checked(256 - usize::from(control))?;
            out.extend_from_slice(literal);
            tail
        };
    }
    Some(out)
}

/// Turns one decoded strip (byte planes) into whole elements appended to `out`.
fn interleave(plain: &[u8], element: usize, out: &mut Vec<u8>) {
    let start = out.len();
    out.resize(start + plain.len(), 0);
    let count = plain.len() / element;
    for (plane, bytes) in plain.chunks_exact(count.max(1)).enumerate() {
        let column = out[start..].iter_mut().skip(plane).step_by(element);
        for (dst, &src) in column.zip(bytes) {
            *dst = src;
        }
    }
}

/// Every planar array inside `obj`, decoded, in file order, looking `SEARCH_DEPTH` levels down.
pub(crate) fn find_images(obj: Obj<'_>) -> Result<Vec<Image>, ReadError> {
    let mut images = Vec::new();
    search(obj, SEARCH_DEPTH, &mut images)?;
    Ok(images)
}

fn search(obj: Obj<'_>, depth: u32, out: &mut Vec<Image>) -> Result<(), ReadError> {
    for piece in scan(obj.body, obj.reader().offset())? {
        match piece {
            Piece::Obj(inner) if inner.version == 6 => out.push(decode(inner)?),
            Piece::Obj(inner) if depth > 1 => search(inner, depth - 1, out)?,
            Piece::Obj(_) | Piece::Raw { .. } => {}
        }
    }
    Ok(())
}
