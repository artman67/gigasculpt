//! Noise: a smooth value noise of a point's place, so it follows the points, not their order or
//! the threads. Each lattice corner's value comes from the command stream keyed by the corner's
//! three cell numbers and the seed; between corners it blends with the smoothstep.

use std::cmp::Ordering;

use crate::rng::CommandRng;

/// The noise change of frame coordinates `c` by up to `amount` (already weighed) along each
/// frame axis, in cells `scale` wide.
pub(super) fn noise(c: [f64; 3], amount: f64, scale: f32, seed: u64) -> [f64; 3] {
    let mut d = [0.0; 3];
    if amount == 0.0 || scale.partial_cmp(&0.0).is_none_or(Ordering::is_le) {
        return d;
    }
    let g = c.map(|x| x / f64::from(scale));
    let cell = g.map(libm::floor);
    let t = [0, 1, 2].map(|k| {
        let f = g[k] - cell[k];
        f * f * (3.0 - 2.0 * f)
    });
    let base = cell.map(|x| x as i64);
    for corner in 0..8 {
        let bit = |k: usize| (corner >> k) & 1;
        let at = [0, 1, 2].map(|k| base[k] + bit(k));
        let weight: f64 = (0..3)
            .map(|k| if bit(k) == 1 { t[k] } else { 1.0 - t[k] })
            .product();
        let h = (at.iter()).fold(seed, |h, &i| CommandRng::from_seed(h).u64_at(i as u64));
        for (k, d) in d.iter_mut().enumerate() {
            *d += weight * slice(h, k);
        }
    }
    d.map(|n| amount * n)
}

/// Bits `21 k` to `21 k + 20` of `h` as a value in `[-1, 1)`.
fn slice(h: u64, k: usize) -> f64 {
    let v = (h >> (21 * k)) & ((1 << 21) - 1);
    v as f64 / f64::from(1u32 << 20) - 1.0
}
