//! The comparison rule of ARCHITECTURE 10.3, measured in `docs/spikes/golden.md`:
//!
//! - Same renderer (adapter, driver and wgpu version all match the reference's manifest): the
//!   image must be byte-identical.
//! - Different renderer, zebra images: pass if the worst FLIP pixel is at most 0.30.
//! - Different renderer, look images: pass if every channel of every pixel is within 2 sRGB
//!   levels; otherwise only if at most 50 pixels exceed FLIP 0.06 and the worst is at most 0.15.
//! - Never the FLIP mean: one gray level on the whole background moves it more than a real
//!   regression does.
//!
//! Bytes are compared first; FLIP runs only on a mismatch.

use std::cmp::Ordering;

use gs_render::Image;

/// Zebra images across renderers: the worst FLIP pixel allowed. GPU noise reached 0.103; a
/// 0.5% weaker stroke scored 0.67.
pub const ZEBRA_MAX: f32 = 0.30;
/// Look images across renderers pass outright within this many 8-bit levels per channel.
pub const LOOK_LEVELS: u8 = 2;
/// Otherwise at most [`LOOK_PIXELS`] pixels may exceed this FLIP value...
pub const LOOK_PIXEL: f32 = 0.06;
pub const LOOK_PIXELS: usize = 50;
/// ...and none may exceed this.
pub const LOOK_MAX: f32 = 0.15;

/// Which part of the rule applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Same renderer: byte equality.
    Bytes,
    /// Another renderer, zebra image.
    Zebra,
    /// Another renderer, any other look.
    Look,
}

/// The outcome of one comparison, with the numbers behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub rule: Rule,
    pub pass: bool,
    /// Pixels that differ at all (every pixel when the sizes differ).
    pub differing: usize,
    /// The largest difference of any channel, in 8-bit levels.
    pub max_level: u8,
    /// The worst FLIP value of any pixel; 0 when the bytes match.
    pub flip_max: f32,
    /// Pixels whose FLIP value exceeds [`LOOK_PIXEL`].
    pub flip_over: usize,
}

impl Verdict {
    /// One line for reports.
    pub fn summary(&self) -> String {
        let outcome = if self.pass { "passes" } else { "fails" };
        format!(
            "{outcome} the {:?} rule: {} pixels differ, by up to {} levels; FLIP worst {:.3}, {} pixels over {LOOK_PIXEL}",
            self.rule, self.differing, self.max_level, self.flip_max, self.flip_over
        )
    }
}

/// Pixels that differ, and the largest channel difference.
fn levels(a: &Image, b: &Image) -> (usize, u8) {
    let (mut differing, mut max) = (0, 0u8);
    for (p, q) in a.rgba.chunks(4).zip(b.rgba.chunks(4)) {
        let d = (0..4).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        differing += usize::from(d > 0);
        max = max.max(d);
    }
    (differing, max)
}

/// FLIP's per-pixel error between two images of one size. NaN counts as the worst error.
fn flip(expected: &Image, actual: &Image) -> Vec<f32> {
    let rgb = |i: &Image| {
        let data: Vec<u8> = i.rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        nv_flip::FlipImageRgb8::with_data(i.width, i.height, &data)
    };
    let errors = nv_flip::flip(
        rgb(expected),
        rgb(actual),
        nv_flip::DEFAULT_PIXELS_PER_DEGREE,
    );
    errors
        .to_vec()
        .into_iter()
        .map(|e| if e.is_nan() { 1.0 } else { e })
        .collect()
}

/// Compares `actual` against the reference `expected` by the rule for `same_renderer` and
/// `zebra` images.
pub fn compare(expected: &Image, actual: &Image, same_renderer: bool, zebra: bool) -> Verdict {
    let rule = match (same_renderer, zebra) {
        (true, _) => Rule::Bytes,
        (false, true) => Rule::Zebra,
        (false, false) => Rule::Look,
    };
    if (expected.width, expected.height) != (actual.width, actual.height) {
        let pixels = (actual.width * actual.height).max(expected.width * expected.height);
        return Verdict {
            rule,
            pass: false,
            differing: pixels as usize,
            max_level: u8::MAX,
            flip_max: 1.0,
            flip_over: pixels as usize,
        };
    }
    let (differing, max_level) = levels(expected, actual);
    let (flip_max, flip_over) = if differing == 0 {
        (0.0, 0)
    } else {
        let errors = flip(expected, actual);
        let worst = errors.iter().copied().fold(0.0, f32::max);
        let over = |e: &&f32| matches!((**e).partial_cmp(&LOOK_PIXEL), Some(Ordering::Greater));
        (worst, errors.iter().filter(over).count())
    };
    let pass = differing == 0
        || match rule {
            Rule::Bytes => false,
            Rule::Zebra => flip_max <= ZEBRA_MAX,
            Rule::Look => {
                max_level <= LOOK_LEVELS || (flip_over <= LOOK_PIXELS && flip_max <= LOOK_MAX)
            }
        };
    Verdict {
        rule,
        pass,
        differing,
        max_level,
        flip_max,
        flip_over,
    }
}

/// A picture of where two images differ: FLIP's error per pixel in the magma colors (black
/// for none, through purple and orange, to pale yellow for the worst). Images of different
/// sizes give a solid red picture the size of `actual`.
pub fn diff_map(expected: &Image, actual: &Image) -> Image {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return Image::filled(actual.width, actual.height, [255, 0, 0, 255]);
    }
    let errors = flip(expected, actual);
    let map = nv_flip::FlipImageFloat::with_data(actual.width, actual.height, &errors)
        .apply_color_lut(&nv_flip::magma_lut());
    Image {
        width: actual.width,
        height: actual.height,
        rgba: map
            .to_vec()
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
    }
}
