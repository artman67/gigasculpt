//! Burley's normalized diffusion profile for subsurface scattering (Christensen and Burley
//! 2015, as Cycles' `bssrdf.h` and HDRP use it):
//!
//! `R(r) = (e^(-r/d) + e^(-r/(3d))) / (8 pi d r)`, which integrates to 1 over the plane,
//! with shape `d = l / s(A)` from the mean free path `l` and the surface albedo `A`, and
//! `s(A) = 1.9 - A + 3.5 (A - 0.8)^2` (their fit for diffuse surface transmission).
//!
//! This is the approximation it is: a semi-infinite flat medium's response, applied to a
//! curved surface by measuring distances along it.

use std::sync::OnceLock;

/// Samples of the profile's radius per color channel.
pub const SAMPLES: usize = 64;
/// The share of the profile's energy the samples reach; the far tail is left out and the
/// rest renormalized.
const REACH: f32 = 0.995;

/// The profile's shape `d` for mean free path `mfp` and surface albedo `albedo`.
pub fn shape(mfp: f32, albedo: f32) -> f32 {
    let a = albedo.clamp(0.0, 1.0);
    mfp / (1.9 - a + 3.5 * (a - 0.8) * (a - 0.8))
}

/// The share of the profile's energy within radius `x d`.
fn cdf(x: f32) -> f32 {
    1.0 - 0.25 * libm::expf(-x) - 0.75 * libm::expf(-x / 3.0)
}

/// The radius in units of `d` holding share `u` of the profile's energy: the inverse of its
/// CDF, by bisection.
fn holding(u: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 64.0f32);
    for _ in 0..40 {
        let mid = f32::midpoint(lo, hi);
        if cdf(mid) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f32::midpoint(lo, hi)
}

/// Radii in units of `d` that split the profile's energy (up to `REACH`) into `count` equal
/// parts, each at its part's middle, nearest first.
pub fn split(count: usize) -> Vec<f32> {
    (0..count)
        .map(|k| holding((k as f32 + 0.5) / count as f32 * REACH))
        .collect()
}

/// [`split`] into `SAMPLES` parts, as the bake samples the profile.
pub fn radii() -> &'static [f32; SAMPLES] {
    static RADII: OnceLock<[f32; SAMPLES]> = OnceLock::new();
    RADII.get_or_init(|| {
        let radii = split(SAMPLES);
        std::array::from_fn(|k| radii[k])
    })
}
