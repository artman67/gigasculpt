//! Tileable value noise for alphas: random values on an integer lattice that wraps, blended
//! smoothly in between, summed over octaves. Every lattice value comes from the counter-based
//! generator (`crate::rng`), so the same seed always gives the same noise.

use crate::rng::CommandRng;

/// The noise at `(x, y)`, each 0 to 1 across the picture (wrapping), with `cells` lattice
/// cells across at the first octave, each further octave twice as fine at half the weight. The
/// value is 0 to 1.
pub(crate) fn value(x: f32, y: f32, cells: u32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut weight, mut total) = (0.0f32, 1.0f32, 0.0f32);
    for octave in 0..octaves {
        let n = cells.saturating_mul(1 << octave.min(16));
        let stream = CommandRng::from_seed(u64::from(seed) << 32 | u64::from(octave));
        sum += weight * lattice(stream, x, y, n);
        total += weight;
        weight *= 0.5;
    }
    if total > 0.0 { sum / total } else { 0.0 }
}

/// One octave: the lattice of `n` by `n` cells blended at `(x, y)`.
fn lattice(stream: CommandRng, x: f32, y: f32, n: u32) -> f32 {
    let at = |i: u32, j: u32| {
        let k = u64::from(j % n) * u64::from(n) + u64::from(i % n);
        (stream.u64_at(k) >> 40) as f32 / 16_777_216.0
    };
    let (fx, fy) = (x.rem_euclid(1.0) * n as f32, y.rem_euclid(1.0) * n as f32);
    let (i, j) = (fx.floor(), fy.floor());
    let (tx, ty) = (fade(fx - i), fade(fy - j));
    let (i, j) = (i as u32 % n, j as u32 % n);
    let top = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
    let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
    top + (bottom - top) * ty
}

/// The quintic blend `6t^5 - 15t^4 + 10t^3`: flat at both ends, so cells join smoothly.
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
