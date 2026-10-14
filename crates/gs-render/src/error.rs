//! The one error type of gs-render.

use std::fmt;

/// Everything that can go wrong while opening a device, uploading, rendering or baking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// No adapter of the requested kind is installed.
    NoAdapter(String),
    /// The adapter refused to open a device, or lacks a feature gs-render always needs.
    Device(String),
    /// The mesh is malformed: an index out of range, or an attribute slice of the wrong length.
    Mesh(String),
    /// The request needs a feature this adapter does not have.
    Unsupported(String),
    /// Reading the rendered image back from the GPU failed.
    Readback(String),
    /// Encoding a PNG failed.
    Png(String),
    /// A MatCap look recipe could not be read or is out of range.
    Recipe(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, detail) = match self {
            RenderError::NoAdapter(d) => ("no adapter", d),
            RenderError::Device(d) => ("device", d),
            RenderError::Mesh(d) => ("bad mesh", d),
            RenderError::Unsupported(d) => ("unsupported", d),
            RenderError::Readback(d) => ("readback", d),
            RenderError::Png(d) => ("png", d),
            RenderError::Recipe(d) => ("recipe", d),
        };
        write!(f, "{what}: {detail}")
    }
}

impl std::error::Error for RenderError {}
