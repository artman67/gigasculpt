//! The pyramid a stroke samples: the processed picture and smaller copies of it, and the one
//! sampling function (decision 0018).

use super::picture::Picture;
use super::tone::{self, Image};
use crate::brush::AlphaSpec;

/// One level: width, height and 16-bit values row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Level {
    width: u32,
    height: u32,
    values: Vec<u16>,
}

/// A picture through a brush's tone settings, at every level: level 0 is the processed
/// picture, and each next level averages 2 by 2 texels of the one before (rows and columns
/// past the edge read the last one), down to 1 by 1. Pure: the same picture and settings
/// always give the same bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pyramid {
    levels: Vec<Level>,
    picture: String,
    tone: String,
}

impl Pyramid {
    /// `picture` through the tone settings of `spec`, at every level.
    pub fn build(picture: &Picture, spec: &AlphaSpec) -> Pyramid {
        let Image {
            width,
            height,
            values,
        } = tone::process(picture, spec);
        let mut levels = vec![Level {
            width: width as u32,
            height: height as u32,
            values,
        }];
        while let Some(last) = levels.last()
            && (last.width > 1 || last.height > 1)
        {
            levels.push(halve(last));
        }
        Pyramid {
            levels,
            picture: picture.hash().to_string(),
            tone: spec.tone_key(),
        }
    }

    /// How many levels it has.
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// The width and height of level `k`.
    pub fn size(&self, k: usize) -> [u32; 2] {
        let l = &self.levels[k.min(self.levels.len() - 1)];
        [l.width, l.height]
    }

    /// The value of texel `(x, y)` of level `k`.
    pub fn texel(&self, k: usize, x: u32, y: u32) -> u16 {
        let l = &self.levels[k.min(self.levels.len() - 1)];
        l.values[(y * l.width + x) as usize]
    }

    /// The hash of the picture it was made from.
    pub fn picture_hash(&self) -> &str {
        &self.picture
    }

    /// The tone settings it was made with ([`AlphaSpec::tone_key`]).
    pub fn tone_key(&self) -> &str {
        &self.tone
    }

    /// Bytes its levels take.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.values.len() * 2).sum()
    }

    /// The level a dab of radius `r` reads where vertices are about `spacing` apart: the finest
    /// whose texel spacing across the footprint (`2r / (tiles * size)`, the smaller of the two
    /// axes) is at least half the vertex spacing, or the coarsest.
    pub fn level_for(&self, r: f32, spacing: f32, tiles: [u32; 2]) -> usize {
        let fits = |l: &Level| {
            let texels = (tiles[0] * l.width).max(tiles[1] * l.height) as f32;
            2.0 * r / texels >= 0.5 * spacing
        };
        (self.levels.iter().position(fits)).unwrap_or(self.levels.len() - 1)
    }

    /// The value at `(u, v)` on level `k`, 0 to 1: `u` and `v` run -1 to 1 across the dab
    /// along its two axes, the picture's top row at `v = 1`; 0 outside. With `tiles` the picture
    /// repeats inside. Reads the 4 texels around the point (texel `i` centered at
    /// `(i + 0.5) / size`, held at the edges, wrapped across tiles) and blends them.
    #[inline]
    pub fn sample(&self, k: usize, u: f32, v: f32, tiles: [u32; 2]) -> f32 {
        if !(u.abs() <= 1.0 && v.abs() <= 1.0) {
            return 0.0;
        }
        let l = &self.levels[k.min(self.levels.len() - 1)];
        let (x0, x1, fx) = axis(f32::midpoint(u, 1.0), l.width, tiles[0]);
        let (y0, y1, fy) = axis(f32::midpoint(1.0, -v), l.height, tiles[1]);
        let at = |x: u32, y: u32| f32::from(l.values[(y * l.width + x) as usize]);
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        (top + (bottom - top) * fy) / 65535.0
    }
}

/// The two texels around position `s` (0 to 1 across the footprint) on an axis of `n` texels
/// repeated `tiles` times, and how far between them. Every dab vertex reads two of these, so
/// it stays in line and off the platform's `floorf`.
#[inline]
fn axis(s: f32, n: u32, tiles: u32) -> (u32, u32, f32) {
    let s = if tiles > 1 {
        let s = s * tiles as f32;
        s - floor(s) as f32
    } else {
        s
    };
    // `s` is in 0..=1 (0..1 when tiled), so `x0` is -1 to n - 1 and `x0 + 1` is 0 to n.
    let x = s * n as f32 - 0.5;
    let x0 = floor(x);
    let f = x - x0 as f32;
    let (x0, last) = (i64::from(x0), i64::from(n) - 1);
    if tiles > 1 {
        let wrap = |i: i64| match i {
            ..0 => last as u32,
            i if i > last => 0,
            i => i as u32,
        };
        (wrap(x0), wrap(x0 + 1), f)
    } else {
        let hold = |i: i64| i.clamp(0, last) as u32;
        (hold(x0), hold(x0 + 1), f)
    }
}

/// `x` rounded down, exactly, for `x` from -1 to 2^24 (a texel position).
#[inline]
fn floor(x: f32) -> i32 {
    let t = x as i32;
    if t as f32 > x { t - 1 } else { t }
}

/// The next level: each texel the rounded mean of 2 by 2, reading the last row or column again
/// past the edge.
fn halve(l: &Level) -> Level {
    let (w, h) = (l.width.div_ceil(2), l.height.div_ceil(2));
    let at = |x: u32, y: u32| {
        u32::from(l.values[(y.min(l.height - 1) * l.width + x.min(l.width - 1)) as usize])
    };
    let mut values = vec![0u16; (w * h) as usize];
    let mut rows: Vec<&mut [u16]> = values.chunks_mut(w as usize).collect();
    gs_par::apply_each(&mut rows, |y, row| {
        let y = y as u32;
        for (x, v) in row.iter_mut().enumerate() {
            let x = x as u32;
            let sum = at(2 * x, 2 * y)
                + at(2 * x + 1, 2 * y)
                + at(2 * x, 2 * y + 1)
                + at(2 * x + 1, 2 * y + 1);
            *v = ((sum + 2) / 4) as u16;
        }
    });
    Level {
        width: w,
        height: h,
        values,
    }
}

/// How the influence stage turns a value into a factor (decision 0018, choice 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    /// `v - mid`, negative below the mid value: the Offset kernel, where it carves.
    Signed,
    /// `|v - mid|`: a signed kernel's post effects, which read only the size.
    Size,
    /// `max(0, v - mid) / (1 - mid)`, 0 to 1: every other kernel.
    Intensity,
}

/// What a stroke reads of its alpha: the pyramid and how the brush uses it.
#[derive(Clone, Copy, Debug)]
pub struct AlphaRead<'a> {
    pub pyramid: &'a Pyramid,
    pub mid: f32,
    pub tiles: [u32; 2],
    pub sign: Sign,
}

impl AlphaRead<'_> {
    /// The factor at `(u, v)` on level `k`.
    #[inline]
    pub fn factor(&self, k: usize, u: f32, v: f32) -> f32 {
        let x = self.pyramid.sample(k, u, v, self.tiles);
        match self.sign {
            Sign::Signed => x - self.mid,
            Sign::Size => (x - self.mid).abs(),
            Sign::Intensity if self.mid >= 1.0 => 0.0,
            Sign::Intensity => (x - self.mid).max(0.0) / (1.0 - self.mid),
        }
    }
}
