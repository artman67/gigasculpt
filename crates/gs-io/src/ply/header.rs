//! The PLY header: the format line, then each element with its count and properties, up to
//! `end_header`.

use std::io::BufRead;

use crate::ReadError;
use crate::mesh::{Lines, words};

/// How the data after the header is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Ascii,
    Binary { big_endian: bool },
}

/// A property's number type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    fn parse(word: &[u8]) -> Option<Self> {
        Some(match word {
            b"char" | b"int8" => Self::I8,
            b"uchar" | b"uint8" => Self::U8,
            b"short" | b"int16" => Self::I16,
            b"ushort" | b"uint16" => Self::U16,
            b"int" | b"int32" => Self::I32,
            b"uint" | b"uint32" => Self::U32,
            b"float" | b"float32" => Self::F32,
            b"double" | b"float64" => Self::F64,
            _ => return None,
        })
    }

    /// Bytes in binary data.
    pub fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// A scalar, or a list stored as a count and its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Scalar(Ty),
    List(Ty, Ty),
}

impl Kind {
    pub fn is_scalar(self) -> bool {
        matches!(self, Self::Scalar(_))
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::Scalar(Ty::F32 | Ty::F64))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Property {
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub(crate) struct Element {
    pub name: String,
    pub count: u64,
    pub props: Vec<Property>,
}

pub(crate) struct Header {
    pub format: Format,
    pub elements: Vec<Element>,
}

fn text(word: &[u8]) -> String {
    String::from_utf8_lossy(word).into_owned()
}

/// Reads the header, leaving `lines` right after `end_header`.
pub(crate) fn read<R: BufRead>(lines: &mut Lines<R>) -> Result<Header, ReadError> {
    let bad = |line, reason: &str| ReadError::BadLine {
        line,
        reason: reason.into(),
    };
    let mut format = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut ended = false;
    lines.each(|at, line| {
        if at == 1 {
            return if line == b"ply" {
                Ok(true)
            } else {
                Err(bad(1, "a PLY file starts with the line \"ply\""))
            };
        }
        let w: Vec<&[u8]> = words(line).collect();
        match w[..] {
            [b"end_header"] => {
                ended = true;
                return Ok(false);
            }
            [b"comment" | b"obj_info", ..] | [] => {}
            [b"format", kind, b"1.0"] => {
                format = Some(match kind {
                    b"ascii" => Format::Ascii,
                    b"binary_little_endian" => Format::Binary { big_endian: false },
                    b"binary_big_endian" => Format::Binary { big_endian: true },
                    _ => return Err(bad(at, "unknown PLY format")),
                });
            }
            [b"element", name, count] => {
                let count = std::str::from_utf8(count).ok().and_then(|c| c.parse().ok());
                let count = count.ok_or(bad(at, "an element count is not a whole number"))?;
                let (name, props) = (text(name), Vec::new());
                elements.push(Element { name, count, props });
            }
            [b"property", ty, name] | [b"property", b"list", _, ty, name] => {
                let item = Ty::parse(ty).ok_or(bad(at, "unknown property type"))?;
                let kind = match w[..] {
                    [_, _, count, _, _] => {
                        let count = Ty::parse(count).ok_or(bad(at, "unknown list count type"))?;
                        Kind::List(count, item)
                    }
                    _ => Kind::Scalar(item),
                };
                let el = (elements.last_mut()).ok_or(bad(at, "a property before any element"))?;
                el.props.push(Property {
                    name: text(name),
                    kind,
                });
            }
            _ => return Err(bad(at, "a header line PLY does not have")),
        }
        Ok(true)
    })?;
    if lines.line == 0 {
        return Err(bad(1, "a PLY file starts with the line \"ply\""));
    }
    if !ended {
        return Err(bad(lines.line + 1, "the header ends without end_header"));
    }
    let format = format.ok_or(bad(lines.line, "the header has no format line"))?;
    Ok(Header { format, elements })
}
