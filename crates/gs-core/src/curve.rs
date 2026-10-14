//! Curves over `0..=1`: pen pressure curves, custom falloff profiles, an alpha's tone curve and
//! an alpha recipe's radial profile (decision 0018).
//!
//! Each point is sharp or smooth. Between two points the curve is a cubic Hermite segment whose
//! end tangents come from the points: a sharp point's tangent on each side is the slope of the
//! segment on that side, so two sharp ends give a straight line; a smooth point's tangent is
//! the harmonic mean of the slopes on its two sides (Fritsch and Carlson's monotone choice), or
//! 0 where the curve turns (a local peak or dip, or a flat side). Harmonic-mean tangents are at
//! most twice either slope, inside the region where a Hermite cubic stays monotone, so a
//! smooth stretch never overshoots its points and a falloff never leaves 0 to 1. The first and
//! last points always take the slope of their one segment.
//!
//! A curve with a smooth point is evaluated through a table of 257 values over `0..=1`, built
//! once, with straight lines between them (as Blender and Krita do). A curve whose points are
//! all sharp is evaluated exactly as straight segments, without the table. An exact copy
//! (`Curve::exact`, for Accurate Curve, decision 0059) works its smooth stretches out at each read.
//!
//! A curve never leaves the range of its points' values. What range those may take belongs to
//! the curve's use, not to the curve (review 0004, M3): every use today reads its curve as a
//! fraction, 0 to 1 (a falloff, a tone curve, a recipe's profile, a pressure curve as a
//! multiplier of its slider), checked where the curve is read with the setting's path in the
//! refusal (`brush/load.rs`, the recipe check).
//!
//! A curve may also carry a `Shape` (decision 0048, section 1): Tile repeats the drawn curve
//! `tile` times across the input, and Steps hold it to `steps` plateaus, which Step strength
//! blends back toward the smooth curve. Both are worked out at each read from the drawn curve,
//! not through its table, so a plateau's edge is exact.

use serde::{Deserialize, Serialize};

/// Entries in a smooth curve's table, over `0..=1`.
const TABLE: usize = 257;

/// The most copies Tile makes and the most plateaus Steps make.
const MOST: u32 = 64;

/// What a curve does with its drawn points at each read (decision 0048, section 1). The default
/// changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// Plateaus, 1 (off) to 64. With `S` of them, the input `x` reads the drawn curve at
    /// `k / (S - 1)`, where `k = floor(S x)` held to `S - 1`, so the first plateau is the
    /// curve's start and the last its end.
    pub steps: u32,
    /// How far the plateaus pull the curve, 0 (not at all) to 1 (fully).
    pub step_strength: f32,
    /// Copies of the drawn curve across the input, 1 (off) to 64; `x = 1` is the end of the
    /// last copy.
    pub tile: u32,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape {
            steps: 1,
            step_strength: 1.0,
            tile: 1,
        }
    }
}

impl Shape {
    fn is_default(&self) -> bool {
        *self == Shape::default()
    }

    /// Why this shape is not one a curve can carry, if it is not.
    fn check(&self) -> Result<(), String> {
        if !(1..=MOST).contains(&self.steps) {
            return Err(format!("steps must be 1 to {MOST}, not {}", self.steps));
        }
        if !(1..=MOST).contains(&self.tile) {
            return Err(format!("tile must be 1 to {MOST}, not {}", self.tile));
        }
        if !(0.0..=1.0).contains(&self.step_strength) {
            return Err(format!(
                "step_strength must be 0 to 1, not {}",
                self.step_strength
            ));
        }
        Ok(())
    }
}

/// A curve through points `[x, y]`. The first point has `x = 0`, the last `x = 1`, and `x`
/// rises strictly in between, so every input has exactly one value.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "CurveFile", into = "CurveFile")]
pub struct Curve {
    pts: Vec<[f32; 2]>,
    /// Which points are smooth; empty when every point is sharp.
    smooth: Vec<bool>,
    /// The values at `k / 256`, when a point is smooth and the curve is not exact.
    table: Vec<f32>,
    /// Steps and Tile.
    shape: Shape,
}

impl PartialEq for Curve {
    /// Two curves are equal when their points, kinds and shapes are (the table follows from
    /// them).
    fn eq(&self, other: &Curve) -> bool {
        self.pts == other.pts && self.smooth == other.smooth && self.shape == other.shape
    }
}

/// A curve as preset and recipe files write it: a list of points (all sharp), or a table with
/// the points, `smooth`, the indices of the points that stay sharp and the shape's settings
/// that are not their defaults. A curve with a shape is always written as a table.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum CurveFile {
    Sharp(Vec<[f32; 2]>),
    Smooth(SmoothFile),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SmoothFile {
    points: Vec<[f32; 2]>,
    smooth: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sharp: Vec<usize>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    steps: u32,
    #[serde(default = "full", skip_serializing_if = "is_full")]
    step_strength: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    tile: u32,
}

fn one() -> u32 {
    1
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_one(n: &u32) -> bool {
    *n == 1
}

fn full() -> f32 {
    1.0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_full(s: &f32) -> bool {
    s.to_bits() == 1f32.to_bits()
}

impl TryFrom<CurveFile> for Curve {
    type Error = String;
    fn try_from(f: CurveFile) -> Result<Curve, String> {
        match f {
            CurveFile::Sharp(pts) => Curve::new(pts),
            CurveFile::Smooth(s) => {
                if let Some(&i) = s.sharp.iter().find(|&&i| i >= s.points.len()) {
                    return Err(format!(
                        "sharp point {i} does not exist: the curve has {} points",
                        s.points.len()
                    ));
                }
                let kinds = (0..s.points.len())
                    .map(|i| s.smooth && !s.sharp.contains(&i))
                    .collect();
                let shape = Shape {
                    steps: s.steps,
                    step_strength: s.step_strength,
                    tile: s.tile,
                };
                Curve::with_kinds(s.points, kinds)?.shaped(shape)
            }
        }
    }
}

impl From<Curve> for CurveFile {
    fn from(c: Curve) -> CurveFile {
        if c.smooth.is_empty() && c.shape.is_default() {
            return CurveFile::Sharp(c.pts);
        }
        let sharp = (c.smooth.iter().enumerate())
            .filter(|e| !*e.1)
            .map(|e| e.0)
            .collect();
        CurveFile::Smooth(SmoothFile {
            points: c.pts,
            smooth: !c.smooth.is_empty(),
            sharp,
            steps: c.shape.steps,
            step_strength: c.shape.step_strength,
            tile: c.shape.tile,
        })
    }
}

impl Curve {
    /// A curve of straight segments through `pts`, or why it is not one.
    pub fn new(pts: Vec<[f32; 2]>) -> Result<Curve, String> {
        let n = pts.len();
        Curve::with_kinds(pts, vec![false; n])
    }

    /// A curve through `pts`, smooth at the points `smooth` marks, or why it is not one.
    pub fn with_kinds(pts: Vec<[f32; 2]>, smooth: Vec<bool>) -> Result<Curve, String> {
        let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
            return Err("a curve needs at least two points".into());
        };
        if pts.len() < 2 || first[0] != 0.0 || last[0].to_bits() != 1f32.to_bits() {
            return Err("a curve runs from x = 0 to x = 1 in at least two points".into());
        }
        if pts.windows(2).any(|w| w[1][0] <= w[0][0]) {
            return Err("curve points must have rising x".into());
        }
        if pts.iter().any(|p| !p[1].is_finite()) {
            return Err("curve values must be finite numbers".into());
        }
        if smooth.len() != pts.len() {
            return Err("a curve needs one kind per point".into());
        }
        Ok(Curve::built(pts, smooth))
    }

    /// The curve through checked points `pts`, with its table when a point is smooth.
    fn built(pts: Vec<[f32; 2]>, smooth: Vec<bool>) -> Curve {
        let smooth = if smooth.iter().any(|&s| s) {
            smooth
        } else {
            Vec::new()
        };
        let mut c = Curve {
            pts,
            smooth,
            table: Vec::new(),
            shape: Shape::default(),
        };
        if !c.smooth.is_empty() {
            c.table = (0..TABLE)
                .map(|k| c.cubic(k as f64 / (TABLE - 1) as f64) as f32)
                .collect();
        }
        c
    }

    /// The first point whose value is outside `0..=1`, if any.
    pub fn outside_fraction(&self) -> Option<[f32; 2]> {
        (self.pts.iter().copied()).find(|p| !(0.0..=1.0).contains(&p[1]))
    }

    /// This curve with every point's value held to `0..=1`, so its values stay there too.
    #[must_use]
    pub fn clamped(&self) -> Curve {
        if self.outside_fraction().is_none() {
            return self.clone();
        }
        self.mapped(|y| y.clamp(0.0, 1.0))
    }

    /// This curve with `f` applied to every point's value, its kinds and shape kept.
    fn mapped(&self, f: impl Fn(f32) -> f32) -> Curve {
        let pts = (self.pts.iter()).map(|&[x, y]| [x, f(y)]);
        let smooth = (0..self.pts.len()).map(|i| self.is_smooth(i));
        let mut c = Curve::built(pts.collect(), smooth.collect());
        c.shape = self.shape;
        c
    }

    /// This curve carrying `shape`, or why it cannot (decision 0048, section 1).
    pub fn shaped(mut self, shape: Shape) -> Result<Curve, String> {
        shape.check()?;
        self.shape = shape;
        Ok(self)
    }

    /// The curve's Steps and Tile.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// This curve read exactly, without its table: each read of a smooth stretch works out its
    /// cubic (Accurate Curve, decision 0059). Equal to the curve; slower to read.
    #[must_use]
    pub fn exact(&self) -> Curve {
        Curve {
            table: Vec::new(),
            ..self.clone()
        }
    }

    /// This curve turned over: every point's value `y` becomes `1 - y`, at the same input, with
    /// its kind; the shape stays (decision 0048, section 2).
    #[must_use]
    pub fn inverted(&self) -> Curve {
        self.mapped(|y| 1.0 - y)
    }

    /// This curve felt through a sensitivity `k` from -1 to 1 (decision 0036, section 3 as
    /// amended): every value `y` becomes `1 - |k| (1 - y)`, so 1 is the curve itself, 0 is full
    /// everywhere and a curve at full stays there bit for bit; below 0 the curve also runs
    /// backward, so a light touch gives what a full one did. Its kinds and shape stay.
    #[must_use]
    pub fn with_sensitivity(&self, k: f32) -> Curve {
        if k.to_bits() == 1f32.to_bits() {
            return self.clone();
        }
        let a = k.abs();
        let feel = |y: f32| 1.0 - a * (1.0 - y);
        if k >= 0.0 {
            return self.mapped(feel);
        }
        let mut pts: Vec<[f32; 2]> = Vec::with_capacity(self.pts.len());
        let mut smooth = Vec::with_capacity(self.pts.len());
        for (i, &[x, y]) in self.pts.iter().enumerate().rev() {
            let x = 1.0 - x;
            // Two inputs this close to 1 may round to one; the later is dropped.
            if pts.last().is_some_and(|p| x <= p[0]) {
                continue;
            }
            pts.push([x, feel(y)]);
            smooth.push(self.is_smooth(i));
        }
        let mut c = Curve::built(pts, smooth);
        c.shape = self.shape;
        c
    }

    /// The same value everywhere.
    pub fn constant(y: f32) -> Curve {
        Curve::built(vec![[0.0, y], [1.0, y]], vec![false; 2])
    }

    /// `y = x`.
    pub fn linear() -> Curve {
        Curve::built(vec![[0.0, 0.0], [1.0, 1.0]], vec![false; 2])
    }

    /// The points, in order.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.pts
    }

    /// Whether point `i` is smooth.
    pub fn is_smooth(&self, i: usize) -> bool {
        self.smooth.get(i).copied().unwrap_or(false)
    }

    /// True for `y = x` written as two sharp points: a curve that changes nothing.
    pub fn is_identity(&self) -> bool {
        *self == Curve::linear()
    }

    /// The value at `x`, clamped into `0..=1` (NaN reads as 0).
    pub fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let x = if x.is_nan() { 0.0 } else { x };
        let Shape {
            steps,
            step_strength: s,
            ..
        } = self.shape;
        let here = self.tiled(x);
        if steps < 2 || s <= 0.0 {
            return here;
        }
        let k = ((steps as f32 * x) as u32).min(steps - 1);
        let plateau = self.tiled(k as f32 / (steps - 1) as f32);
        if s >= 1.0 {
            return plateau;
        }
        here + s * (plateau - here)
    }

    /// The tiled curve at `x`, in `0..=1`: copy `i` of `tile` runs the drawn curve over its
    /// share of the input, and the last copy ends at 1.
    fn tiled(&self, x: f32) -> f32 {
        let n = self.shape.tile;
        if n < 2 {
            return self.drawn(x);
        }
        let t = n as f32 * x;
        let i = t.floor().min((n - 1) as f32);
        self.drawn((t - i).clamp(0.0, 1.0))
    }

    /// The drawn curve at `x`, in `0..=1`.
    fn drawn(&self, x: f32) -> f32 {
        if self.smooth.is_empty() {
            return self.straight(x);
        }
        if self.table.is_empty() {
            return self.cubic(f64::from(x)) as f32;
        }
        let s = x * (TABLE - 1) as f32;
        let i = (s as usize).min(TABLE - 2);
        let (a, b) = (self.table[i], self.table[i + 1]);
        a + (b - a) * (s - i as f32)
    }

    /// The straight segments through the points at `x`, in `0..=1`.
    fn straight(&self, x: f32) -> f32 {
        let i = self
            .pts
            .partition_point(|p| p[0] <= x)
            .clamp(1, self.pts.len() - 1);
        let ([x0, y0], [x1, y1]) = (self.pts[i - 1], self.pts[i]);
        y0 + (y1 - y0) * ((x - x0) / (x1 - x0))
    }

    /// The slope of segment `k`.
    fn slope(&self, k: usize) -> f64 {
        let ([x0, y0], [x1, y1]) = (self.pts[k], self.pts[k + 1]);
        (f64::from(y1) - f64::from(y0)) / (f64::from(x1) - f64::from(x0))
    }

    /// The tangent at point `i` as segment `k` (one of the segments beside it) sees it.
    fn tangent(&self, i: usize, k: usize) -> f64 {
        let last = self.pts.len() - 1;
        if !self.is_smooth(i) || i == 0 || i == last {
            return self.slope(k);
        }
        let (a, b) = (self.slope(i - 1), self.slope(i));
        if a * b <= 0.0 {
            0.0
        } else {
            2.0 * a * b / (a + b)
        }
    }

    /// The Hermite curve at `x`, in `0..=1`, in `f64`.
    fn cubic(&self, x: f64) -> f64 {
        let n = self.pts.len();
        let k = (self.pts[1..n - 1].iter())
            .take_while(|p| f64::from(p[0]) <= x)
            .count();
        let ([x0, y0], [x1, y1]) = (self.pts[k], self.pts[k + 1]);
        let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
        let h = x1 - x0;
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let (m0, m1) = (self.tangent(k, k) * h, self.tangent(k + 1, k) * h);
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * y0
            + (t3 - 2.0 * t2 + t) * m0
            + (3.0 * t2 - 2.0 * t3) * y1
            + (t3 - t2) * m1
    }
}
