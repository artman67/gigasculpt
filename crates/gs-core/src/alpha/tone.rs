//! The tone pipeline: a picture through a brush's tone settings, in their fixed order (flips and
//! quarter turns, invert, maximize, blur, noise, contrast, curve, radial fade). A step at its
//! neutral value is skipped, so an untouched picture passes through bit for bit. Values stay 16
//! bits between steps; each pass works row by row in parallel and every row's result depends
//! only on its inputs, so the output is the same at any thread count.

use super::noise;
use super::picture::{Picture, quantize};
use crate::brush::{AlphaSpec, Flip};

/// A grey image being processed: width, height and 16-bit values row by row from the top.
pub(crate) struct Image {
    pub width: usize,
    pub height: usize,
    pub values: Vec<u16>,
}

impl Image {
    /// A new image `width` by `height` whose row `y` is filled by `f(y, row)`, rows in
    /// parallel.
    fn rows(width: usize, height: usize, f: impl Fn(usize, &mut [u16]) + Sync) -> Image {
        let mut values = vec![0u16; width * height];
        let mut rows: Vec<&mut [u16]> = values.chunks_mut(width).collect();
        gs_par::apply_each(&mut rows, |y, row| f(y, row));
        Image {
            width,
            height,
            values,
        }
    }

    /// A new image `width` by `height` whose pixel `(x, y)` is `f(x, y)`, rows in parallel.
    fn build(width: usize, height: usize, f: impl Fn(usize, usize) -> u16 + Sync) -> Image {
        Image::rows(width, height, |y, row| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = f(x, y);
            }
        })
    }

    fn at(&self, x: usize, y: usize) -> u16 {
        self.values[y * self.width + x]
    }

    /// Rows and columns swapped.
    fn transposed(&self) -> Image {
        Image::build(self.height, self.width, |x, y| self.at(y, x))
    }
}

/// `picture` through the tone settings of `spec`.
pub(crate) fn process(picture: &Picture, spec: &AlphaSpec) -> Image {
    let mut img = orient(picture, spec.flip, spec.turn, spec.invert);
    if spec.maximize {
        img = maximize(&img);
    }
    if spec.blur > 0.0 {
        // Sigma in pixels of a copy 256 pixels wide, the same both ways.
        img = blur(&img, spec.blur * img.width as f32 / 256.0);
    }
    if spec.noise > 0.0
        || spec.contrast.to_bits() != 1f32.to_bits()
        || !spec.curve.is_identity()
        || spec.radial_fade > 0.0
    {
        img = pointwise(&img, spec);
    }
    img
}

/// Flips, then quarter turns clockwise (a turned picture swaps width and height), then invert:
/// each pixel read from where it came from.
fn orient(p: &Picture, flip: Flip, turn: u8, invert: bool) -> Image {
    let (w, h) = (p.width() as usize, p.height() as usize);
    let (fh, fv) = match flip {
        Flip::None => (false, false),
        Flip::Horizontal => (true, false),
        Flip::Vertical => (false, true),
        Flip::Both => (true, true),
    };
    let flipped = |x: usize, y: usize| {
        let x = if fh { w - 1 - x } else { x };
        let y = if fv { h - 1 - y } else { y };
        p.at(x as u32, y as u32)
    };
    // One clockwise quarter turn sends (x, y) of a w by h image to (h - 1 - y, x).
    let turns = usize::from(turn % 4);
    let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    Image::build(ow, oh, |x, y| {
        let (sx, sy) = match turns {
            1 => (y, h - 1 - x),
            2 => (w - 1 - x, h - 1 - y),
            3 => (w - 1 - y, x),
            _ => (x, y),
        };
        let v = flipped(sx, sy);
        if invert { u16::MAX - v } else { v }
    })
}

/// The darkest value stretched to 0 and the lightest to the top, in integers (rounded).
fn maximize(img: &Image) -> Image {
    let lo = img.values.iter().copied().min().unwrap_or(0);
    let hi = img.values.iter().copied().max().unwrap_or(0);
    if hi == lo {
        return Image::build(img.width, img.height, |x, y| img.at(x, y));
    }
    let range = u64::from(hi - lo);
    Image::build(img.width, img.height, |x, y| {
        let v = u64::from(img.at(x, y) - lo);
        ((v * 65535 + range / 2) / range) as u16
    })
}

/// The widths of three box blurs (odd) whose combined variance is that of a Gaussian with
/// standard deviation `sigma` (W. Wells, "Efficient synthesis of Gaussian filters by cascaded
/// uniform filters", 1986; the widths as in Kovesi, "Fast almost-Gaussian filtering", 2010).
fn box_widths(sigma: f32) -> [usize; 3] {
    let s2 = f64::from(sigma) * f64::from(sigma);
    let ideal = (12.0 * s2 / 3.0 + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let m = ((12.0 * s2 - 3.0 * (wl * wl) as f64 - 12.0 * wl as f64 - 9.0)
        / (-4.0 * wl as f64 - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    std::array::from_fn(|i| if i < m { wl as usize } else { wl as usize + 2 })
}

/// A Gaussian blur of standard deviation `sigma` pixels, as three box blurs each way, on integer
/// sums (exact) with the edges held. Each box costs the same at any radius.
fn blur(img: &Image, sigma: f32) -> Image {
    let mut out = Image::build(img.width, img.height, |x, y| img.at(x, y));
    for width in box_widths(sigma) {
        let r = width / 2;
        if r > 0 {
            out = box_rows(&box_rows(&out, r).transposed(), r).transposed();
        }
    }
    out
}

/// Every row's box blur of radius `r`: each value the rounded mean of the `2r + 1` values
/// around it, with the row's end values held past its ends. From running sums.
fn box_rows(img: &Image, r: usize) -> Image {
    let n = img.width;
    let width = 2 * r as u64 + 1;
    Image::rows(n, img.height, |y, row| {
        let src = &img.values[y * n..(y + 1) * n];
        // prefix[k] sums the held row from index -r up to k - r - 1.
        let held = |k: usize| u64::from(src[k.saturating_sub(r).min(n - 1)]);
        let mut prefix = Vec::with_capacity(n + 2 * r + 1);
        prefix.push(0u64);
        for k in 0..n + 2 * r {
            let last = prefix[prefix.len() - 1];
            prefix.push(last + held(k));
        }
        for (i, v) in row.iter_mut().enumerate() {
            let sum = prefix[i + 2 * r + 1] - prefix[i];
            *v = ((sum + width / 2) / width) as u16;
        }
    })
}

/// Noise, contrast, curve and radial fade, pixel by pixel in that order.
fn pointwise(img: &Image, spec: &AlphaSpec) -> Image {
    let (w, h) = (img.width as f32, img.height as f32);
    Image::build(img.width, img.height, |x, y| {
        let mut v = f32::from(img.at(x, y)) / 65535.0;
        let (u, t) = ((x as f32 + 0.5) / w, (y as f32 + 0.5) / h);
        if spec.noise > 0.0 {
            let n = noise::value(u, t, spec.noise_scale, 1, spec.noise_seed);
            v = (v + spec.noise * (2.0 * n - 1.0)).clamp(0.0, 1.0);
        }
        if spec.contrast.to_bits() != 1f32.to_bits() {
            v = ((v - 0.5) * spec.contrast + 0.5).clamp(0.0, 1.0);
        }
        if !spec.curve.is_identity() {
            v = spec.curve.eval(v).clamp(0.0, 1.0);
        }
        if spec.radial_fade > 0.0 {
            let (dx, dy) = (2.0 * u - 1.0, 2.0 * t - 1.0);
            let d = (dx * dx + dy * dy).sqrt();
            v *= libm::powf((1.0 - d).max(0.0), spec.radial_fade);
        }
        quantize(v)
    })
}
