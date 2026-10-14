//! Pictures as 8-bit RGBA PNG files hold them: what `image.diff` compares and a transparent
//! `view.snapshot` mattes.

use gs_protocol::{CommandError, ErrorCode};

use crate::errors::{error, internal};

/// Tightly packed RGBA8 rows, top row first.
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// Reads an 8-bit RGBA PNG file.
pub(crate) fn read(file: &str) -> Result<Picture, CommandError> {
    let bytes = std::fs::read(file)
        .map_err(|e| error(ErrorCode::Precondition, format!("cannot read {file}: {e}")))?;
    decode(&bytes, file)
}

/// Decodes an 8-bit RGBA PNG; an error names it as `what`.
pub(crate) fn decode(png: &[u8], what: &str) -> Result<Picture, CommandError> {
    let unreadable = |e: &dyn std::fmt::Display| {
        error(ErrorCode::Precondition, format!("cannot read {what}: {e}"))
    };
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .map_err(|e| unreadable(&e))?;
    let mut rgba = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut rgba).map_err(|e| unreadable(&e))?;
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        let found = format!("{:?} {:?}, not 8-bit RGBA", info.bit_depth, info.color_type);
        return Err(unreadable(&found));
    }
    rgba.truncate(info.buffer_size());
    Ok(Picture {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// `picture` as an 8-bit RGBA PNG.
pub(crate) fn encode(picture: &Picture) -> Result<Vec<u8>, CommandError> {
    let failed = |e: png::EncodingError| internal(&format!("encoding a PNG: {e}"));
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, picture.width, picture.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(&picture.rgba).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}
