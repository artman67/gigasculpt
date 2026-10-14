//! The values after a PLY header, one element instance at a time: the words of a line in
//! ASCII, packed numbers in binary.

use std::io::Read;

use super::MAX_CORNERS;
use super::header::{Element, Kind, Ty};
use crate::ReadError;

/// Where values come from, and how an error names its place there.
pub(crate) trait Source {
    /// The next value, of type `ty`.
    fn next(&mut self, ty: Ty) -> Result<f64, ReadError>;
    /// An error at the place reading stands.
    fn bad(&self, reason: String) -> ReadError;
}

/// One ASCII line's words.
pub(crate) struct Words<'a, I: Iterator<Item = &'a [u8]>> {
    pub words: I,
    pub line: u64,
}

impl<'a, I: Iterator<Item = &'a [u8]>> Source for Words<'a, I> {
    fn next(&mut self, ty: Ty) -> Result<f64, ReadError> {
        let word = self.words.next();
        let word =
            word.ok_or_else(|| self.bad("fewer values than the element has properties".into()))?;
        let text = std::str::from_utf8(word).ok();
        let value = match ty {
            Ty::F32 => text.and_then(|t| t.parse::<f32>().ok()).map(f64::from),
            Ty::F64 => text.and_then(|t| t.parse::<f64>().ok()),
            _ => text.and_then(|t| t.parse::<i64>().ok()).map(|i| i as f64),
        };
        value.ok_or_else(|| self.bad("a value is not a number of its property's type".into()))
    }

    fn bad(&self, reason: String) -> ReadError {
        ReadError::BadLine {
            line: self.line,
            reason,
        }
    }
}

/// Binary values after the header.
pub(crate) struct Binary<R> {
    pub r: R,
    pub big_endian: bool,
    pub offset: u64,
}

impl<R: Read> Source for Binary<R> {
    fn next(&mut self, ty: Ty) -> Result<f64, ReadError> {
        let mut b = [0u8; 8];
        let n = ty.size();
        self.r.read_exact(&mut b[..n]).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => ReadError::Truncated {
                offset: self.offset as usize,
                what: "PLY data",
            },
            kind => ReadError::Io {
                offset: self.offset as usize,
                kind,
            },
        })?;
        self.offset += n as u64;
        if self.big_endian {
            b[..n].reverse();
        }
        let [b0, b1, b2, b3, ..] = b;
        Ok(match ty {
            Ty::I8 => f64::from(b0.cast_signed()),
            Ty::U8 => f64::from(b0),
            Ty::I16 => f64::from(i16::from_le_bytes([b0, b1])),
            Ty::U16 => f64::from(u16::from_le_bytes([b0, b1])),
            Ty::I32 => f64::from(i32::from_le_bytes([b0, b1, b2, b3])),
            Ty::U32 => f64::from(u32::from_le_bytes([b0, b1, b2, b3])),
            Ty::F32 => f64::from(f32::from_le_bytes([b0, b1, b2, b3])),
            Ty::F64 => f64::from_le_bytes(b),
        })
    }

    fn bad(&self, reason: String) -> ReadError {
        ReadError::BadValue {
            offset: self.offset,
            reason,
        }
    }
}

/// Reads one instance of `el` from `src`: each scalar (and each list's length) into `row`, in
/// property order, and the items of the list property at index `list_at` into `list`.
pub(crate) fn walk(
    el: &Element,
    row: &mut Vec<f64>,
    list_at: Option<usize>,
    list: &mut Vec<f64>,
    src: &mut impl Source,
) -> Result<(), ReadError> {
    row.clear();
    list.clear();
    for (i, p) in el.props.iter().enumerate() {
        match p.kind {
            Kind::Scalar(ty) => row.push(src.next(ty)?),
            Kind::List(count, item) => {
                let n = src.next(count)?;
                if !(0.0..=MAX_CORNERS).contains(&n) || n.fract() != 0.0 {
                    return Err(src.bad(format!("a list length of {n}")));
                }
                row.push(n);
                for _ in 0..n as usize {
                    let x = src.next(item)?;
                    if list_at == Some(i) {
                        list.push(x);
                    }
                }
            }
        }
    }
    Ok(())
}
