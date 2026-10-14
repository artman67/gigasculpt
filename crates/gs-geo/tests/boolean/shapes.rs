//! Research 26's meshes, as its "How the numbers were made" describes them, for the boolean
//! tests and the probe (`examples/boolean_probe.rs`): the perf suite's regular lumpy sphere, the
//! pair, the body and its cutter, and the messy set. Where the research leaves a number out (the
//! pair's turn, where the cutter pokes in, which way the shells and the fold move), the one
//! chosen here is named beside it.

use std::collections::BTreeMap;

use gs_core::transform::Similarity;
use gs_core::{Quat, Vec3};
use gs_protocol::{NO_VERTEX, PlainMesh};

/// The perf suite's regular sphere (perf/runner `mesh::sphere`, not irregular): a cube-sphere
/// spaced by equal angles, `n` quads along each cube edge (12 n^2 triangles), pushed out to a
/// gently lumpy surface of radius about 1. `n` 41 gives 20,172 triangles, 65 gives 50,700, 204
/// gives 499,392, 289 gives 1,002,252, 456 gives 2,495,232 and 645 gives 4,992,300.
pub fn sphere(n: u32) -> PlainMesh {
    let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    for axis in 0..3 {
        for hi in [false, true] {
            for (a, b) in (0..n).flat_map(|a| (0..n).map(move |b| (a, b))) {
                let mut corner = |da: u32, db: u32| {
                    let mut lattice = [if hi { n } else { 0 }; 3];
                    lattice[(axis + 1) % 3] = a + da;
                    lattice[(axis + 2) % 3] = b + db;
                    *index.entry(lattice).or_insert_with(|| {
                        positions.push(place(lattice, n));
                        (positions.len() - 1) as u32
                    })
                };
                let mut quad = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)];
                if !hi {
                    quad.reverse();
                }
                faces.push(quad);
            }
        }
    }
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

/// The point of lattice corner `l` (0 to `n` along each axis) on the lumpy surface.
fn place(l: [u32; 3], n: u32) -> [f32; 3] {
    let quarter = 0.25 * std::f32::consts::PI;
    let c = l.map(|x| libm::tanf(quarter * (x as f32 * 2.0 / n as f32 - 1.0)));
    let d = Vec3::new(c[0], c[1], c[2]).normalized();
    let r = 1.0 + 0.06 * d.x * d.y + 0.04 * d.z * d.z * d.x - 0.03 * d.y * d.z;
    (d * r).to_array()
}

/// The pair's second operand as a transform of the first: scaled 0.8, turned 30 degrees about
/// (1, 1, 1) (the research does not give its turn) and moved by (0.7, 0.3, 0.2).
pub fn pair_b() -> Similarity {
    turned(0.8, 30.0, [0.7, 0.3, 0.2])
}

/// The body's cutter (a 20,172-triangle sphere) as a transform: scaled 0.3 and moved to
/// (0.95, 0.1, 0.05), so it pokes into the body's side (the research does not give where).
pub fn cutter() -> Similarity {
    Similarity {
        translation: Vec3::new(0.95, 0.1, 0.05),
        rotation: Quat::IDENTITY,
        scale: 0.3,
    }
}

/// The messy set's partner, `small_b`: scaled 0.6, turned 30 degrees about (1, 1, 1) and moved
/// by (0.6, 0.2, 0.1).
pub fn small_b() -> Similarity {
    turned(0.6, 30.0, [0.6, 0.2, 0.1])
}

/// Scaled by `scale`, turned `degrees` about (1, 1, 1), then moved by `to`.
pub fn turned(scale: f32, degrees: f32, to: [f32; 3]) -> Similarity {
    let axis = Vec3::new(1.0, 1.0, 1.0).normalized();
    Similarity {
        translation: Vec3::from_array(to),
        rotation: Quat::about(axis, degrees.to_radians()),
        scale,
    }
}

/// `mesh` with a copy of itself scaled 0.7 and moved by 0.5 along x, in the same mesh: two
/// shells that cross.
pub fn shells(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    let base = mesh.positions.len() as u32;
    let moved = mesh.positions.iter().map(|p| {
        let q = p.map(|c| c * 0.7);
        [q[0] + 0.5, q[1], q[2]]
    });
    out.positions.extend(moved);
    let shifted = mesh.faces.iter().map(|f| f.map(|v| v + base));
    out.faces.extend(shifted);
    out
}

/// `mesh` with every point above height 0.6 pushed down by 1.8, so its cap folds through it.
pub fn fold(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        if p[1] > 0.6 {
            p[1] -= 1.8;
        }
    }
    out
}

/// `mesh` with every face turned the other way: inside out.
pub fn inverted(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    for f in &mut out.faces {
        let n = if f[3] == NO_VERTEX { 3 } else { 4 };
        f[..n].reverse();
    }
    out
}

/// `mesh` with a hole: every face whose corners all sit at height 0.8 or more is removed.
pub fn open(mesh: &PlainMesh) -> PlainMesh {
    let top = |v: &u32| *v == NO_VERTEX || mesh.positions[*v as usize][1] >= 0.8;
    let faces = mesh.faces.iter().copied();
    PlainMesh {
        faces: faces.filter(|f| !f.iter().all(top)).collect(),
        ..mesh.clone()
    }
}

/// `mesh` with a fin: one extra triangle on the first face's first edge, so that edge has three
/// faces.
pub fn fin(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    let [a, b, ..] = mesh.faces[0];
    let p = mesh.positions[a as usize].map(|c| c * 1.3);
    out.positions.push(p);
    out.faces
        .push([a, b, (out.positions.len() - 1) as u32, NO_VERTEX]);
    out
}

/// The axis-aligned box from `lo` to `hi`, one quad a side, wound outward.
pub fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> PlainMesh {
    let positions = (0..8)
        .map(|i| [0, 1, 2].map(|a| if i >> a & 1 == 1 { hi[a] } else { lo[a] }))
        .collect();
    let faces = vec![
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}
