//! After meshing: find where each new vertex came from on the old surface, optionally move it
//! there (project), and optionally smooth how far the projection moved things (blur).

use super::grid::corner;
use super::grid::{Field, NONE};
use super::input::Soup;
use super::nets::Nets;
use crate::math::{V3, at, closest, dist2, sub};

/// A point on the input surface or on a cap over a hole: a triangle and barycentric weights,
/// or `tri == NONE` when no triangle is within reach (a film over a hole, on the generalized
/// winding number's route).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Source {
    pub tri: u32,
    pub bary: [f32; 3],
}

/// The closest of the candidate triangles to `p`, ties going to the lowest-numbered one.
pub(crate) fn nearest(soup: &Soup, p: V3, candidates: impl Iterator<Item = u32>) -> Source {
    let mut best = u64::MAX;
    for t in candidates.filter(|&t| t != NONE) {
        let tri = soup.corners(t);
        let d2 = dist2(p, at(&tri, closest(p, &tri)));
        best = best.min((u64::from(d2.to_bits()) << 32) + u64::from(t));
    }
    let tri = best as u32;
    let bary = if tri == NONE {
        [0.0; 3]
    } else {
        closest(p, &soup.corners(tri))
    };
    Source { tri, bary }
}

/// Steps a walk to the closest triangle may take.
const WALK: usize = 64;

/// Where each vertex came from: its closest point on the input or its caps. The search starts
/// from the triangles nearest to the cell's eight corners, then walks: it moves to whichever
/// triangle sharing a corner with the current one is closer, until none is. Triangles smaller
/// than a voxel are often nobody's nearest at a sample, so the start alone can be half a voxel
/// off.
/// Also returns the number of walk steps taken in all.
pub(crate) fn sources(field: &Field, soup: &Soup, nets: &Nets) -> (Vec<Source>, u64) {
    let (start, list) = rings(soup);
    let found = gs_par::map(nets.verts.len(), |v| {
        let (cell, p) = (nets.cells[v], nets.verts[v]);
        let near = (0..8).map(|k| {
            let d = corner(k);
            field.near(std::array::from_fn(|a| cell[a] + d[a]))
        });
        let mut best = nearest(soup, p, near);
        let mut steps = 0;
        for _ in 0..WALK {
            let Some(corners) = soup.tris.get(best.tri as usize) else {
                break;
            };
            let around = corners
                .iter()
                .flat_map(|&c| &list[start[c as usize] as usize..start[c as usize + 1] as usize]);
            let next = nearest(soup, p, around.copied());
            if next.tri == best.tri {
                break;
            }
            best = next;
            steps += 1;
        }
        (best, steps)
    });
    let steps = found.iter().map(|f| f.1).sum();
    (found.into_iter().map(|f| f.0).collect(), steps)
}

/// For each input vertex, the triangles that use it, as offsets into one list.
fn rings(soup: &Soup) -> (Vec<u32>, Vec<u32>) {
    let mut start = vec![0u32; soup.verts.len() + 1];
    for t in &soup.tris {
        for &v in t {
            start[v as usize + 1] += 1;
        }
    }
    for v in 1..start.len() {
        start[v] += start[v - 1];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; soup.tris.len() * 3];
    for (t, corners) in (0u32..).zip(&soup.tris) {
        for &v in corners {
            list[fill[v as usize] as usize] = t;
            fill[v as usize] += 1;
        }
    }
    (start, list)
}

/// Final vertex positions in grid units. With `project`, each vertex moves to its source
/// point; then `blur` (0 to 100) runs `ceil(blur / 4)` passes that replace each vertex's move
/// with half its own plus half the average of its neighbours' moves.
pub(crate) fn place(
    nets: &Nets,
    soup: &Soup,
    sources: &[Source],
    project: bool,
    blur: f32,
) -> Vec<V3> {
    if !project {
        return nets.verts.clone();
    }
    let mut moves: Vec<V3> = gs_par::map(nets.verts.len(), |v| {
        let s = sources[v];
        if s.tri == NONE {
            [0.0; 3]
        } else {
            sub(at(&soup.corners(s.tri), s.bary), nets.verts[v])
        }
    });
    let passes = (blur.clamp(0.0, 100.0) / 4.0).ceil() as usize;
    if passes >= 1 {
        let (start, list) = neighbours(nets.verts.len(), &nets.faces);
        for _ in 0..passes {
            moves = gs_par::map(moves.len(), |v| {
                let around = &list[start[v] as usize..start[v + 1] as usize];
                let mut mean = [0.0f32; 3];
                for &n in around {
                    for a in 0..3 {
                        mean[a] += moves[n as usize][a] / around.len() as f32;
                    }
                }
                std::array::from_fn(|a| 0.5 * moves[v][a] + 0.5 * mean[a])
            });
        }
    }
    gs_par::map(nets.verts.len(), |v| {
        std::array::from_fn(|a| nets.verts[v][a] + moves[v][a])
    })
}

/// For each vertex, the vertices it shares a face edge with, once per face (so twice each on a
/// closed manifold). Returned as offsets into one list.
fn neighbours(n: usize, faces: &[[u32; 4]]) -> (Vec<u32>, Vec<u32>) {
    let mut start = vec![0u32; n + 1];
    for f in faces {
        for &v in f {
            start[v as usize + 1] += 2;
        }
    }
    for v in 1..start.len() {
        start[v] += start[v - 1];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; start[n] as usize];
    for f in faces {
        for k in 0..4 {
            let v = f[k] as usize;
            list[fill[v] as usize] = f[(k + 1) % 4];
            list[fill[v] as usize + 1] = f[(k + 3) % 4];
            fill[v] += 2;
        }
    }
    (start, list)
}
