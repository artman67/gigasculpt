//! Primitive shapes (decision 0047): the cube and the cube sphere, built from one subdivided cube
//! grid, and the radial ones (the uv sphere and the cylinder), built as a revolved profile. Each
//! is built along Y and turned to its `axis` as it is placed. Each can be made with detail levels
//! above it (`levels`).

mod cube;
mod cylinder;
mod levels;
mod sphere;

pub(crate) use cube::PrimitiveCube;
pub(crate) use sphere::PrimitiveSphere;

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_1_SQRT_2, TAU};

use gs_protocol::{NO_VERTEX, PlainMesh, Vec3};

use crate::sculpt::Axis;

/// The axis a primitive is built along when the call names none.
pub(crate) fn along_y() -> Axis {
    Axis::Y
}

/// The surface of the cube from -1 to 1 on every axis, each side split into an `n` by `n` grid
/// of quads wound counter-clockwise seen from outside. Grid points shared by sides are one
/// vertex, so there are `6n² + 2` vertices and `6n²` faces.
pub(crate) fn cube_grid(n: u32) -> PlainMesh {
    let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut positions = Vec::new();
    for i in 0..=n {
        for j in 0..=n {
            for k in 0..=n {
                let point = [i, j, k];
                if point.iter().any(|&c| c == 0 || c == n) {
                    index.insert(point, positions.len() as u32);
                    positions.push(point.map(|c| c as f32 * 2.0 / n as f32 - 1.0));
                }
            }
        }
    }
    let mut faces = Vec::new();
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for side in [0, n] {
            for a in 0..n {
                for b in 0..n {
                    let corner = |du: u32, dv: u32| {
                        let mut point = [side; 3];
                        point[u] = a + du;
                        point[v] = b + dv;
                        index[&point]
                    };
                    let quad = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)];
                    faces.push(if side == n {
                        quad
                    } else {
                        [quad[0], quad[3], quad[2], quad[1]]
                    });
                }
            }
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

/// A surface of revolution about Y: one vertex at height `top`, then a ring of `segments`
/// vertices for each (radius, height) in `rings`, top to bottom, then one vertex at height
/// `bottom`. Each end vertex is welded and ringed by a fan of triangles; neighbouring rings are
/// joined by quads. Every face is wound counter-clockwise seen from outside.
pub(crate) fn revolve(top: f64, rings: &[[f64; 2]], bottom: f64, segments: u32) -> PlainMesh {
    let n = segments;
    let mut positions = vec![[0.0, top as f32, 0.0]];
    for &[radius, height] in rings {
        for k in 0..n {
            let [c, s] = turn(u64::from(k), u64::from(n));
            positions.push([(radius * c) as f32, height as f32, (radius * s) as f32]);
        }
    }
    let last = positions.len() as u32;
    positions.push([0.0, bottom as f32, 0.0]);
    let ring = |r: u32, k: u32| ring_vertex(n, r, k);
    let count = rings.len() as u32;
    let mut faces = Vec::new();
    for k in 0..n {
        faces.push([0, ring(0, k + 1), ring(0, k), NO_VERTEX]);
    }
    for r in 1..count {
        for k in 0..n {
            faces.push([
                ring(r - 1, k),
                ring(r - 1, k + 1),
                ring(r, k + 1),
                ring(r, k),
            ]);
        }
    }
    for k in 0..n {
        faces.push([last, ring(count - 1, k), ring(count - 1, k + 1), NO_VERTEX]);
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

/// The cosine and sine of `num / den` of a turn. The angle is folded into the first eighth of a
/// turn before `libm` is asked, so quarter turns are exactly 0 and ±1, and angles mirrored
/// across an axis or a diagonal give exactly mirrored values.
pub(crate) fn turn(num: u64, den: u64) -> [f64; 2] {
    let num = num % den;
    if 2 * num > den {
        let [c, s] = turn(den - num, den);
        return [c, -s];
    }
    if 4 * num > den {
        let [c, s] = turn(den - 2 * num, 2 * den);
        return [-c, s];
    }
    if 8 * num > den {
        let [c, s] = turn(den - 4 * num, 4 * den);
        return [s, c];
    }
    if 8 * num == den {
        return [FRAC_1_SQRT_2, FRAC_1_SQRT_2];
    }
    if num == 0 {
        return [1.0, 0.0];
    }
    let (s, c) = libm::sincos(TAU * num as f64 / den as f64);
    [c, s]
}

/// Vertex `k` of ring `r` of a mesh `revolve` builds with `segments` around.
fn ring_vertex(segments: u32, r: u32, k: u32) -> u32 {
    1 + r * segments + k % segments
}

/// A point built along Y turned to `axis` by a cyclic swap of coordinates (exact, and a
/// rotation, so faces keep their winding) and moved by `center`.
fn placed([x, y, z]: [f32; 3], axis: Axis, Vec3(center): Vec3) -> [f32; 3] {
    let turned = match axis {
        Axis::X => [y, z, x],
        Axis::Y => [x, y, z],
        Axis::Z => [z, x, y],
    };
    std::array::from_fn(|i| turned[i] + center[i])
}

/// `mesh` with every point moved by `to`.
fn moved(mut mesh: PlainMesh, to: impl Fn([f32; 3]) -> [f32; 3]) -> PlainMesh {
    for p in &mut mesh.positions {
        *p = to(*p);
    }
    mesh
}
