//! How a chunk's values become bytes and back: a filter that puts alike bytes together, then
//! zstd, then a BLAKE3 hash of what is stored (decision 0016, section 2).
//!
//! Every filter is byte planes of the chunk's element: byte `b` of element `i` goes to
//! `b * n + i`. For a point (three `f32`) that is the same as per-axis planes then byte planes;
//! for a face it follows the corner transform below. The high bytes of nearby floats and ids are
//! nearly equal, so once they sit together zstd finds them (research 15, section 2.1).

use gs_core::{NO, Vec3};

/// zstd level: level 3 gains 3% on positions and costs 30 to 60% more time (research 15, 7.4).
const LEVEL: i32 = 1;

/// A chunk's filter, by the name its table entry gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    /// Byte planes of 12-byte points.
    Points,
    /// The corner transform, then byte planes of 16-byte faces.
    Faces,
    /// Byte planes of 4-byte words.
    Words,
    /// Bytes as they are.
    Bytes,
}

impl Filter {
    /// Each filter's name in files and the bytes of one element.
    fn row(self) -> (&'static str, usize) {
        match self {
            Filter::Points => ("points", 12),
            Filter::Faces => ("faces", 16),
            Filter::Words => ("words", 4),
            Filter::Bytes => ("bytes", 1),
        }
    }

    pub fn name(self) -> &'static str {
        self.row().0
    }

    /// Bytes per element.
    pub fn width(self) -> usize {
        self.row().1
    }
}

/// Byte planes: byte `b` of element `i` (of `width` bytes) goes to `b * n + i`.
pub(crate) fn planes(bytes: &[u8], width: usize) -> Vec<u8> {
    let n = bytes.len() / width;
    let mut out = vec![0u8; n * width];
    for (i, element) in bytes.chunks_exact(width).enumerate() {
        for (b, &x) in element.iter().enumerate() {
            out[b * n + i] = x;
        }
    }
    out
}

/// The inverse of [`planes`].
pub(crate) fn unplanes(planes: &[u8], width: usize) -> Vec<u8> {
    let n = planes.len() / width;
    let mut out = vec![0u8; n * width];
    for (i, element) in out.chunks_exact_mut(width).enumerate() {
        for (b, x) in element.iter_mut().enumerate() {
            *x = planes[b * n + i];
        }
    }
    out
}

/// Points as little-endian bytes.
pub(crate) fn point_bytes(points: &[Vec3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(points.len() * 12);
    for p in points {
        for c in p.bits() {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

pub(crate) fn points_of(bytes: &[u8]) -> Vec<Vec3> {
    let word = |b: &[u8]| f32::from_bits(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    (bytes.as_chunks::<12>().0.iter())
        .map(|p| Vec3::new(word(&p[0..4]), word(&p[4..8]), word(&p[8..12])))
        .collect()
}

/// 32-bit words as little-endian bytes.
pub(crate) fn word_bytes(words: impl Iterator<Item = u32>) -> Vec<u8> {
    words.flat_map(u32::to_le_bytes).collect()
}

pub(crate) fn words_of(bytes: &[u8]) -> Vec<u32> {
    (bytes.as_chunks::<4>().0.iter())
        .map(|b| u32::from_le_bytes(*b))
        .collect()
}

/// One corner relative to the previous face's first corner `prev`: a bijection on 32-bit words
/// that keeps a missing corner (`NO`) as `NO` whatever `prev` is, so the fourth corner of every
/// triangle compresses to nothing. The word `NO` would otherwise take is given to `prev - 1`.
fn corner_out(c: u32, prev: u32) -> u32 {
    if c == NO {
        return NO;
    }
    match c.wrapping_sub(prev) {
        NO => NO.wrapping_sub(prev),
        d => d,
    }
}

/// The inverse of [`corner_out`].
fn corner_in(d: u32, prev: u32) -> u32 {
    if d == NO {
        NO
    } else if prev != 0 && d == NO.wrapping_sub(prev) {
        prev.wrapping_sub(1)
    } else {
        d.wrapping_add(prev)
    }
}

/// Face slots as bytes: every corner relative to the previous face's first corner (zero before
/// the first face), little-endian.
pub(crate) fn face_bytes(faces: &[[u32; 4]]) -> Vec<u8> {
    let mut prev = 0;
    let mut out = Vec::with_capacity(faces.len() * 16);
    for f in faces {
        for &c in f {
            out.extend_from_slice(&corner_out(c, prev).to_le_bytes());
        }
        prev = f[0];
    }
    out
}

pub(crate) fn faces_of(bytes: &[u8]) -> Vec<[u32; 4]> {
    let words = words_of(bytes);
    let mut prev = 0;
    (words.as_chunks::<4>().0.iter())
        .map(|w| {
            let f = [0, 1, 2, 3].map(|j| corner_in(w[j], prev));
            prev = f[0];
            f
        })
        .collect()
}

/// A chunk ready to store: its bytes, the size before compression, and its hash.
pub(crate) struct Packed {
    pub stored: Vec<u8>,
    pub raw: u64,
    pub hash: String,
}

/// Filters and compresses `bytes` (elements of the filter's width).
pub(crate) fn pack(bytes: &[u8], filter: Filter) -> std::io::Result<Packed> {
    let filtered = match filter {
        Filter::Bytes => bytes.to_vec(),
        other => planes(bytes, other.width()),
    };
    let stored = zstd::bulk::compress(&filtered, LEVEL)?;
    Ok(Packed {
        hash: hash16(&stored),
        stored,
        raw: bytes.len() as u64,
    })
}

/// Decompresses `stored` into exactly `raw` bytes and undoes the filter; `None` if it does not
/// decompress to that size.
pub(crate) fn unpack(stored: &[u8], raw: usize, filter: Filter) -> Option<Vec<u8>> {
    let bytes = zstd::bulk::decompress(stored, raw).ok()?;
    if bytes.len() != raw || !raw.is_multiple_of(filter.width()) {
        return None;
    }
    Some(match filter {
        Filter::Bytes => bytes,
        other => unplanes(&bytes, other.width()),
    })
}

/// The first 16 bytes of the BLAKE3 hash, as 32 hex digits.
pub(crate) fn hash16(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..32].to_string()
}

/// zstd, unfiltered: for the manifest.
pub(crate) fn compress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::bulk::compress(bytes, LEVEL)
}

pub(crate) fn decompress(stored: &[u8], raw: usize) -> Option<Vec<u8>> {
    zstd::bulk::decompress(stored, raw)
        .ok()
        .filter(|b| b.len() == raw)
}
