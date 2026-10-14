//! The pairs of triangles that may meet, found on a grid, and the tests `boolean.check` runs on
//! them: two triangles crossing, and two lying on each other. Serial and sorted, so an answer
//! never depends on the thread count.

use crate::math::{cross, dot, sub};

/// A point, in `f64`.
pub(super) type P = [f64; 3];

/// The mean edge length of `tris`.
pub(super) fn mean_edge(tris: &[[P; 3]]) -> f64 {
    let length = |a: P, b: P| dot(sub(a, b), sub(a, b)).sqrt();
    let sum: f64 = (tris.iter())
        .map(|t| (0..3).map(|k| length(t[k], t[(k + 1) % 3])).sum::<f64>())
        .sum();
    sum / (3 * tris.len()).max(1) as f64
}

/// Every two triangles of `tris` whose boxes, each grown by `pad`, overlap: the pairs, lower
/// first, sorted, each once. With `across` set to n, only pairs of a triangle below n and one
/// at n or above. The cells are two mean edges wide, or a sixteenth of the largest box when
/// that is wider, so no triangle lands in more than a few thousand.
pub(super) fn near(tris: &[[P; 3]], pad: f64, across: Option<usize>) -> Vec<(u32, u32)> {
    let boxes: Vec<(P, P)> = tris.iter().map(|t| grown(t, pad)).collect();
    let low = |m: P, b: &(P, P)| std::array::from_fn(|a| m[a].min(b.0[a]));
    let lo: P = boxes.iter().fold([f64::INFINITY; 3], low);
    let widest = (boxes.iter())
        .flat_map(|(a, b)| (0..3).map(move |k| b[k] - a[k]))
        .fold(0.0, f64::max);
    let cell = (2.0 * mean_edge(tris))
        .max(widest / 16.0)
        .max(f64::MIN_POSITIVE);
    let key = |p: P| -> [u64; 3] { std::array::from_fn(|a| ((p[a] - lo[a]) / cell) as u64) };
    let mut entries: Vec<([u64; 3], u32)> = Vec::new();
    for (t, (a, b)) in boxes.iter().enumerate() {
        let (ka, kb) = (key(*a), key(*b));
        for x in ka[0]..=kb[0] {
            for y in ka[1]..=kb[1] {
                for z in ka[2]..=kb[2] {
                    entries.push(([x, y, z], t as u32));
                }
            }
        }
    }
    entries.sort_unstable();
    let wanted = |s: u32, t: u32| across.is_none_or(|n| (s as usize) < n && t as usize >= n);
    let mut pairs = Vec::new();
    for cell in entries.chunk_by(|a, b| a.0 == b.0) {
        for (i, &(_, s)) in cell.iter().enumerate() {
            for &(_, t) in &cell[i + 1..] {
                if wanted(s, t) && overlap(&boxes[s as usize], &boxes[t as usize]) {
                    pairs.push((s, t));
                }
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

fn grown(t: &[P; 3], pad: f64) -> (P, P) {
    let lo = std::array::from_fn(|a| t[0][a].min(t[1][a]).min(t[2][a]) - pad);
    let hi = std::array::from_fn(|a| t[0][a].max(t[1][a]).max(t[2][a]) + pad);
    (lo, hi)
}

fn overlap(a: &(P, P), b: &(P, P)) -> bool {
    (0..3).all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k])
}

/// True when an edge of `a` passes through the inside of `b`, or one of `b` through `a`'s, by
/// more than `eps` (six times a tetrahedron's volume) on every side: touching, and lying flat
/// on each other, do not count.
pub(super) fn crosses(a: &[P; 3], b: &[P; 3], eps: f64) -> bool {
    let edges = |t: &[P; 3]| [0, 1, 2].map(|k| (t[k], t[(k + 1) % 3]));
    (edges(a).into_iter()).any(|e| through(e, b, eps))
        || (edges(b).into_iter()).any(|e| through(e, a, eps))
}

/// True when segment `(p, q)` passes through the inside of triangle `t`: its ends lie on either
/// side of `t`'s plane and its line on one side of all three of `t`'s edges, each beyond `eps`.
/// (gs-testkit's `closed` claim asks the same with code of its own, as it must.)
fn through((p, q): (P, P), t: &[P; 3], eps: f64) -> bool {
    let side = |d: f64| i8::from(d > eps) - i8::from(d < -eps);
    let plane = |x: P| side(orient(t[0], t[1], t[2], x));
    if plane(p) * plane(q) >= 0 {
        return false;
    }
    let turns = [0, 1, 2].map(|k| side(orient(p, q, t[k], t[(k + 1) % 3])));
    turns[0] != 0 && turns.iter().all(|&s| s == turns[0])
}

/// Six times the signed volume of the tetrahedron `a b c d`.
fn orient(a: P, b: P, c: P, d: P) -> f64 {
    dot(sub(b, a), cross(sub(c, a), sub(d, a)))
}

/// True when every corner of each triangle lies within `depth` of the other's plane and the two,
/// seen along the first's normal, overlap by more than a millionth of the smaller.
pub(super) fn lie_on(a: &[P; 3], b: &[P; 3], depth: f64) -> bool {
    let normal = |t: &[P; 3]| cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let within = |t: &[P; 3], s: &[P; 3]| {
        let n = normal(s);
        let length = dot(n, n).sqrt();
        length > 0.0
            && t.iter()
                .all(|&p| dot(n, sub(p, s[0])).abs() <= depth * length)
    };
    if !(within(a, b) && within(b, a)) {
        return false;
    }
    let n = normal(a).map(f64::abs);
    let drop = if n[0] >= n[1] && n[0] >= n[2] {
        0
    } else if n[1] >= n[2] {
        1
    } else {
        2
    };
    let (u, v) = ((drop + 1) % 3, (drop + 2) % 3);
    let flat = |t: &[P; 3]| t.map(|p| [p[u], p[v]]);
    let (mut window, subject) = (flat(a), flat(b));
    if area(&window) < 0.0 {
        window.reverse();
    }
    let smaller = area(&window).abs().min(area(&subject).abs());
    area(&clip(&subject, &window)).abs() > 1e-6 * smaller
}

/// `subject` clipped to the convex, counterclockwise triangle `window` (Sutherland and Hodgman).
fn clip(subject: &[[f64; 2]], window: &[[f64; 2]; 3]) -> Vec<[f64; 2]> {
    let mut out = subject.to_vec();
    for k in 0..3 {
        let (p, q) = (window[k], window[(k + 1) % 3]);
        let side = |r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        let meet = |s: [f64; 2], r: [f64; 2], ds: f64, dr: f64| {
            let t = ds / (ds - dr);
            [s[0] + t * (r[0] - s[0]), s[1] + t * (r[1] - s[1])]
        };
        let input = std::mem::take(&mut out);
        for (i, &r) in input.iter().enumerate() {
            let s = input[(i + input.len() - 1) % input.len()];
            let (ds, dr) = (side(s), side(r));
            if dr >= 0.0 {
                if ds < 0.0 {
                    out.push(meet(s, r, ds, dr));
                }
                out.push(r);
            } else if ds >= 0.0 {
                out.push(meet(s, r, ds, dr));
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The signed area of a polygon, counterclockwise positive.
fn area(polygon: &[[f64; 2]]) -> f64 {
    let n = polygon.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (p, q) = (polygon[i], polygon[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    twice / 2.0
}
