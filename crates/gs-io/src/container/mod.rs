//! What every file kind here (brushes, tools, projects) shares, learned only from the files
//! (clean room): the 92-byte preamble, the object container, planar arrays with their
//! run-length strips, named setting records and the name, rectangle and picture objects.
//! Layout notes: `docs/decisions/0004-zbp-format.md` and `docs/decisions/0005-ztl-format.md`.

pub(crate) mod chunk;
pub(crate) mod planar;
pub(crate) mod record;
mod types;

pub use types::{Curve, CurvePoint, Image, NamedImage, Preamble, RawObject, Setting, SettingKind};

use crate::ReadError;
use chunk::{Cursor, Obj};
use record::latin1;

/// Length of the preamble before the first object.
pub(crate) const PREAMBLE: usize = 92;
/// Every file starts with these 11 bytes, the format's signature text.
const SIGNATURE: &[u8] = &[
    0x5A, 0x42, 0x72, 0x75, 0x73, 0x68, 0x20, 0x46, 0x69, 0x6C, 0x65,
];
/// Where the file kind tag sits; the format number and the writer's version text follow it.
const TAG_AT: usize = 76;

/// Reads the preamble of a file whose kind tag is one of `tags` (`PBZ\0` for a brush, `LTZ\0`
/// for a tool, `RPZ\0` for a project) and says which one; `None` if the signature or the tag
/// differs.
pub(crate) fn preamble(
    bytes: &[u8],
    tags: &[&[u8; 4]],
) -> Result<Option<(Preamble, usize)>, ReadError> {
    if !bytes.starts_with(SIGNATURE) {
        return Ok(None);
    }
    let mut c = Cursor::new(bytes, 0);
    let copyright = c.take(TAG_AT, "preamble")?;
    let tag = c.take(4, "preamble")?;
    let Some(kind) = tags.iter().position(|t| t[..] == *tag) else {
        return Ok(None);
    };
    let format = c.u32("preamble")?;
    let writer = c.take(PREAMBLE - c.offset(), "preamble")?;
    let preamble = Preamble {
        copyright: text_before_zero(copyright),
        format,
        writer: text_before_zero(writer),
    };
    Ok(Some((preamble, kind)))
}

/// The text up to the first zero byte.
pub(crate) fn text_before_zero(bytes: &[u8]) -> String {
    latin1(bytes.split(|&b| b == 0).next().unwrap_or_default())
}

/// A name object, a rectangle object and an image container: how icons and alphas are stored.
pub(crate) fn picture(c: &mut Cursor<'_>, what: &'static str) -> Result<NamedImage, ReadError> {
    let name = name(c.object(what)?)?;
    c.object(what)?;
    let container = c.object(what)?;
    let image = planar::find_images(container)?.into_iter().next();
    let reason = "image container holds no pixels";
    let image = image.ok_or(ReadError::BadImage {
        offset: container.offset,
        reason,
    })?;
    Ok(NamedImage { name, image })
}

/// A name object: 2 flag bytes, then the text up to a zero byte.
pub(crate) fn name(obj: Obj<'_>) -> Result<String, ReadError> {
    let text = obj.body.get(2..).ok_or(ReadError::Truncated {
        offset: obj.offset,
        what: "name",
    })?;
    Ok(text_before_zero(text))
}
