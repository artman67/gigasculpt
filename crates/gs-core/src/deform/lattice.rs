//! The cage (decision 0041, amended 2026-10-12, step DF3): a lattice of points across the box,
//! some of them moved, and every point of the part moved as the lattice around it is. Soft
//! weighs a cell's 4 by 4 by 4 lattice points by the uniform cubic B-spline, smooth to its
//! second derivative, with the lattice padded past each face by a point on the line through the
//! last two so the faces move with their points; hard weighs a cell's 8 corners trilinearly.
//! Beyond the box a point moves as the nearest point of the box does.

use super::Ready;

/// The cage: `size` points along the frame's x, y and z, 2 to 32, resting evenly across the
/// box, point `[0, 0, 0]` at its low corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Lattice {
    /// The cubic B-spline when true, trilinear when false.
    pub soft: bool,
    pub size: [u32; 3],
    /// The moved points; every other point stays.
    pub offsets: Vec<Offset>,
}

/// Cage point `at` moved `by` scene units along the frame's axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offset {
    pub at: [u32; 3],
    pub by: [f32; 3],
}

impl Lattice {
    /// The cage with every offset's lengths mapped by `f`.
    #[must_use]
    pub(super) fn scaled(self, f: impl Fn(f32) -> f32) -> Lattice {
        let offsets = (self.offsets.iter())
            .map(|o| Offset {
                at: o.at,
                by: o.by.map(&f),
            })
            .collect();
        Lattice { offsets, ..self }
    }
}

/// The cage made ready: its offsets in `f64` on the lattice padded by one point past each
/// face, x fastest.
#[derive(Clone, Debug)]
pub(super) struct Grid {
    soft: bool,
    /// Points along each axis, without the padding.
    n: [usize; 3],
    by: Vec<[f64; 3]>,
}

impl Grid {
    /// `None` when no point moves, and the cage changes nothing. An offset outside the cage is
    /// left out (the engine refuses one); of a point named twice, the last.
    pub(super) fn new(l: &Lattice) -> Option<Grid> {
        let n = l.size.map(|s| s as usize);
        let inside = |o: &&Offset| (0..3).all(|k| (o.at[k] as usize) < n[k]);
        let moved = || {
            (l.offsets.iter())
                .filter(inside)
                .filter(|o| o.by.iter().any(|&b| b != 0.0))
        };
        if n.iter().any(|&s| s < 2) || moved().next().is_none() {
            return None;
        }
        let m = n.map(|s| s + 2);
        let mut by = vec![[0.0; 3]; m.iter().product()];
        for o in l.offsets.iter().filter(inside) {
            by[index(n, o.at.map(|a| a as usize + 1))] = o.by.map(f64::from);
        }
        // Along each axis in turn, every line's two padding points, those of the axes padded
        // before included, so a corner's padding is the line through the padded edges.
        for k in 0..3 {
            let (a, b) = ((k + 1) % 3, (k + 2) % 3);
            for (i, j) in (0..m[a]).flat_map(|i| (0..m[b]).map(move |j| (i, j))) {
                let at = |s: usize| {
                    let mut p = [0; 3];
                    (p[k], p[a], p[b]) = (s, i, j);
                    index(n, p)
                };
                let (last, end) = (n[k], n[k] + 1);
                by[at(0)] = line(by[at(1)], by[at(2)]);
                by[at(end)] = line(by[at(last)], by[at(last - 1)]);
            }
        }
        Some(Grid {
            soft: l.soft,
            n,
            by,
        })
    }
}

/// The grid as the vertex stage reads it (gs-render `shaders/deform_tables.wgsl` `lattice`, `DF9b`):
/// the points along each axis and 1 when soft, then every padded offset; zeros for no grid.
pub(super) fn table(g: Option<&Grid>) -> Vec<[f32; 4]> {
    let Some(g) = g else {
        return vec![[0.0; 4]];
    };
    let head = [g.n[0], g.n[1], g.n[2], usize::from(g.soft)].map(|v| v as f32);
    let by =
        g.by.iter()
            .map(|b| [b[0] as f32, b[1] as f32, b[2] as f32, 0.0]);
    std::iter::once(head).chain(by).collect()
}

/// Where padded point `p` of a cage of `n` points a side is in the grid.
fn index(n: [usize; 3], p: [usize; 3]) -> usize {
    p[0] + (n[0] + 2) * (p[1] + (n[1] + 2) * p[2])
}

/// The point past `a` on the line from `b` through `a`: `2 a - b`.
fn line(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|k| 2.0 * a[k] - b[k])
}

/// The cage's change of frame coordinates `c` at weight `w`: the offsets around it, each
/// weighted by its place in its cell along each axis, summed, then times `w`.
pub(super) fn lattice(r: &Ready, c: [f64; 3], w: f64) -> [f64; 3] {
    let Some(g) = &r.cage else {
        return [0.0; 3];
    };
    let mut first = [0; 3];
    let mut weights = [[0.0; 4]; 3];
    for k in 0..3 {
        let (lo, hi, n) = (r.lo[k], r.hi[k], g.n[k]);
        let f = if hi > lo {
            (c[k].clamp(lo, hi) - lo) / (hi - lo) * (n - 1) as f64
        } else {
            0.0
        };
        let cell = (f as usize).min(n - 2);
        let t = f - cell as f64;
        // The padded index of the cell's first point: one before the cell when soft.
        first[k] = if g.soft { cell } else { cell + 1 };
        weights[k] = if g.soft {
            spline(t)
        } else {
            [1.0 - t, t, 0.0, 0.0]
        };
    }
    let reach = if g.soft { 4 } else { 2 };
    let mut d = [0.0; 3];
    for (z, &wz) in weights[2][..reach].iter().enumerate() {
        for (y, &wy) in weights[1][..reach].iter().enumerate() {
            let row = index(g.n, [first[0], first[1] + y, first[2] + z]);
            for (by, &wx) in g.by[row..row + reach].iter().zip(&weights[0][..reach]) {
                let b = wx * wy * wz;
                for (d, by) in d.iter_mut().zip(by) {
                    *d += b * by;
                }
            }
        }
    }
    d.map(|d| d * w)
}

/// The uniform cubic B-spline's weights of a cell's four points at place `t` in it, 0 to 1:
/// the point before the cell, its two ends, and the point after it.
fn spline(t: f64) -> [f64; 4] {
    let (s, t2, t3) = (1.0 - t, t * t, t * t * t);
    [
        s * s * s / 6.0,
        (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0,
        (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0,
        t3 / 6.0,
    ]
}
