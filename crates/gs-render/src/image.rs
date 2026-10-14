//! An RGBA8 image read back from the GPU, and its PNG encoding.

use crate::error::RenderError;

/// Tightly packed RGBA8 rows, top row first, sRGB-encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// An image filled with one color.
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> Image {
        Image {
            width,
            height,
            rgba: color.repeat((width * height) as usize),
        }
    }

    /// The pixel at column `x`, row `y` (row 0 is the top).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Encodes the image as an 8-bit RGBA PNG tagged sRGB.
    pub fn png(&self) -> Result<Vec<u8>, RenderError> {
        encode_png(self.width, self.height, png::BitDepth::Eight, &self.rgba)
    }

    /// Decodes an 8-bit RGBA PNG, as [`Image::png`] writes them.
    pub fn from_png(bytes: &[u8]) -> Result<Image, RenderError> {
        let bad = |e: png::DecodingError| RenderError::Png(e.to_string());
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .map_err(bad)?;
        let mut rgba = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut rgba).map_err(bad)?;
        if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
            return Err(RenderError::Png(format!(
                "{:?} {:?}, not 8-bit RGBA",
                info.bit_depth, info.color_type
            )));
        }
        rgba.truncate(info.buffer_size());
        Ok(Image {
            width: info.width,
            height: info.height,
            rgba,
        })
    }

    /// Tiles images `columns` across (clamped to 1 and the number of images), left to right
    /// and top to bottom, each with its label burned into its top-left corner. Every tile is
    /// the size of the largest image; what an image does not cover is [`crate::BACKGROUND`].
    pub fn sheet(tiles: &[(&str, &Image)], columns: u32) -> Image {
        let columns = columns.clamp(1, (tiles.len() as u32).max(1));
        let rows = (tiles.len() as u32).div_ceil(columns);
        let width = tiles.iter().map(|t| t.1.width).max().unwrap_or(0);
        let height = tiles.iter().map(|t| t.1.height).max().unwrap_or(0);
        let mut sheet = Image::filled(width * columns, height * rows, crate::BACKGROUND);
        for (i, (label, image)) in tiles.iter().enumerate() {
            let mut tile = (*image).clone();
            // Burned into the tile before tiling, so a long label is clipped to its own tile.
            crate::font::label(&mut tile, 0, 0, label);
            let (x, y) = (i as u32 % columns, i as u32 / columns);
            sheet.blit(&tile, x * width, y * height);
        }
        sheet
    }

    /// Copies `src` with its top-left corner at (`x0`, `y0`), clipped to this image.
    pub(crate) fn blit(&mut self, src: &Image, x0: u32, y0: u32) {
        let w = src.width.min(self.width.saturating_sub(x0)) as usize;
        for y in 0..src.height.min(self.height.saturating_sub(y0)) {
            let from = (y * src.width * 4) as usize;
            let to = (((y0 + y) * self.width + x0) * 4) as usize;
            self.rgba[to..to + w * 4].copy_from_slice(&src.rgba[from..from + w * 4]);
        }
    }

    /// Fills a rectangle, clipped to the image.
    pub(crate) fn fill_rect(&mut self, x0: u32, y0: u32, w: u32, h: u32, color: [u8; 4]) {
        for y in y0..(y0 + h).min(self.height) {
            for x in x0..(x0 + w).min(self.width) {
                let i = ((y * self.width + x) * 4) as usize;
                self.rgba[i..i + 4].copy_from_slice(&color);
            }
        }
    }
}

/// RGBA PNG of 8- or 16-bit samples (16-bit samples big-endian, as PNG stores them).
pub(crate) fn encode_png(
    width: u32,
    height: u32,
    depth: png::BitDepth,
    data: &[u8],
) -> Result<Vec<u8>, RenderError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(depth);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder
        .write_header()
        .map_err(|e| RenderError::Png(e.to_string()))?;
    writer
        .write_image_data(data)
        .map_err(|e| RenderError::Png(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| RenderError::Png(e.to_string()))?;
    Ok(out)
}
