//! Named setting records (object version 5) and the curves some of them carry.
//!
//! A record is 7 head bytes (kind, flags; bit 0 of the third byte is the on/off state), two
//! value objects of 4 bytes each, a curve flag byte with an optional curve object, a name flag
//! byte and the name ending in a zero byte.

use super::chunk::Obj;
use super::types::{Curve, CurvePoint, Setting, SettingKind};
use crate::ReadError;

/// Bytes per curve point: x, y, a point kind and a weight.
const POINT: usize = 16;
/// Where the point count sits in a curve body.
const POINT_COUNT_AT: usize = 24;

/// Reads one record. Brush files name every record; tool files also hold unnamed ones (name
/// flag 0 and nothing after it), which `unnamed_ok` accepts with an empty name.
pub(crate) fn parse_setting(
    obj: Obj<'_>,
    group: u8,
    unnamed_ok: bool,
) -> Result<Setting, ReadError> {
    let offset = obj.offset;
    let bad = |reason| ReadError::BadSetting { offset, reason };
    let mut c = obj.reader();
    let mut head = [0; 7];
    head.copy_from_slice(c.take(7, "setting head")?);
    let value = read_value(c.object("setting value")?)?;
    let second = read_value(c.object("setting second value")?)?;
    let curve = match c.u8("setting curve flag")? {
        0 => None,
        _ => Some(parse_curve(c.object("setting curve")?)?),
    };
    let named = c.u8("setting name flag")? != 0;
    if !named && !unnamed_ok {
        return Err(bad("record has no name"));
    }
    let name = c.rest();
    let name = match (named, name) {
        (false, []) => name,
        (false, _) => return Err(bad("an unnamed record has bytes after its name flag")),
        (true, _) => name
            .strip_suffix(&[0])
            .ok_or(bad("name is not zero-terminated"))?,
    };
    Ok(Setting {
        name: latin1(name),
        group,
        kind: SettingKind::from_byte(head[0]),
        on: head[2] & 1 == 1,
        value,
        second,
        head,
        curve,
    })
}

/// A value object: a 4-byte little-endian float.
fn read_value(obj: Obj<'_>) -> Result<f32, ReadError> {
    let bytes: [u8; 4] = obj.body.try_into().map_err(|_| ReadError::BadSetting {
        offset: obj.offset,
        reason: "value is not 4 bytes",
    })?;
    Ok(f32::from_le_bytes(bytes))
}

/// A curve object: a header holding the point count, then the points at the end of the body.
fn parse_curve(obj: Obj<'_>) -> Result<Curve, ReadError> {
    let mut c = obj.reader();
    c.take(POINT_COUNT_AT, "curve header")?;
    let count = c.u32("curve point count")? as usize;
    let split = count
        .checked_mul(POINT)
        .and_then(|len| obj.body.len().checked_sub(len))
        .filter(|&split| split >= POINT_COUNT_AT + 4)
        .ok_or(ReadError::BadSetting {
            offset: obj.offset,
            reason: "curve shorter than its points",
        })?;
    let (head, points) = obj.body.split_at(split);
    let points = points.as_chunks::<POINT>().0.iter().map(point).collect();
    Ok(Curve {
        version: obj.version,
        head: head.to_vec(),
        points,
    })
}

fn point(p: &[u8; POINT]) -> CurvePoint {
    let word = |i: usize| [p[i], p[i + 1], p[i + 2], p[i + 3]];
    CurvePoint {
        x: f32::from_le_bytes(word(0)),
        y: f32::from_le_bytes(word(4)),
        kind: u32::from_le_bytes(word(8)),
        weight: f32::from_le_bytes(word(12)),
    }
}

/// Text in the files is one byte per character (Latin-1).
pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}
