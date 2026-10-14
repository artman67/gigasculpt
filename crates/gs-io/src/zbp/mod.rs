//! Reader for ZBP brush files (.ZBP), learned only from brush files and a small reader that
//! reads them (clean room: never from the program that writes them). Layout notes and open
//! questions: `docs/decisions/0004-zbp-format.md`.
//!
//! A file is a 92-byte preamble, the brush icon, a file header (format version, base brush
//! type, intensities, credit text) and one brush object running to the end of the file. The
//! brush object holds the alpha, any embedded meshes and the named settings. [`read`] decodes
//! all of it into a [`ZbpBrush`]; parts whose meaning is not known yet are kept as bytes, so
//! an import can list them instead of dropping them.

mod body;

pub use crate::container::{
    Curve, CurvePoint, Image, NamedImage, Preamble, RawObject, Setting, SettingKind,
};

use crate::container::chunk::{Cursor, Piece, scan};
use crate::container::record::latin1;
use crate::container::{PREAMBLE, picture};

/// Why a brush file could not be read (the error type every reader here shares).
pub type ZbpError = crate::ReadError;

/// The kind tag of a brush file's preamble.
const TAG: &[u8; 4] = b"PBZ\0";

/// Everything read from one brush file.
#[derive(Debug, Clone, PartialEq)]
pub struct ZbpBrush {
    pub preamble: Preamble,
    /// The brush icon (96 by 96, 4 bytes per pixel, in every file seen).
    pub icon: NamedImage,
    pub header: Header,
    pub head: BrushHead,
    /// The brush alpha, 16-bit grey in nearly every file.
    pub alpha: Option<NamedImage>,
    /// Embedded meshes (insert and multi-insert brushes), not decoded yet.
    pub meshes: Vec<MeshEntry>,
    /// Named settings in file order.
    pub settings: Vec<Setting>,
    /// Plain-field blocks between the meshes or alpha and the settings, and between groups of
    /// settings, not decoded yet.
    pub blocks: Vec<Block>,
    /// Other objects found among the settings, with any images inside them.
    pub sections: Vec<Section>,
}

/// The file header between the icon and the brush object.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Header layout version (1002, 1005, 1006 or 1007 seen).
    pub version: u32,
    /// The base brush type number.
    pub base_type: u32,
    pub z_intensity: f32,
    pub rgb_intensity: f32,
    /// Credit text (author, web address), non-empty lines only. Empty for unknown versions.
    pub credits: Vec<String>,
    /// The whole header as stored, for the fields not decoded yet.
    pub raw: Vec<u8>,
}

/// The fixed fields at the start of the brush object.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushHead {
    /// Brush object version (0x41 to 0x5A seen).
    pub version: u16,
    /// Draw size, stored from version 0x4C on.
    pub draw_size: Option<f32>,
    pub z_intensity: f32,
    pub rgb_intensity: f32,
    /// A float whose meaning is not known yet.
    pub unknown: f32,
    /// Nonzero when an alpha follows (0x40 to 0x46 seen); the low bits are not understood.
    pub alpha_flags: u32,
}

/// One embedded mesh of an insert or multi-insert brush.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshEntry {
    /// The field before the entry (2 in every file seen).
    pub tag: u32,
    pub name: String,
    /// The mesh object, not decoded yet.
    pub mesh: RawObject,
    /// The object that follows each mesh, not decoded yet.
    pub companion: RawObject,
}

/// Plain fields between objects, with the number of settings read before them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub after: usize,
    pub bytes: Vec<u8>,
}

/// An object among the settings that is not a setting, with the images found inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub after: usize,
    pub object: RawObject,
    pub images: Vec<Image>,
}

/// Reads a whole brush file.
pub fn read(bytes: &[u8]) -> Result<ZbpBrush, ZbpError> {
    let preamble = preamble(bytes)?;
    let mut c = Cursor::new(bytes, 0);
    c.take(PREAMBLE, "preamble")?;
    let icon = picture(&mut c, "icon")?;
    let offset = c.offset();
    let mut pieces = scan(c.rest(), offset)?.into_iter();
    let layout = |reason| ZbpError::BadLayout { offset, reason };
    let (
        Some(Piece::Raw {
            offset: header_at,
            bytes: raw,
        }),
        Some(Piece::Obj(obj)),
        None,
    ) = (pieces.next(), pieces.next(), pieces.next())
    else {
        return Err(layout(
            "expected the file header, then the brush object up to the end",
        ));
    };
    let header = header(raw, header_at)?;
    let body = body::parse_body(obj)?;
    Ok(ZbpBrush {
        preamble,
        icon,
        header,
        head: body.head,
        alpha: body.alpha,
        meshes: body.meshes,
        settings: body.settings,
        blocks: body.blocks,
        sections: body.sections,
    })
}

fn preamble(bytes: &[u8]) -> Result<Preamble, ZbpError> {
    match crate::container::preamble(bytes, &[TAG])? {
        Some((preamble, _)) => Ok(preamble),
        None => Err(ZbpError::NotZbp),
    }
}

/// Where the credit text starts, by header version.
fn credits_at(version: u32) -> Option<usize> {
    match version {
        1005 => Some(92),
        1006 => Some(96),
        1007 => Some(116),
        _ => None,
    }
}

fn header(raw: &[u8], offset: usize) -> Result<Header, ZbpError> {
    let mut c = Cursor::new(raw, offset);
    let version = c.u32("file header")?;
    let base_type = c.u32("file header")?;
    let z_intensity = c.f32("file header")?;
    let rgb_intensity = c.f32("file header")?;
    let text = credits_at(version)
        .and_then(|at| raw.get(at..))
        .unwrap_or_default();
    let credits = text
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(latin1)
        .collect();
    Ok(Header {
        version,
        base_type,
        z_intensity,
        rgb_intensity,
        credits,
        raw: raw.to_vec(),
    })
}
