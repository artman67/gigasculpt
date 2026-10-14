//! Auto UV's cutter, seam finder and metrics (decision 0061, step UV2), its flattener and
//! solver (UV3) and its packer (UV4), on research 28's bench shapes and on hand-made layouts
//! with known answers. One test binary, so the suite links once.

mod borders;
mod charts;
mod disc;
mod flatten;
mod hints;
mod metrics;
mod pack;
mod seams;
mod solve;
mod split;
mod symmetry;
mod threads;

use std::collections::BTreeMap;

use gs_geo::uv::{Charts, Steer, Surface, cut};
use gs_protocol::{AttrValues, GROUP, PlainMesh, corners};
use gs_testkit::bench_shapes;

/// The four bench shapes at about 2,000 faces each.
fn shapes() -> Vec<(&'static str, PlainMesh)> {
    bench_shapes::all(2000).into()
}

fn surface(mesh: &PlainMesh) -> Surface<'_> {
    Surface {
        points: &mesh.positions,
        faces: &mesh.faces,
    }
}

/// The cutter's charts, which must not be refused.
fn charts(mesh: &PlainMesh, steer: &Steer) -> Charts {
    cut(surface(mesh), steer).unwrap_or_else(|refusal| panic!("refused: {refusal}"))
}

/// A mesh's face groups.
fn groups(mesh: &PlainMesh) -> Vec<u32> {
    match mesh.attributes.get(GROUP).map(|a| &a.values) {
        Some(AttrValues::U32(groups)) => groups.clone(),
        _ => panic!("the mesh carries face groups"),
    }
}

/// Every face edge by its two points, lower first: the (face, corner) of each face along it, the
/// corner the edge starts from. Written here, apart from the cutter's own edge map.
fn edges(faces: &[[u32; 4]]) -> BTreeMap<[u32; 2], Vec<(usize, usize)>> {
    let mut out: BTreeMap<[u32; 2], Vec<(usize, usize)>> = BTreeMap::new();
    for (f, face) in faces.iter().enumerate() {
        let c = corners(face);
        for k in 0..c.len() {
            let (a, b) = (c[k], c[(k + 1) % c.len()]);
            out.entry([a.min(b), a.max(b)]).or_default().push((f, k));
        }
    }
    out
}

/// Whether face `f`'s edge from corner `k` is a seam in `seams`.
fn bit(seams: &[u8], f: usize, k: usize) -> bool {
    seams[f] >> k & 1 == 1
}

/// A flat open grid of `n` by `n` quads over x and z from 0 to 1, facing up.
fn grid(n: u32) -> PlainMesh {
    let mut positions = Vec::new();
    for i in 0..=n {
        for j in 0..=n {
            positions.push([i as f32 / n as f32, 0.0, j as f32 / n as f32]);
        }
    }
    let at = |i: u32, j: u32| i * (n + 1) + j;
    let mut faces = Vec::new();
    for i in 0..n {
        for j in 0..n {
            faces.push([at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}
