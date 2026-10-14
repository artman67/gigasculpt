//! The fixed 64-byte header and trailer (decision 0016, section 1). Little-endian.
//!
//! Header: signature (8), format version (u32), oldest reader version (u32), kind (u32: 0 scene,
//! 1 parts), reserved (u32), preview offset (u64), preview length (u64), zeros to 64.
//! Trailer: manifest offset (u64), manifest stored size (u64), manifest raw size (u64), manifest
//! BLAKE3-128 (16 bytes), file length (u64), reserved (u64), end signature (8).

use super::{FileKind, SceneError};

pub(crate) const HEADER: usize = 64;
pub(crate) const TRAILER: usize = 64;
/// PNG's pattern: a high-bit byte, the name, CR LF, an end-of-file byte and LF, so a text-mode
/// transfer or a 7-bit channel damages it visibly.
pub(crate) const SIGNATURE: [u8; 8] = [0x89, b'G', b'S', b'S', 0x0D, 0x0A, 0x1A, 0x0A];
pub(crate) const END: [u8; 8] = *b"GSS-END\n";

/// What the header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The format version that wrote the file.
    pub format: u32,
    /// The oldest format version that can read it.
    pub oldest_reader: u32,
    pub kind: FileKind,
    /// Where the preview PNG is, and its length; `(0, 0)` without one.
    pub preview: (u64, u64),
}

/// What the trailer says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Trailer {
    pub manifest_at: u64,
    pub manifest_stored: u64,
    pub manifest_raw: u64,
    pub manifest_hash: [u8; 16],
    pub length: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(w)
}

impl Header {
    pub(crate) fn bytes(&self) -> [u8; HEADER] {
        let mut b = [0u8; HEADER];
        b[..8].copy_from_slice(&SIGNATURE);
        b[8..12].copy_from_slice(&self.format.to_le_bytes());
        b[12..16].copy_from_slice(&self.oldest_reader.to_le_bytes());
        b[16..20].copy_from_slice(&self.kind.code().to_le_bytes());
        b[24..32].copy_from_slice(&self.preview.0.to_le_bytes());
        b[32..40].copy_from_slice(&self.preview.1.to_le_bytes());
        b
    }

    /// Reads a header: `NotScene` without the signature, `Incomplete` when it is cut short,
    /// `Damaged` with an unknown kind.
    pub(crate) fn read(b: &[u8]) -> Result<Header, SceneError> {
        if b.len() < 8 || b[..8] != SIGNATURE {
            return Err(SceneError::NotScene);
        }
        if b.len() < HEADER {
            return Err(SceneError::Incomplete {
                reason: "the file ends inside its header".into(),
            });
        }
        let kind = FileKind::of_code(u32_at(b, 16)).ok_or_else(|| SceneError::Damaged {
            offset: 16,
            reason: "the header names an unknown file kind".into(),
        })?;
        Ok(Header {
            format: u32_at(b, 8),
            oldest_reader: u32_at(b, 12),
            kind,
            preview: (u64_at(b, 24), u64_at(b, 32)),
        })
    }
}

impl Trailer {
    pub(crate) fn bytes(&self) -> [u8; TRAILER] {
        let mut b = [0u8; TRAILER];
        b[..8].copy_from_slice(&self.manifest_at.to_le_bytes());
        b[8..16].copy_from_slice(&self.manifest_stored.to_le_bytes());
        b[16..24].copy_from_slice(&self.manifest_raw.to_le_bytes());
        b[24..40].copy_from_slice(&self.manifest_hash);
        b[40..48].copy_from_slice(&self.length.to_le_bytes());
        b[56..64].copy_from_slice(&END);
        b
    }

    /// Reads the last 64 bytes of a file of `length` bytes. A file cut short, or one whose
    /// trailer does not point inside it, is incomplete.
    pub(crate) fn read(b: &[u8], length: u64) -> Result<Trailer, SceneError> {
        let incomplete = |reason: &str| SceneError::Incomplete {
            reason: reason.into(),
        };
        if b.len() < TRAILER || b[56..64] != END {
            return Err(incomplete(
                "the end signature is missing: the file was cut short",
            ));
        }
        let mut manifest_hash = [0u8; 16];
        manifest_hash.copy_from_slice(&b[24..40]);
        let t = Trailer {
            manifest_at: u64_at(b, 0),
            manifest_stored: u64_at(b, 8),
            manifest_raw: u64_at(b, 16),
            manifest_hash,
            length: u64_at(b, 40),
        };
        if t.length != length {
            return Err(incomplete("the file is not the length its trailer records"));
        }
        let end = t.manifest_at.checked_add(t.manifest_stored);
        let body = length - TRAILER as u64;
        if t.manifest_at < HEADER as u64 || end.is_none_or(|e| e > body) {
            return Err(incomplete("the trailer points outside the file"));
        }
        Ok(t)
    }
}

/// Zero bytes that bring `at` to the next multiple of 8.
pub(crate) fn padding(at: u64) -> usize {
    ((8 - at % 8) % 8) as usize
}
