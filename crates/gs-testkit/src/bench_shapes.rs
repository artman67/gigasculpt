//! Research 28's four bench shapes, the inputs Auto UV is measured on (decision 0061 section
//! 10): its unwrap tests and perf scenarios read these, so every result is on the same faces.
//!
//! | Shape | What it is |
//! |---|---|
//! | [`sphere`] | A cube-sphere: a cube cut `n` by `n` per side, its points pushed onto the unit sphere. Closed, genus 0. |
//! | [`head`] | The sphere's lattice pushed out into a head: an ellipsoid with nose, brow, eye sockets, cheeks, chin, lips, ears and sculpt noise. Closed, genus 0, mirror-symmetric in x to the bit. |
//! | [`ring`] | A hard-surface part: a rounded square swept by a rounded-rectangle profile. Closed, genus 1, one face group per profile segment (4 flats and 4 bevels, groups 1 to 8). |
//! | [`sleeve`] | A tube open at both ends with folds and an elbow bulge. Open, two rims, genus 0. |
//!
//! Each is quads only, wound outward, at a size given in faces and rounded to its lattice:
//! [`all`]`(50_000)` is research 28's 100k-triangle row (sphere and head 91 per side, ring 500
//! by 100, sleeve 224 by 224), 10,086, 10,035 and 10,000 faces its 20k row, and 726, 672 and
//! 784 faces its base row. Built in `f64` with `libm`'s trigonometry, never the platform's, so
//! every machine makes the same bits.

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

use gs_protocol::{AttrDomain, AttrValues, Attribute, GROUP, PlainMesh};

use crate::fixtures::{mesh, unit};
use crate::measure::{P, add, cross, dot, sub};

/// The four shapes at about `faces` faces each, by name.
pub fn all(faces: u32) -> [(&'static str, PlainMesh); 4] {
    [
        ("sphere", sphere(faces)),
        ("head", head(faces)),
        ("ring", ring(faces)),
        ("sleeve", sleeve(faces)),
    ]
}

/// A cube-sphere of about `faces` quads: a cube cut `n` by `n` per side, `n` the nearest whole
/// number to the square root of `faces / 6`, its points on the unit sphere.
pub fn sphere(faces: u32) -> PlainMesh {
    let (points, quads) = cube_sphere(side(faces, 6.0));
    mesh(&points, quads)
}

/// The sphere's lattice at about `faces` quads, each point moved along its direction `d` to a
/// head's radius `r(d)` and squeezed into an ellipsoid: `(0.82 r dx, r dy, 0.92 r dz)`. The
/// face looks along +z. `r` reads `|dx|`, so the head is its own mirror in x.
pub fn head(faces: u32) -> PlainMesh {
    let (points, quads) = cube_sphere(side(faces, 6.0));
    let points: Vec<P> = (points.iter())
        .map(|&d| {
            let r = head_radius(d);
            [0.82 * r * d[0], r * d[1], 0.92 * r * d[2]]
        })
        .collect();
    mesh(&points, quads)
}

/// A rounded-square ring around the y axis of about `faces` quads: `m` stations along a path of
/// half side 1 and corner radius 0.3, each a rounded-rectangle profile of `k` points (half width
/// 0.25, half height 0.15, corner radius a fifth of each), `k` the nearest whole number to the
/// square root of `faces / 5` and `m` to `faces / k`. Each face carries the group of its
/// profile segment: 1 to 4 the flats, 5 to 8 the bevels (a bevel needs `k` of 25 or more to
/// hold a point).
pub fn ring(faces: u32) -> PlainMesh {
    let k = side(faces, 5.0).max(4);
    let m = ((f64::from(faces) / f64::from(k)).round() as u32).max(4);
    let (path, profile) = (rounded_square(1.0, 0.3, m), rounded_square(1.0, 0.2, k));
    let mut points = Vec::new();
    for &([px, pz], _) in &path {
        let [dx, dz] = outward(px, pz);
        for &([qa, qb], _) in &profile {
            let w = 0.25 * qa;
            points.push([px + w * dx, 0.15 * qb, pz + w * dz]);
        }
    }
    let at = |i: u32, j: u32| (i % m) * k + j % k;
    let mut quads = Vec::new();
    let mut groups = Vec::new();
    for i in 0..m {
        for (j, &(_, segment)) in (0..k).zip(&profile) {
            quads.push([at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
            groups.push(segment + 1);
        }
    }
    let quads = outward_wound(&points, quads);
    let mut mesh = mesh(&points, quads);
    let column = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(groups),
    };
    mesh.attributes.insert(GROUP.into(), column);
    mesh
}

/// An open tube along y from -1.5 to 1.5 of about `faces` quads, `a` around by `a` along, `a`
/// the nearest whole number to the square root of `faces`. Its radius is 0.5 with two folds of
/// sculpt-like waves and an elbow bulge on the +x side at y = 0.3.
pub fn sleeve(faces: u32) -> PlainMesh {
    let a = side(faces, 1.0).max(3);
    let mut points = Vec::new();
    for j in 0..=a {
        let y = -1.5 + 3.0 * f64::from(j) / f64::from(a);
        for i in 0..a {
            let th = 2.0 * PI * f64::from(i) / f64::from(a);
            let (cos, sin) = (libm::cos(th), libm::sin(th));
            let mut r = 0.5
                + 0.06 * libm::sin(3.0 * y) * libm::cos(2.0 * th)
                + 0.025 * libm::sin(7.0 * th + 2.0 * y);
            let e = y - 0.3;
            r += 0.04 * libm::exp(-(e * e) / 0.05) * (1.0 + cos);
            points.push([r * cos, y, r * sin]);
        }
    }
    let mut quads = Vec::new();
    for j in 0..a {
        for i in 0..a {
            let (p, q) = (j * a + i, j * a + (i + 1) % a);
            quads.push([p, p + a, q + a, q]);
        }
    }
    mesh(&points, quads)
}

/// The nearest whole number to the square root of `faces / per`, at least 1.
fn side(faces: u32, per: f64) -> u32 {
    ((f64::from(faces) / per).sqrt().round() as u32).max(1)
}

/// The unit-sphere points and outward quads of a cube cut `n` by `n` per side. Points are found
/// by their whole lattice coordinates (from -n to n in steps of 2), so the ones shared along
/// the cube's edges are exact, and numbered in the order first met.
fn cube_sphere(n: u32) -> (Vec<P>, Vec<[u32; 4]>) {
    let n = i64::from(n);
    let mut index: BTreeMap<[i64; 3], u32> = BTreeMap::new();
    let mut lattice: Vec<[i64; 3]> = Vec::new();
    let mut quads = Vec::new();
    for side in 0..6 {
        for a in 0..n {
            for b in 0..n {
                let keys = [(0, 0), (1, 0), (1, 1), (0, 1)]
                    .map(|(da, db)| cube_point(side, 2 * (a + da) - n, 2 * (b + db) - n, n));
                let mut quad = keys.map(|key| {
                    *index.entry(key).or_insert_with(|| {
                        lattice.push(key);
                        (lattice.len() - 1) as u32
                    })
                });
                if !faces_out(keys) {
                    quad.reverse();
                }
                quads.push(quad);
            }
        }
    }
    let n = n as f64;
    let points = lattice.iter().map(|k| unit(k.map(|c| c as f64 / n)));
    (points.collect(), quads)
}

/// The lattice point at `(s, t)` on side `side` of the cube of half side `n`.
fn cube_point(side: u32, s: i64, t: i64, n: i64) -> [i64; 3] {
    match side {
        0 => [n, s, -t],
        1 => [-n, s, t],
        2 => [s, n, -t],
        3 => [s, -n, t],
        4 => [s, t, n],
        _ => [-s, t, -n],
    }
}

/// Whether a lattice quad's first three corners wind counter-clockwise seen from outside: its
/// normal points away from the centre (exact, in whole numbers).
fn faces_out(k: [[i64; 3]; 4]) -> bool {
    // Whole numbers this small stay exact in f64.
    let p = |i: usize| k[i].map(|c| c as f64);
    let normal = cross(sub(p(1), p(0)), sub(p(2), p(0)));
    dot(normal, add(add(p(0), p(1)), add(p(2), p(3)))) >= 0.0
}

/// The head's bumps: centre direction, signed height and width, in the order they are added.
const BUMPS: [(P, f64, f64); 9] = [
    ([0.0, -0.15, 1.0], 0.22, 0.012),  // nose
    ([0.0, -0.32, 1.0], 0.06, 0.03),   // nose base
    ([0.33, 0.12, 0.94], -0.10, 0.01), // eye sockets
    ([0.3, 0.27, 0.9], 0.05, 0.02),    // brow
    ([0.55, -0.2, 0.8], 0.05, 0.05),   // cheeks
    ([0.0, -0.85, 0.5], 0.12, 0.05),   // chin
    ([0.0, -0.55, 0.83], 0.04, 0.006), // lips
    ([1.0, 0.0, -0.05], 0.16, 0.008),  // ears
    ([0.0, 0.6, -0.8], 0.08, 0.2),     // back of the skull
];

/// The head's radius along the unit direction `d`: 1 plus each bump's
/// `height * exp(-(1 - d.c) / width)`, plus two waves of sculpt noise. Reads `|dx|` only.
fn head_radius(d: P) -> f64 {
    let d @ [x, y, z] = [d[0].abs(), d[1], d[2]];
    let mut r = 1.0;
    for (centre, height, width) in BUMPS {
        r += height * libm::exp(-(1.0 - dot(d, unit(centre))) / width);
    }
    r += 0.012 * libm::sin(9.0 * x + 2.0) * libm::sin(11.0 * y) * libm::cos(7.0 * z);
    r + 0.006 * libm::sin(23.0 * y + 5.0 * x) * libm::sin(19.0 * z + 1.0)
}

/// `count` points evenly spaced by length around a rounded square of half side `half` and
/// corner radius `radius`, from (half, radius - half) counter-clockwise, each with its segment:
/// the flats 0 to 3 and the corners 4 to 7, in order around.
fn rounded_square(half: f64, radius: f64, count: u32) -> Vec<([f64; 2], u32)> {
    let flat = 2.0 * (half - radius);
    let period = flat + FRAC_PI_2 * radius;
    let total = 4.0 * period;
    (0..count)
        .map(|i| {
            let s = total * f64::from(i) / f64::from(count);
            let side = ((s / period).floor() as u32).min(3);
            let u = s - f64::from(side) * period;
            let (x, y, segment) = if u < flat {
                (half, -half + radius + u, side)
            } else {
                let (a, c) = ((u - flat) / radius, half - radius);
                (
                    c + radius * libm::cos(a),
                    c + radius * libm::sin(a),
                    4 + side,
                )
            };
            // A quarter turn per side, exactly.
            let turned = [[x, y], [-y, x], [-x, -y], [y, -x]][side as usize];
            (turned, segment)
        })
        .collect()
}

/// The ring's outward direction in the xz plane at path point (px, pz): away from the nearest
/// point of the square of half side 0.7 that holds the path's corner centres.
fn outward(px: f64, pz: f64) -> [f64; 2] {
    let (dx, dz) = (px - px.clamp(-0.7, 0.7), pz - pz.clamp(-0.7, 0.7));
    let length = libm::hypot(dx, dz);
    if length < 1e-9 {
        return [1.0, 0.0];
    }
    [dx / length, dz / length]
}

/// `quads`, every one turned over if their signed volume about the origin is negative.
fn outward_wound(points: &[P], quads: Vec<[u32; 4]>) -> Vec<[u32; 4]> {
    let at = |v: u32| points[v as usize];
    let volume: f64 = (quads.iter())
        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
        .map(|[a, b, c]| dot(at(a), cross(at(b), at(c))))
        .sum();
    if volume >= 0.0 {
        return quads;
    }
    quads.into_iter().map(|[a, b, c, d]| [d, c, b, a]).collect()
}
