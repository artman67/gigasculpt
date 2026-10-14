//! Test meshes: the golden spike's dented icosphere, a quad sphere and a flat quad grid.
//! Shared with `examples/previews.rs`.

use gs_protocol::{NO_VERTEX, PlainMesh};
use std::collections::BTreeMap;

/// A mesh with no attribute columns.
pub fn plain(positions: Vec<[f32; 3]>, faces: Vec<[u32; 4]>) -> PlainMesh {
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

fn unit(p: [f32; 3]) -> [f32; 3] {
    let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    [p[0] / l, p[1] / l, p[2] / l]
}

/// The unit icosphere of the golden spike: 20 * 4^subdiv triangles.
pub fn icosphere(subdiv: u32) -> PlainMesh {
    let t = f32::midpoint(1.0, 5f32.sqrt());
    #[rustfmt::skip]
    let mut pos: Vec<[f32; 3]> = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ].map(unit).to_vec();
    #[rustfmt::skip]
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdiv {
        let mut cache: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        let mut mid = |a: u32, b: u32, pos: &mut Vec<[f32; 3]>| {
            *cache.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (pa, pb) = (pos[a as usize], pos[b as usize]);
                pos.push(unit([pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]]));
                (pos.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let (ab, bc, ca) = (
                mid(a, b, &mut pos),
                mid(b, c, &mut pos),
                mid(c, a, &mut pos),
            );
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    plain(
        pos,
        tris.into_iter()
            .map(|[a, b, c]| [a, b, c, NO_VERTEX])
            .collect(),
    )
}

/// The spike's stroke: 8 inward dabs along an arc on the upper front, `depth` deep (0.12 in
/// the spike; 0.1152 is its "4% weaker" stroke).
pub fn dent(mesh: &mut PlainMesh, depth: f32) {
    let (a, b) = (unit([-0.2, 0.75, 0.65]), unit([0.55, 0.45, 0.7]));
    let r2 = 0.35f32 * 0.35;
    for k in 0..8 {
        let t = k as f32 / 7.0;
        let c = unit([
            a[0] * (1.0 - t) + b[0] * t,
            a[1] * (1.0 - t) + b[1] * t,
            a[2] * (1.0 - t) + b[2] * t,
        ]);
        for p in &mut mesh.positions {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            let u = 1.0 - (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) / r2;
            if u > 0.0 {
                let w = u * u * (3.0 - 2.0 * u) * depth * 0.25;
                let n = unit(*p);
                *p = [p[0] - n[0] * w, p[1] - n[1] * w, p[2] - n[2] * w];
            }
        }
    }
}

/// A unit sphere of 6 * n * n quads: a cube grid pushed onto the sphere, welded at the seams.
pub fn quad_sphere(n: u32) -> PlainMesh {
    // Face normal, then u and v axes with u x v = normal, so quads wind counter-clockwise outside.
    const FACES: [[[i32; 3]; 3]; 6] = [
        [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
        [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
        [[0, 1, 0], [0, 0, 1], [1, 0, 0]],
        [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
        [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
        [[0, 0, -1], [0, 1, 0], [1, 0, 0]],
    ];
    let n = n.cast_signed();
    let mut index: BTreeMap<[i32; 3], u32> = BTreeMap::new();
    let mut positions = Vec::new();
    let mut vertex = |key: [i32; 3]| {
        *index.entry(key).or_insert_with(|| {
            let c = key.map(|k| k as f32 / n as f32);
            // Spherified cube: more even quads than plain normalization.
            let (x2, y2, z2) = (c[0] * c[0], c[1] * c[1], c[2] * c[2]);
            positions.push([
                c[0] * (1.0 - y2 / 2.0 - z2 / 2.0 + y2 * z2 / 3.0).sqrt(),
                c[1] * (1.0 - z2 / 2.0 - x2 / 2.0 + z2 * x2 / 3.0).sqrt(),
                c[2] * (1.0 - x2 / 2.0 - y2 / 2.0 + x2 * y2 / 3.0).sqrt(),
            ]);
            (positions.len() - 1) as u32
        })
    };
    let mut faces = Vec::new();
    for [normal, u, v] in FACES {
        let at = |i: i32, j: i32| -> [i32; 3] {
            std::array::from_fn(|k| normal[k] * n + (2 * i - n) * u[k] + (2 * j - n) * v[k])
        };
        for i in 0..n {
            for j in 0..n {
                faces.push([
                    vertex(at(i, j)),
                    vertex(at(i + 1, j)),
                    vertex(at(i + 1, j + 1)),
                    vertex(at(i, j + 1)),
                ]);
            }
        }
    }
    plain(positions, faces)
}

/// An n by n grid of quads covering -1..1 in x and y at z = 0, facing +Z.
pub fn grid(n: u32) -> PlainMesh {
    let xs = steps(-1.0, 1.0, n);
    height_field(&xs, &xs, |_| 0.0)
}

/// `n + 1` evenly spaced values from `a` to `b`.
pub fn steps(a: f32, b: f32, n: u32) -> Vec<f32> {
    let step = (b - a) / n as f32;
    (0..=n).map(|i| a + i as f32 * step).collect()
}

/// Quads through the points `(x, y, z(x))` for every x in `xs` and y in `ys`, rows of x
/// one after another; facing +Z where z is flat.
pub fn height_field(xs: &[f32], ys: &[f32], z: impl Fn(f32) -> f32) -> PlainMesh {
    let mut positions = Vec::with_capacity(xs.len() * ys.len());
    for &y in ys {
        for &x in xs {
            positions.push([x, y, z(x)]);
        }
    }
    let (columns, rows) = (xs.len() as u32 - 1, ys.len() as u32 - 1);
    let id = |i: u32, j: u32| j * (columns + 1) + i;
    let faces = (0..rows)
        .flat_map(|j| {
            (0..columns).map(move |i| [id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)])
        })
        .collect();
    plain(positions, faces)
}
