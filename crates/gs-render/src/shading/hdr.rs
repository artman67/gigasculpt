//! Equirectangular HDR images as environments: a reader for Radiance `.hdr` files (RGBE, flat
//! or run-length encoded, as Poly Haven and most HDRI sources publish them) and the mapping
//! from image to directions.
//!
//! Mapping, camera-locked like every MatCap: the image's center column looks toward the viewer
//! (+z, the light that falls on the visible side of the subject), the right quarter toward +x,
//! the top row straight up.

use super::Rgb;
use super::environment::{BRIGHTEST, CUBE_SIZE, Environment};
use super::irradiance::Sh9;
use crate::error::RenderError;
use crate::math::V3;
use std::f32::consts::PI;

/// A linear-light image, row by row from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Equirect {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
}

fn bad(what: &str) -> RenderError {
    RenderError::Recipe(format!("hdr: {what}"))
}

/// One RGBE pixel to linear light (Ward's encoding: a shared exponent biased by 128).
fn rgbe([r, g, b, e]: [u8; 4]) -> Rgb {
    if e == 0 {
        return [0.0; 3];
    }
    let f = libm::scalbnf(1.0, i32::from(e) - 136);
    [f32::from(r) * f, f32::from(g) * f, f32::from(b) * f]
}

/// One scanline of `width` pixels from `data`, flat or in the new run-length form; returns the
/// pixels and the bytes read.
fn scanline(data: &[u8], width: usize) -> Result<(Vec<[u8; 4]>, usize), RenderError> {
    let rle = data.len() >= 4 && data[0] == 2 && data[1] == 2 && data[2] & 0x80 == 0;
    if !rle || !(8..0x8000).contains(&width) {
        let bytes = data
            .get(..width * 4)
            .ok_or_else(|| bad("the image data ends early"))?;
        let pixels = bytes.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        return Ok((pixels, width * 4));
    }
    if usize::from(data[2]) << 8 | usize::from(data[3]) != width {
        return Err(bad("a run-length scanline has the wrong width"));
    }
    let mut pixels = vec![[0u8; 4]; width];
    let mut at = 4;
    for channel in 0..4usize {
        let mut x = 0;
        while x < width {
            let byte = |i: usize| {
                data.get(i)
                    .copied()
                    .ok_or_else(|| bad("the image data ends early"))
            };
            let count = usize::from(byte(at)?);
            let (run, n) = if count > 128 {
                (true, count - 128)
            } else {
                (false, count)
            };
            if n == 0 || x + n > width {
                return Err(bad("a run overflows its scanline"));
            }
            for (k, pixel) in pixels[x..x + n].iter_mut().enumerate() {
                pixel[channel] = byte(at + 1 + if run { 0 } else { k })?;
            }
            at += 1 + if run { 1 } else { n };
            x += n;
        }
    }
    Ok((pixels, at))
}

impl Equirect {
    /// Reads a Radiance `.hdr` file (`#?RADIANCE` or `#?RGBE`, 32-bit RGBE, `-Y h +X w`).
    pub fn read_hdr(bytes: &[u8]) -> Result<Equirect, RenderError> {
        let text_end = |from: usize| {
            bytes[from..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| from + p)
        };
        let mut at = 0;
        let mut format_ok = false;
        loop {
            let end = text_end(at).ok_or_else(|| bad("the header never ends"))?;
            let line = String::from_utf8_lossy(&bytes[at..end]);
            at = end + 1;
            if line.is_empty() {
                break;
            }
            format_ok |= line == "FORMAT=32-bit_rle_rgbe";
        }
        if !format_ok {
            return Err(bad("not a 32-bit RGBE file"));
        }
        let end = text_end(at).ok_or_else(|| bad("no resolution line"))?;
        let size = String::from_utf8_lossy(&bytes[at..end]).to_string();
        at = end + 1;
        let words: Vec<&str> = size.split_whitespace().collect();
        let (height, width) = match words.as_slice() {
            ["-Y", h, "+X", w] => (h.parse::<usize>().ok(), w.parse::<usize>().ok()),
            _ => (None, None),
        };
        let (Some(height), Some(width)) = (height, width) else {
            return Err(bad(&format!("resolution {size:?} is not -Y h +X w")));
        };
        if width == 0 || height == 0 || width.saturating_mul(height) > 1 << 28 {
            return Err(bad(&format!("size {width} by {height}")));
        }
        // A row takes at least 4 bytes per pixel flat, or in the run-length form its 4-byte
        // start and 2 bytes per run of up to 127 pixels in each of the 4 channels: a file too
        // short for the size it claims is refused before memory is reserved for it.
        let row = if (8..0x8000).contains(&width) {
            4 + 8 * width.div_ceil(127)
        } else {
            width * 4
        };
        if height.saturating_mul(row) > bytes.len() - at {
            return Err(bad(&format!(
                "the file is too short for an image of {width} by {height}"
            )));
        }
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let (row, used) = scanline(&bytes[at..], width)?;
            pixels.extend(row.into_iter().map(rgbe));
            at += used;
            let line = &pixels[y * width..];
            if let Some(x) = line.iter().position(|p| p.iter().any(|&c| c > BRIGHTEST)) {
                return Err(bad(&format!(
                    "pixel {x} of row {y} is {:e}, brighter than any light (at most 2^64)",
                    line[x].into_iter().fold(0.0, f32::max)
                )));
            }
        }
        Ok(Equirect {
            width,
            height,
            pixels,
        })
    }

    /// The image position (column, row, fractional) that direction `d` sees.
    fn position(&self, [x, y, z]: V3) -> (f32, f32) {
        let u = 0.5 + libm::atan2f(x, z) / (2.0 * PI);
        let v = libm::acosf(y.clamp(-1.0, 1.0)) / PI;
        (u * self.width as f32 - 0.5, v * self.height as f32 - 0.5)
    }

    /// Bilinear light along unit direction `d`, wrapping around horizontally.
    pub fn light(&self, d: V3) -> Rgb {
        let (x, y) = self.position(d);
        let y = y.clamp(0.0, (self.height - 1) as f32);
        let x = x.rem_euclid(self.width as f32);
        let (c0, y0) = ((x as usize).min(self.width - 1), y as usize);
        let c1 = (c0 + 1) % self.width;
        let (fx, fy) = (x - c0 as f32, y - y0 as f32);
        let y1 = (y0 + 1).min(self.height - 1);
        let p = |c: usize, r: usize| self.pixels[r * self.width + c];
        std::array::from_fn(|k| {
            let top = p(c0, y0)[k] + (p(c1, y0)[k] - p(c0, y0)[k]) * fx;
            let bottom = p(c0, y1)[k] + (p(c1, y1)[k] - p(c0, y1)[k]) * fx;
            top + (bottom - top) * fy
        })
    }

    /// The image as an environment turned `turn` radians about the vertical axis (positive
    /// turns the light toward the viewer's right), sampled with 4 by 4 directions per cube
    /// texel.
    pub fn environment(&self, name: &str, turn: f32) -> Environment {
        let cube = super::cube::Cube::build(CUBE_SIZE, 4, |d| self.light(about_y(d, -turn)));
        Environment::from_cube(name, cube)
    }

    /// The turn that puts the image's dominant light at the azimuth of the default studio's
    /// key light: the dominant light is the direction of the first spherical-harmonic band of
    /// its luminance.
    pub fn key_turn(&self) -> f32 {
        let coarse = super::cube::Cube::build(CUBE_SIZE / 2, 2, |d| self.light(d));
        let c = Sh9::project(&coarse).coefficients;
        let luminance = |rgb: Rgb| 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        // Band 1 holds y, z and x, in that order.
        let (x, z) = (luminance(c[3]), luminance(c[2]));
        let [kx, _, kz] = KEY_DIRECTION;
        libm::atan2f(kx, kz) - libm::atan2f(x, z)
    }
}

/// Toward the default studio's key light (`assets/environments/studio.toml`), whose azimuth
/// captured probes are turned to by default.
const KEY_DIRECTION: V3 = [-0.45, 0.55, 0.70];

/// `d` turned `angle` radians about +y: its azimuth (atan2 of x over z) grows by `angle`.
fn about_y([x, y, z]: V3, angle: f32) -> V3 {
    let (s, c) = (libm::sinf(angle), libm::cosf(angle));
    [x * c + z * s, y, z * c - x * s]
}
