//! Diffuse light from an environment: irradiance by direct integration over the cube (what the
//! MatCap bake reads), and its nine-coefficient spherical-harmonic fit (Ramamoorthi and
//! Hanrahan 2001; what a live lit mode evaluates per pixel).

use super::Rgb;
use super::cube::{Cube, Face, direction};
use crate::math::{V3, dot};
use std::f32::consts::PI;

/// Texels per face of the irradiance map: irradiance changes slowly, and bilinear filtering
/// between 5.6 degree steps stays well inside a percent.
pub const IRRADIANCE_SIZE: usize = 16;
/// Texels per face of the radiance level the integrals read.
const SOURCE_SIZE: usize = 32;

/// The coarsest mip level of `cube` whose faces are still at least `SOURCE_SIZE` texels wide.
fn source(cube: &Cube) -> usize {
    let mut lod = 0;
    while lod + 1 < cube.levels.len() && cube.levels[lod + 1][0].size >= SOURCE_SIZE {
        lod += 1;
    }
    lod
}

/// Irradiance `E(n) = integral of L(w) max(0, n.w) dw` for every texel normal of an
/// `IRRADIANCE_SIZE` cube, summed over the radiance texels.
pub fn irradiance_map(cube: &Cube) -> Cube {
    let texels = cube.texels(source(cube));
    let size = IRRADIANCE_SIZE;
    let level = std::array::from_fn(|face| {
        let values = gs_par::map(size * size, |t| {
            let (i, j) = (t % size, t / size);
            let n = direction(
                face,
                (i as f32 + 0.5) / size as f32,
                (j as f32 + 0.5) / size as f32,
            );
            irradiance(&texels, n)
        });
        Face {
            size,
            texels: values,
        }
    });
    Cube {
        levels: vec![level],
    }
}

/// Irradiance at normal `n` from a list of (direction, solid angle, radiance) texels.
pub fn irradiance(texels: &[(V3, f32, Rgb)], n: V3) -> Rgb {
    texels.iter().fold([0.0; 3], |sum, (w, omega, l)| {
        let k = dot(n, *w).max(0.0) * omega;
        [sum[0] + l[0] * k, sum[1] + l[1] * k, sum[2] + l[2] * k]
    })
}

/// The constant factor of each of the nine real spherical harmonics up to band 2, which
/// [`basis`] multiplies into the polynomials `1, y, z, x, xy, yz, 3z^2 - 1, xz, x^2 - y^2`.
const BASIS: [f32; 9] = [
    0.282_095, 0.488_603, 0.488_603, 0.488_603, 1.092_548, 1.092_548, 0.315_392, 1.092_548,
    0.546_274,
];

/// The nine real spherical harmonics up to band 2 at unit direction `d`.
fn basis([x, y, z]: V3) -> [f32; 9] {
    [
        BASIS[0],
        BASIS[1] * y,
        BASIS[2] * z,
        BASIS[3] * x,
        BASIS[4] * x * y,
        BASIS[5] * y * z,
        BASIS[6] * (3.0 * z * z - 1.0),
        BASIS[7] * x * z,
        BASIS[8] * (x * x - y * y),
    ]
}

/// The cosine lobe's band weights: convolving radiance with `max(0, cos)` scales band 0 by
/// pi, band 1 by 2 pi / 3 and band 2 by pi / 4.
const BAND: [f32; 9] = [
    PI,
    2.0 * PI / 3.0,
    2.0 * PI / 3.0,
    2.0 * PI / 3.0,
    PI / 4.0,
    PI / 4.0,
    PI / 4.0,
    PI / 4.0,
    PI / 4.0,
];

/// An environment's radiance projected on nine spherical harmonics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sh9 {
    pub coefficients: [Rgb; 9],
}

impl Sh9 {
    /// The projection of `cube`'s radiance.
    pub fn project(cube: &Cube) -> Sh9 {
        let mut coefficients = [[0.0f32; 3]; 9];
        for (d, omega, l) in cube.texels(source(cube)) {
            for (c, y) in coefficients.iter_mut().zip(basis(d)) {
                for k in 0..3 {
                    c[k] += l[k] * y * omega;
                }
            }
        }
        Sh9 { coefficients }
    }

    /// Irradiance at unit normal `n` from the fit.
    pub fn irradiance(&self, n: V3) -> Rgb {
        let y = basis(n);
        let mut e = [0.0f32; 3];
        for ((c, yi), band) in self.coefficients.iter().zip(y).zip(BAND) {
            for k in 0..3 {
                e[k] += c[k] * yi * band;
            }
        }
        e.map(|v| v.max(0.0))
    }

    /// The fit as nine polynomial coefficients `k`: the irradiance at `n = (x, y, z)` is
    /// `max(0, k0 + k1 y + k2 z + k3 x + k4 xy + k5 yz + k6 (3z^2 - 1) + k7 xz + k8 (x^2 - y^2))`,
    /// as Lit mode's shader evaluates it.
    pub fn polynomial(&self) -> [Rgb; 9] {
        std::array::from_fn(|i| self.coefficients[i].map(|c| c * BASIS[i] * BAND[i]))
    }
}
