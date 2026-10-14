//! Starting meshes for tests, built as plain data independently of the engine's primitives.
//!
//! Each fixture comes with what it is declared to be ([`Expect`]), and `tests/` checks each one
//! is what it claims (a sphere has the volume of a sphere, a torus has a hole), because a
//! collapsed fixture makes every property pass vacuously (research 06, section 2.4).
//!
//! No trigonometry: circles use twelve segments, whose sines and cosines are exact
//! combinations of `0`, `1/2` and `sqrt(3)/2`, so fixtures are bit-identical on every machine.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, GROUP, HIDE, MASK, NO_VERTEX, PlainMesh};

use crate::measure::{P, add, norm, scale};
use crate::validate::Expect;

/// A named starting mesh and what it is.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// Part name to load it under.
    pub name: &'static str,
    /// The mesh.
    pub mesh: PlainMesh,
    /// What it is declared to be.
    pub expect: Expect,
}

/// The standard set, small enough for property tests: an icosphere, a cube, a torus, an open
/// patch with a boundary, a mask that ramps across it and a strip of hidden faces (so mask and
/// visibility commands, and brushes beside hidden faces, have something to work on), and a
/// closed cylinder made of quads and triangles.
pub fn all() -> Vec<Fixture> {
    let fixture = |name, mesh, expect| Fixture { name, mesh, expect };
    vec![
        fixture("icosphere", icosphere(2), Expect::closed(2)),
        fixture("cube", cube(3), Expect::closed(2)),
        fixture("torus", torus(), Expect::closed(0)),
        fixture(
            "patch",
            partly_hidden(ramp_masked(patch(6))),
            Expect::open(1),
        ),
        fixture("cylinder", cylinder(), Expect::closed(2)),
    ]
}

/// `mesh` with the faces whose corners all lie at z > 0.5 hidden.
fn partly_hidden(mut mesh: PlainMesh) -> PlainMesh {
    let hide = (mesh.faces.iter())
        .map(|f| {
            let far = |&v: &u32| v == NO_VERTEX || mesh.positions[v as usize][2] > 0.5;
            u32::from(f.iter().all(far))
        })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(hide),
    };
    mesh.attributes.insert(HIDE.into(), column);
    mesh
}

/// `mesh` with a vertex mask rising from 0 at x = -0.5 to 1 at x = 0.5.
fn ramp_masked(mut mesh: PlainMesh) -> PlainMesh {
    let ramp = (mesh.positions.iter())
        .map(|p| (p[0] + 0.5).clamp(0.0, 1.0))
        .collect();
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(ramp),
    };
    mesh.attributes.insert(MASK.into(), mask);
    mesh
}

/// A closed tetrahedron with legs 1, 2 and 3 along the axes from (0.5, -1.25, 2): volume
/// exactly 1, area exactly 9 (faces 1, 1.5, 3 and 3.5). Moved off the origin so every term of
/// a volume sum about the origin counts. Carries a vertex mask and a face group.
pub fn tetra() -> PlainMesh {
    let mut mesh = PlainMesh {
        positions: vec![
            [0.5, -1.25, 2.0],
            [1.5, -1.25, 2.0],
            [0.5, 0.75, 2.0],
            [0.5, -1.25, 5.0],
        ],
        faces: vec![
            [0, 2, 1, NO_VERTEX],
            [0, 1, 3, NO_VERTEX],
            [0, 3, 2, NO_VERTEX],
            [1, 2, 3, NO_VERTEX],
        ],
        attributes: BTreeMap::new(),
    };
    // On the mask's 2^-24 grid, so a store holds them unchanged.
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(vec![0.125, 0.25, 0.375, 0.5]),
    };
    let group = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(vec![3, 9, 27, 81]),
    };
    mesh.attributes.insert(MASK.into(), mask);
    mesh.attributes.insert(GROUP.into(), group);
    mesh
}

/// A unit sphere of triangles: an icosahedron whose triangles are split in four
/// `subdivisions` times, with new vertices pushed out onto the sphere.
pub fn icosphere(subdivisions: u32) -> PlainMesh {
    let t = f64::midpoint(1.0, 5f64.sqrt()); // the golden ratio
    let mut points: Vec<P> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .map(unit)
    .to_vec();
    let mut triangles: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        triangles = split(&mut points, &triangles);
    }
    let faces = triangles.iter().map(|&[a, b, c]| [a, b, c, NO_VERTEX]);
    mesh(&points, faces.collect())
}

/// Splits every triangle in four through its edge midpoints, pushed out onto the unit sphere.
fn split(points: &mut Vec<P>, triangles: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut middles: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut middle = |a: u32, b: u32| {
        *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
            points.push(unit(add(points[a as usize], points[b as usize])));
            (points.len() - 1) as u32
        })
    };
    let mut out = Vec::new();
    for &[a, b, c] in triangles {
        let (ab, bc, ca) = (middle(a, b), middle(b, c), middle(c, a));
        out.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
    }
    out
}

/// The surface of the cube from -1 to 1, each side an `n` by `n` grid of quads.
pub fn cube(n: u32) -> PlainMesh {
    let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut points: Vec<P> = Vec::new();
    let mut faces = Vec::new();
    for axis in 0..3 {
        let (u, v) = ([1, 2, 0][axis], [2, 0, 1][axis]);
        for side in [0, n] {
            for a in 0..n {
                for b in 0..n {
                    let mut corner = |du: u32, dv: u32| {
                        let mut lattice = [side; 3];
                        lattice[u] = a + du;
                        lattice[v] = b + dv;
                        *index.entry(lattice).or_insert_with(|| {
                            points.push(lattice.map(|l| f64::from(l * 2) / f64::from(n) - 1.0));
                            (points.len() - 1) as u32
                        })
                    };
                    let mut quad = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)];
                    if side == 0 {
                        quad.reverse();
                    }
                    faces.push(quad);
                }
            }
        }
    }
    mesh(&points, faces)
}

/// Cosine and sine of `k` twelfths of a turn, exactly as far as `f64` allows.
fn circle(k: usize) -> [f64; 2] {
    const H: f64 = 0.866_025_403_784_438_6; // sqrt(3) / 2
    const TABLE: [[f64; 2]; 12] = [
        [1.0, 0.0],
        [H, 0.5],
        [0.5, H],
        [0.0, 1.0],
        [-0.5, H],
        [-H, 0.5],
        [-1.0, 0.0],
        [-H, -0.5],
        [-0.5, -H],
        [0.0, -1.0],
        [0.5, -H],
        [H, -0.5],
    ];
    TABLE[k % 12]
}

/// A ring torus around the y axis, radii 1 and 0.375, twelve quads around each way.
pub fn torus() -> PlainMesh {
    let (big, small) = (1.0, 0.375);
    let mut points = Vec::new();
    for i in 0..12 {
        for j in 0..12 {
            let ([cu, su], [cv, sv]) = (circle(i), circle(j));
            let ring = big + small * cv;
            points.push([ring * cu, small * sv, ring * su]);
        }
    }
    let at = |i: usize, j: usize| (i % 12 * 12 + j % 12) as u32;
    let mut faces = Vec::new();
    for i in 0..12 {
        for j in 0..12 {
            faces.push([at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    mesh(&points, faces)
}

/// An open, gently domed square of `n` by `n` quads over x and z from -1 to 1, facing up.
pub fn patch(n: u32) -> PlainMesh {
    let mut points = Vec::new();
    for i in 0..=n {
        for j in 0..=n {
            let x = f64::from(i * 2) / f64::from(n) - 1.0;
            let z = f64::from(j * 2) / f64::from(n) - 1.0;
            points.push([x, (1.0 - x * x) * (1.0 - z * z) / 4.0, z]);
        }
    }
    let at = |i: u32, j: u32| i * (n + 1) + j;
    let mut faces = Vec::new();
    for i in 0..n {
        for j in 0..n {
            faces.push([at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    mesh(&points, faces)
}

/// A closed cylinder of radius 1 from y = -1 to 1: twelve quads around the side and a fan of
/// twelve triangles on each cap. Its volume is exactly 6 (a twelve-sided prism).
pub fn cylinder() -> PlainMesh {
    let mut points: Vec<P> = Vec::new();
    for y in [-1.0, 1.0] {
        points.extend((0..12).map(|k| {
            let [c, s] = circle(k);
            [c, y, s]
        }));
    }
    points.extend([[0.0, -1.0, 0.0], [0.0, 1.0, 0.0]]);
    let (bottom, top) = (|k: usize| (k % 12) as u32, |k: usize| (k % 12 + 12) as u32);
    let mut faces = Vec::new();
    for k in 0..12 {
        faces.push([bottom(k), top(k), top(k + 1), bottom(k + 1)]);
        faces.push([24, bottom(k), bottom(k + 1), NO_VERTEX]);
        faces.push([25, top(k + 1), top(k), NO_VERTEX]);
    }
    mesh(&points, faces)
}

pub(crate) fn unit(p: P) -> P {
    scale(p, 1.0 / norm(p))
}

pub(crate) fn mesh(points: &[P], faces: Vec<[u32; 4]>) -> PlainMesh {
    PlainMesh {
        positions: points.iter().map(|p| p.map(|c| c as f32)).collect(),
        faces,
        attributes: BTreeMap::new(),
    }
}
