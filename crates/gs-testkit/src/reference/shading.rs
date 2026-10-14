//! Slow f64 references for gs-render's material model (research 11, section 10): the GGX
//! lobe written from Heitz's Lambda form rather than the visibility shortcut, and plain
//! numerical integration over the sphere. Nothing here calls gs-render's shading code.

use std::f64::consts::PI;

pub use crate::measure::dot;
use crate::measure::{cross, norm};

pub type V = [f64; 3];
pub type Rgb = [f64; 3];

/// `a` scaled to unit length.
pub fn unit(a: V) -> V {
    let l = norm(a);
    a.map(|c| c / l)
}

/// Any two unit vectors completing `n` to an orthonormal frame.
fn frame(n: V) -> (V, V) {
    let helper = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t = unit(cross(helper, n));
    (t, cross(n, t))
}

fn local(n: V, theta: f64, phi: f64) -> V {
    let (t, b) = frame(n);
    let (st, ct) = (libm::sin(theta), libm::cos(theta));
    let (sp, cp) = (libm::sin(phi), libm::cos(phi));
    std::array::from_fn(|k| t[k] * st * cp + b[k] * st * sp + n[k] * ct)
}

/// GGX normal distribution.
pub fn ggx(n_h: f64, alpha: f64) -> f64 {
    if n_h <= 0.0 {
        return 0.0;
    }
    let c2 = n_h * n_h;
    let tan2 = (1.0 - c2) / c2;
    let a2 = alpha * alpha;
    a2 / (PI * c2 * c2 * (a2 + tan2) * (a2 + tan2))
}

/// Smith's Lambda for GGX.
fn lambda(c: f64, alpha: f64) -> f64 {
    let tan2 = (1.0 - c * c).max(0.0) / (c * c);
    ((1.0 + alpha * alpha * tan2).sqrt() - 1.0) / 2.0
}

/// Height-correlated masking-shadowing `G2 = 1 / (1 + Lambda(v) + Lambda(l))`.
pub fn g2(n_v: f64, n_l: f64, alpha: f64) -> f64 {
    1.0 / (1.0 + lambda(n_v, alpha) + lambda(n_l, alpha))
}

/// Schlick through `f0` and `f90`, minus the F82 dip set by `tint` at cos 1/7.
pub fn fresnel(f0: Rgb, f90: Rgb, tint: Rgb, c: f64) -> Rgb {
    let s = |c: f64| libm::pow(1.0 - c, 5.0);
    let dip = |c: f64| c * libm::pow(1.0 - c, 6.0);
    let mu = 1.0 / 7.0;
    std::array::from_fn(|k| {
        let at = f0[k] + (f90[k] - f0[k]) * s(mu);
        let b = at * (1.0 - tint[k]) / dip(mu);
        (f0[k] + (f90[k] - f0[k]) * s(c) - b * dip(c)).clamp(0.0, 1.0)
    })
}

/// The GGX lobe `F D G2 / (4 n.v n.l)` with Fresnel `f(v.h)`.
pub fn ggx_lobe(f: &dyn Fn(f64) -> Rgb, alpha: f64, n: V, v: V, l: V) -> Rgb {
    let (n_v, n_l) = (dot(n, v), dot(n, l));
    if n_v <= 0.0 || n_l <= 0.0 {
        return [0.0; 3];
    }
    let h = unit(std::array::from_fn(|k| v[k] + l[k]));
    let k = ggx(dot(n, h), alpha) * g2(n_v, n_l, alpha) / (4.0 * n_v * n_l);
    f(dot(v, h)).map(|x| x * k)
}

/// `integral of g(l) dl` over the hemisphere around `n`, by two stratified sample sets of
/// `grid` by `grid` points (one distributed like the GGX lobe of `alpha` toward `v`, one like
/// the cosine), combined with the balance heuristic. Sharp lobes and broad ones both converge.
pub fn hemisphere(n: V, v: V, alpha: f64, grid: usize, g: &dyn Fn(V) -> Rgb) -> Rgb {
    let pdf_ggx = |l: V| {
        let h = unit(std::array::from_fn(|k| v[k] + l[k]));
        let v_h = dot(v, h);
        if v_h <= 0.0 {
            0.0
        } else {
            ggx(dot(n, h), alpha) * dot(n, h) / (4.0 * v_h)
        }
    };
    let pdf_cos = |l: V| dot(n, l).max(0.0) / PI;
    let mut sum = [0.0; 3];
    for i in 0..grid {
        for j in 0..grid {
            let (u1, u2) = (
                (i as f64 + 0.5) / grid as f64,
                (j as f64 + 0.5) / grid as f64,
            );
            let theta_h = libm::atan(alpha * (u1 / (1.0 - u1)).sqrt());
            let h = local(n, theta_h, 2.0 * PI * u2);
            let from_ggx: V = std::array::from_fn(|k| 2.0 * dot(v, h) * h[k] - v[k]);
            let from_cos = local(n, libm::asin(u1.sqrt()), 2.0 * PI * u2);
            for l in [from_ggx, from_cos] {
                if dot(n, l) <= 0.0 {
                    continue;
                }
                let p = pdf_ggx(l) + pdf_cos(l);
                let value = g(l);
                for k in 0..3 {
                    sum[k] += value[k] / p;
                }
            }
        }
    }
    sum.map(|s| s / (grid * grid) as f64)
}

/// The DFG integrals of the Fresnel-free lobe `D G2 / (4 n.v)` at view cosine `mu`: total,
/// weighted by `(1 - v.h)^5`, and by `v.h (1 - v.h)^6`.
pub fn dfg(mu: f64, alpha: f64, grid: usize) -> [f64; 3] {
    let n = [0.0, 0.0, 1.0];
    let v = [(1.0 - mu * mu).sqrt(), 0.0, mu];
    let weighted = |l: V| {
        let h = unit(std::array::from_fn(|k| v[k] + l[k]));
        let c = dot(v, h);
        let k = ggx_lobe(&|_| [1.0; 3], alpha, n, v, l)[0] * dot(n, l);
        [
            k,
            k * libm::pow(1.0 - c, 5.0),
            k * c * libm::pow(1.0 - c, 6.0),
        ]
    };
    hemisphere(n, v, alpha, grid, &weighted)
}

/// Every direction of a `rows` by `2 rows` latitude-longitude grid, with its solid angle.
pub fn sphere(rows: usize) -> Vec<(V, f64)> {
    let cols = 2 * rows;
    let mut out = Vec::with_capacity(rows * cols);
    for i in 0..rows {
        let (t0, t1) = (
            PI * i as f64 / rows as f64,
            PI * (i + 1) as f64 / rows as f64,
        );
        let omega = (libm::cos(t0) - libm::cos(t1)) * 2.0 * PI / cols as f64;
        let theta = f64::midpoint(t0, t1);
        for j in 0..cols {
            let phi = 2.0 * PI * (j as f64 + 0.5) / cols as f64;
            let d = [
                libm::sin(theta) * libm::cos(phi),
                libm::cos(theta),
                libm::sin(theta) * libm::sin(phi),
            ];
            out.push((d, omega));
        }
    }
    out
}

/// Irradiance at `n` from radiance `light`, summed over `directions`.
pub fn irradiance(directions: &[(V, f64)], light: &dyn Fn(V) -> Rgb, n: V) -> Rgb {
    let mut e = [0.0; 3];
    for &(d, omega) in directions {
        let c = dot(n, d);
        if c > 0.0 {
            let l = light(d);
            for k in 0..3 {
                e[k] += l[k] * c * omega;
            }
        }
    }
    e
}

/// Irradiance at `n` from the nine-coefficient spherical-harmonic fit of `light`.
pub fn sh9_irradiance(directions: &[(V, f64)], light: &dyn Fn(V) -> Rgb, n: V) -> Rgb {
    let basis = |[x, y, z]: V| -> [f64; 9] {
        let c0 = 0.5 * (1.0 / PI).sqrt();
        let c1 = (3.0 / (4.0 * PI)).sqrt();
        let c2 = 0.5 * (15.0 / PI).sqrt();
        let c3 = 0.25 * (5.0 / PI).sqrt();
        [
            c0,
            c1 * y,
            c1 * z,
            c1 * x,
            c2 * x * y,
            c2 * y * z,
            c3 * (3.0 * z * z - 1.0),
            c2 * x * z,
            0.5 * c2 * (x * x - y * y),
        ]
    };
    let band = [
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
    let mut coefficients = [[0.0; 3]; 9];
    for &(d, omega) in directions {
        let l = light(d);
        for (c, y) in coefficients.iter_mut().zip(basis(d)) {
            for k in 0..3 {
                c[k] += l[k] * y * omega;
            }
        }
    }
    let y = basis(n);
    std::array::from_fn(|k| (0..9).map(|i| coefficients[i][k] * y[i] * band[i]).sum())
}

/// GGX-prefiltered radiance along `r` (Karis 2013's estimator converges to this): the mean of
/// `light` under the weight `D(h) n.l` with normal and view along `r`.
pub fn prefiltered(light: &dyn Fn(V) -> Rgb, r: V, alpha: f64, grid: usize) -> Rgb {
    let weight = |l: V| {
        let h = unit(std::array::from_fn(|k| r[k] + l[k]));
        ggx(dot(r, h), alpha) * dot(r, l)
    };
    let lit = hemisphere(r, r, alpha, grid, &|l| light(l).map(|c| c * weight(l)));
    let total = hemisphere(r, r, alpha, grid, &|l| [weight(l); 3]);
    std::array::from_fn(|k| lit[k] / total[k])
}
