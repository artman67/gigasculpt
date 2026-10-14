//! The picture: an immutable 16-bit grey image named by the hash of its pixels.

use std::fmt;

/// The largest width or height a picture may have.
pub const MOST: u32 = 8192;

/// A grey picture, 16 bits per pixel, row by row from the top. It never changes: every edit
/// makes a new one, and its name is the BLAKE3 hash of its canonical encoding (kind, width,
/// height, pixels), so the same pixels always have the same name.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    width: u32,
    height: u32,
    pixels: Vec<u16>,
    hash: String,
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (w, h) = (self.width, self.height);
        write!(f, "Picture({w} by {h}, {})", self.hash)
    }
}

impl Picture {
    /// A picture `width` by `height` (each 2 to 8192), or why it is not one.
    pub fn new(width: u32, height: u32, pixels: Vec<u16>) -> Result<Picture, String> {
        if !(2..=MOST).contains(&width) || !(2..=MOST).contains(&height) {
            return Err(format!(
                "a picture is 2 to {MOST} pixels each way, not {width} by {height}"
            ));
        }
        if pixels.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(format!(
                "a picture {width} by {height} has {} pixels, got {}",
                u64::from(width) * u64::from(height),
                pixels.len()
            ));
        }
        let hash = hash(width, height, &pixels);
        Ok(Picture {
            width,
            height,
            pixels,
            hash,
        })
    }

    /// A picture from values 0 to 1 (rounded to 16 bits; outside the range clamped, NaN 0).
    pub fn from_unit(width: u32, height: u32, values: &[f32]) -> Result<Picture, String> {
        Picture::new(width, height, values.iter().map(|&v| quantize(v)).collect())
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, row by row from the top.
    pub fn pixels(&self) -> &[u16] {
        &self.pixels
    }

    /// The pixel in column `x`, row `y`.
    pub fn at(&self, x: u32, y: u32) -> u16 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Its name: the BLAKE3 hash of kind, size and pixels, as 64 hex digits.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Bytes its pixels take.
    pub fn bytes(&self) -> usize {
        self.pixels.len() * 2
    }
}

/// A value 0 to 1 as 16 bits, rounded to the nearest step.
pub(crate) fn quantize(v: f32) -> u16 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    (v * 65535.0).round() as u16
}

/// The canonical encoding's hash: `gs-alpha`, the kind (0, height), width and height as
/// little-endian 32-bit numbers, then every pixel as little-endian 16 bits.
fn hash(width: u32, height: u32, pixels: &[u16]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"gs-alpha\0");
    h.update(&[0]);
    h.update(&width.to_le_bytes());
    h.update(&height.to_le_bytes());
    for chunk in pixels.chunks(4096) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|p| p.to_le_bytes()).collect();
        h.update(&bytes);
    }
    h.finalize().to_hex().to_string()
}
