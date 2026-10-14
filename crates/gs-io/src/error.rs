//! Why a file could not be read. Every error names where reading stopped: the byte offset in
//! a binary file, the line in a text file.

use std::fmt;

/// A file that could not be read, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The file does not start with the brush file signature.
    NotZbp,
    /// The file does not start with the tool or project file signature.
    NotZtl,
    /// The file ends before the named part is complete.
    Truncated { offset: usize, what: &'static str },
    /// An object's start marker, length or end marker is wrong.
    BadObject { offset: usize, reason: &'static str },
    /// The parts are there but not in the order or number the format has.
    BadLayout { offset: usize, reason: &'static str },
    /// An embedded image or array cannot be decoded.
    BadImage { offset: usize, reason: &'static str },
    /// A named setting record is malformed.
    BadSetting { offset: usize, reason: &'static str },
    /// A mesh's arrays do not fit together (counts, indices, levels).
    BadMesh { offset: usize, reason: &'static str },
    /// A text file (OBJ, ASCII PLY or STL, a PLY header) has a line that cannot be read.
    BadLine { line: u64, reason: String },
    /// A binary mesh file (PLY, STL) holds a value that cannot be used.
    BadValue { offset: u64, reason: String },
    /// The operating system could not read the file at this offset.
    Io {
        offset: usize,
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotZbp => write!(f, "not a brush file: the signature is missing"),
            Self::NotZtl => write!(f, "not a tool or project file: the signature is missing"),
            Self::Truncated { offset, what } => {
                write!(f, "file ends at byte {offset} in the middle of the {what}")
            }
            Self::BadObject { offset, reason } => {
                write!(f, "broken object at byte {offset}: {reason}")
            }
            Self::BadLayout { offset, reason } => {
                write!(f, "unexpected layout at byte {offset}: {reason}")
            }
            Self::BadImage { offset, reason } => write!(f, "bad image at byte {offset}: {reason}"),
            Self::BadSetting { offset, reason } => {
                write!(f, "bad setting at byte {offset}: {reason}")
            }
            Self::BadMesh { offset, reason } => write!(f, "bad mesh at byte {offset}: {reason}"),
            Self::BadLine { line, reason } => write!(f, "line {line}: {reason}"),
            Self::BadValue { offset, reason } => write!(f, "bad value at byte {offset}: {reason}"),
            Self::Io { offset, kind } => write!(f, "could not read byte {offset}: {kind}"),
        }
    }
}

impl std::error::Error for ReadError {}
