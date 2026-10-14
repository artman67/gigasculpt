//! Reading a text file line by line with bounded memory, counting lines for error messages.
//!
//! The loop lives here, in one place, and every turn of it consumes at least one byte or ends
//! at the end of the file, so no reader built on it can spin.

use std::io::BufRead;

use crate::ReadError;

/// The longest line read: a longer one is an error rather than a reason to hold the file.
const MAX_LINE: usize = 1 << 24;

/// The lines of a text file.
pub(crate) struct Lines<R> {
    src: R,
    buf: Vec<u8>,
    /// The number of the last line handed out, from 1.
    pub line: u64,
    /// Bytes consumed so far.
    pub offset: u64,
}

impl<R: BufRead> Lines<R> {
    pub fn new(src: R) -> Self {
        Self {
            src,
            buf: Vec::new(),
            line: 0,
            offset: 0,
        }
    }

    /// Calls `f` with each line's number (from 1) and the line without its end (`\n` or
    /// `\r\n`), until the file ends or `f` returns `false`.
    pub fn each(
        &mut self,
        mut f: impl FnMut(u64, &[u8]) -> Result<bool, ReadError>,
    ) -> Result<(), ReadError> {
        loop {
            let chunk = self.src.fill_buf().map_err(|e| ReadError::Io {
                offset: self.offset as usize,
                kind: e.kind(),
            })?;
            let ended = chunk.is_empty();
            let piece = match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => &chunk[..=i],
                None => chunk,
            };
            let complete = piece.last() == Some(&b'\n');
            let len = piece.len();
            self.buf.extend_from_slice(piece);
            self.src.consume(len);
            self.offset += len as u64;
            if self.buf.len() > MAX_LINE {
                return Err(ReadError::BadLine {
                    line: self.line + 1,
                    reason: "the line is longer than 16 MiB".into(),
                });
            }
            if complete || (ended && !self.buf.is_empty()) {
                self.line += 1;
                let text = self.buf.strip_suffix(b"\n").unwrap_or(&self.buf);
                let text = text.strip_suffix(b"\r").unwrap_or(text);
                let more = f(self.line, text)?;
                self.buf.clear();
                if !more {
                    return Ok(());
                }
            }
            if ended {
                return Ok(());
            }
        }
    }

    /// The source, positioned right after the last line handed out.
    pub fn into_inner(self) -> R {
        self.src
    }
}

/// The words of a line, split at ASCII white space.
pub(crate) fn words(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(u8::is_ascii_whitespace)
        .filter(|w| !w.is_empty())
}

/// A word as a finite `f32`, rounded once from its decimal text.
pub(crate) fn finite(word: &[u8]) -> Option<f32> {
    let x: f32 = std::str::from_utf8(word).ok()?.parse().ok()?;
    x.is_finite().then_some(x)
}
