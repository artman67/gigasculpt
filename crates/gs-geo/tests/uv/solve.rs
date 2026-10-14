//! The flattener's solver (decision 0061 sections 1 and 3, step UV3): a Jacobi-preconditioned
//! conjugate gradient on a sparse Hermitian system, to a relative residual of 1e-6 in at most
//! 2,000 iterations, its sums fixed-chunk so the bits do not depend on the thread count.

use gs_geo::uv::solve::{MAX_ITERATIONS, Matrix, TOLERANCE, solve};

/// A matrix from its rows, each a list of (column, value).
fn rows(rows: &[Vec<(u32, [f64; 2])>]) -> Matrix {
    let mut matrix = Matrix {
        start: vec![0],
        column: Vec::new(),
        value: Vec::new(),
    };
    for row in rows {
        for &(c, v) in row {
            matrix.column.push(c);
            matrix.value.push(v);
        }
        matrix.start.push(matrix.column.len());
    }
    matrix
}

/// `a x`, written here apart from the solver's own product: each complex entry `c + i d` acts on
/// `x + i y` as the real block `[[c, -d], [d, c]]`.
fn times(a: &Matrix, x: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out = vec![[0.0; 2]; x.len()];
    for (j, o) in out.iter_mut().enumerate() {
        for i in a.start[j]..a.start[j + 1] {
            let ([c, d], [re, im]) = (a.value[i], x[a.column[i] as usize]);
            o[0] += c * re - d * im;
            o[1] += d * re + c * im;
        }
    }
    out
}

/// The length of `x` as a real vector.
fn norm(x: &[[f64; 2]]) -> f64 {
    x.iter()
        .map(|z| z[0] * z[0] + z[1] * z[1])
        .sum::<f64>()
        .sqrt()
}

/// `|b - a x| / |b|`.
fn relative_residual(a: &Matrix, b: &[[f64; 2]], x: &[[f64; 2]]) -> f64 {
    let ax = times(a, x);
    let r: Vec<[f64; 2]> = (b.iter().zip(&ax))
        .map(|(b, ax)| [b[0] - ax[0], b[1] - ax[1]])
        .collect();
    norm(&r) / norm(b)
}

#[test]
fn a_small_known_system_is_solved_to_the_tolerance() {
    // Hermitian and diagonally dominant, so positive definite.
    let a = rows(&[
        vec![(0, [4.0, 0.0]), (1, [1.0, 1.0])],
        vec![(0, [1.0, -1.0]), (1, [3.0, 0.0]), (2, [0.0, 1.0])],
        vec![(1, [0.0, -1.0]), (2, [2.0, 0.0])],
    ]);
    let want = [[1.0, 2.0], [-0.5, 0.25], [3.0, -1.0]];
    let b = times(&a, &want);
    let got = solve(&a, &b, vec![[0.0; 2]; 3]);
    assert!(got.converged, "the solve converges");
    assert!(
        got.iterations <= 6,
        "conjugate gradient takes at most the real system's size, 6, not {}",
        got.iterations
    );
    let r = relative_residual(&a, &b, &got.x);
    assert!(r <= TOLERANCE, "relative residual {r}");
    assert!(
        (got.residual - r).abs() <= TOLERANCE,
        "the reported residual {} is the true one, {r}",
        got.residual
    );
    for (x, w) in got.x.iter().zip(&want) {
        let (du, dv) = (x[0] - w[0], x[1] - w[1]);
        let error = (du * du + dv * dv).sqrt();
        assert!(error < 1e-5, "{x:?} is {w:?}");
    }
}

/// A 2D grid of `n` by `n` unknowns, each tied to its neighbours with complex weights, Hermitian
/// and diagonally dominant; and a right-hand side that varies over it.
fn grid_system(n: u32) -> (Matrix, Vec<[f64; 2]>) {
    let at = |i: u32, j: u32| i * n + j;
    let mut all = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let mut row = Vec::new();
            if i > 0 {
                row.push((at(i - 1, j), [-1.0, 0.0]));
            }
            if j > 0 {
                row.push((at(i, j - 1), [-1.0, -0.1]));
            }
            row.push((at(i, j), [4.5, 0.0]));
            if j + 1 < n {
                row.push((at(i, j + 1), [-1.0, 0.1]));
            }
            if i + 1 < n {
                row.push((at(i + 1, j), [-1.0, 0.0]));
            }
            all.push(row);
        }
    }
    let b = (0..n * n)
        .map(|k| {
            let t = f64::from(k % 97) / 97.0;
            [t - 0.5, f64::from(k % 13) / 13.0]
        })
        .collect();
    (rows(&all), b)
}

#[test]
fn the_solve_gives_the_same_bits_at_one_and_twenty_four_threads() {
    // 22,500 unknowns: several of gs-par's fixed chunks.
    let (a, b) = grid_system(150);
    let at = |threads| gs_par::with_threads(threads, || solve(&a, &b, vec![[0.0; 2]; b.len()]));
    let (one, many) = (at(1), at(24));
    assert!(one.converged, "the grid converges");
    assert_eq!(one.iterations, many.iterations, "the same iterations");
    let bits =
        |x: &[[f64; 2]]| -> Vec<[u64; 2]> { x.iter().map(|z| z.map(f64::to_bits)).collect() };
    assert_eq!(bits(&one.x), bits(&many.x), "the same bits");
    assert_eq!(
        one.residual.to_bits(),
        many.residual.to_bits(),
        "the same residual"
    );
}

#[test]
fn the_solve_stops_at_the_iteration_cap_and_says_so() {
    // A chain of 20,000 unknowns pushed at one end: the answer reaches the far end, but each
    // iteration carries the push only one link further.
    let n = 20_000u32;
    let all: Vec<Vec<(u32, [f64; 2])>> = (0..n)
        .map(|j| {
            let mut row = Vec::new();
            if j > 0 {
                row.push((j - 1, [-1.0, 0.0]));
            }
            row.push((j, [2.0, 0.0]));
            if j + 1 < n {
                row.push((j + 1, [-1.0, 0.0]));
            }
            row
        })
        .collect();
    let a = rows(&all);
    let mut b = vec![[0.0; 2]; n as usize];
    b[0] = [1.0, 0.0];
    let got = solve(&a, &b, vec![[0.0; 2]; n as usize]);
    assert_eq!(got.iterations, MAX_ITERATIONS, "it stops at the cap");
    assert!(!got.converged, "and says it did not converge");
    let r = relative_residual(&a, &b, &got.x);
    assert!(
        got.residual > TOLERANCE && (got.residual - r).abs() <= 1e-9,
        "it reports the residual it reached, {} (true {r})",
        got.residual
    );
}
