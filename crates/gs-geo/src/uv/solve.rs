//! The flattener's solver (decision 0061 section 1): a Jacobi-preconditioned conjugate gradient
//! on a sparse Hermitian positive definite system of complex unknowns. It runs on the real system
//! twice the size (a complex `c + i d` acting on `x + i y` is the real block `[[c, -d], [d, c]]`),
//! so its dot products are real: `re re + im im` summed over the unknowns. Every sum is a
//! [`gs_par::reduce`] and every update a [`gs_par::apply`] of one row each, so the bits do not
//! depend on the thread count.

/// The relative residual `|b - A x| / |b|` the solve stops at.
pub const TOLERANCE: f64 = 1e-6;
/// The most iterations the solve takes.
pub const MAX_ITERATIONS: usize = 2000;

/// A sparse matrix on complex unknowns `[re, im]`, by rows: row `i`'s entries are
/// `start[i]..start[i + 1]`, each a `column` and a `value`. The solve reads it as Hermitian
/// (entry `(j, i)` the conjugate of `(i, j)`) and positive definite.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matrix {
    /// Where each row's entries start, one more than there are rows.
    pub start: Vec<usize>,
    /// Each entry's column.
    pub column: Vec<u32>,
    /// Each entry's value.
    pub value: Vec<[f64; 2]>,
}

/// What the solve reached.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// The unknowns.
    pub x: Vec<[f64; 2]>,
    /// How many iterations it took.
    pub iterations: usize,
    /// Whether `residual` is at most [`TOLERANCE`].
    pub converged: bool,
    /// The true relative residual `|b - A x| / |b|` of `x` (0 when `b` is 0 and so is `A x`).
    pub residual: f64,
}

/// Solves `a x = b` from the start `x`. An unknown whose diagonal entry is not positive stays
/// where `x` puts it. Stops when the residual falls to [`TOLERANCE`] of `|b|`, at
/// [`MAX_ITERATIONS`], or when the matrix proves not positive definite along the way.
pub fn solve(a: &Matrix, b: &[[f64; 2]], mut x: Vec<[f64; 2]>) -> Solution {
    let n = b.len();
    let inverse: Vec<f64> = gs_par::map(n, |i| {
        let d = (a.start[i]..a.start[i + 1])
            .find(|&e| a.column[e] as usize == i)
            .map_or(0.0, |e| a.value[e][0]);
        if d > 0.0 { 1.0 / d } else { 0.0 }
    });
    let scale = dot(b, b).sqrt();
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let mut r = vec![[0.0; 2]; n];
    residual(a, b, &x, &mut r);
    let mut p: Vec<[f64; 2]> = gs_par::map(n, |i| [r[i][0] * inverse[i], r[i][1] * inverse[i]]);
    let mut q = vec![[0.0; 2]; n];
    let (mut rr, mut rz) = sums(&r, &inverse);
    let mut iterations = 0;
    while iterations < MAX_ITERATIONS && rr.sqrt() > TOLERANCE * scale {
        gs_par::apply(&mut q, |i, qi| *qi = row(a, &p, i));
        let pq = dot(&p, &q);
        let usable = pq > 0.0 && pq.is_finite() && rz > 0.0;
        if !usable {
            break;
        }
        let alpha = rz / pq;
        add_scaled(&mut x, alpha, &p);
        add_scaled(&mut r, -alpha, &q);
        let rz_before = rz;
        (rr, rz) = sums(&r, &inverse);
        let beta = rz / rz_before;
        gs_par::apply(&mut p, |i, pi| {
            pi[0] = r[i][0] * inverse[i] + beta * pi[0];
            pi[1] = r[i][1] * inverse[i] + beta * pi[1];
        });
        iterations += 1;
    }
    residual(a, b, &x, &mut r);
    let residual = dot(&r, &r).sqrt() / scale;
    Solution {
        x,
        iterations,
        converged: residual <= TOLERANCE,
        residual,
    }
}

/// Row `i` of `a` times `v`.
fn row(a: &Matrix, v: &[[f64; 2]], i: usize) -> [f64; 2] {
    (a.start[i]..a.start[i + 1]).fold([0.0; 2], |s, e| {
        let ([c, d], [x, y]) = (a.value[e], v[a.column[e] as usize]);
        [s[0] + (c * x - d * y), s[1] + (c * y + d * x)]
    })
}

/// Adds `s w` to `v`.
fn add_scaled(v: &mut [[f64; 2]], s: f64, w: &[[f64; 2]]) {
    gs_par::apply(v, |i, vi| {
        vi[0] += s * w[i][0];
        vi[1] += s * w[i][1];
    });
}

/// Writes `b - a x` into `r`.
fn residual(a: &Matrix, b: &[[f64; 2]], x: &[[f64; 2]], r: &mut [[f64; 2]]) {
    gs_par::apply(r, |i, ri| {
        let ax = row(a, x, i);
        *ri = [b[i][0] - ax[0], b[i][1] - ax[1]];
    });
}

/// The real dot product of `u` and `v`.
fn dot(u: &[[f64; 2]], v: &[[f64; 2]]) -> f64 {
    gs_par::reduce(
        u.len(),
        || 0.0,
        |s, i| s + (u[i][0] * v[i][0] + u[i][1] * v[i][1]),
        |s, t| s + t,
    )
}

/// `|r|^2` and `r` dotted with the preconditioned `r`.
fn sums(r: &[[f64; 2]], inverse: &[f64]) -> (f64, f64) {
    gs_par::reduce(
        r.len(),
        || (0.0, 0.0),
        |(rr, rz), i| {
            let s = r[i][0] * r[i][0] + r[i][1] * r[i][1];
            (rr + s, rz + s * inverse[i])
        },
        |(a, b), (c, d)| (a + c, b + d),
    )
}
