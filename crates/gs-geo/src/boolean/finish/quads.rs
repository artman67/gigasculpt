//! Kept quads (decision 0057, section 6): the output faces in triangle order, two output
//! triangles that are both halves of one untouched input quad joined back into it.

use gs_protocol::{NO_VERTEX, arity, face_triangles};

use super::{Layout, NOWHERE, Traced};

/// The output faces in triangle order with their sources, the two halves of an untouched input
/// quad joined back into it where the first of them stands.
pub(super) fn faces(
    lay: &Layout<'_>,
    traced: &[Traced],
    corners: &dyn Fn(usize) -> [u32; 3],
    positions: &[[f32; 3]],
) -> (Vec<[u32; 4]>, Vec<Option<u32>>) {
    // Untouched triangles by their source, as the group numbers triangles.
    let mut kept: Vec<u64> = (traced.iter().enumerate())
        .filter(|(_, tr)| tr.untouched())
        .map(|(t, tr)| (lay.tris[tr.leaf as usize] + u64::from(tr.src)) << 32 | t as u64)
        .collect();
    gs_par::sort_keys(&mut kept);
    let output_of = |global: u64| {
        let i = kept.partition_point(|&k| k >> 32 < global);
        (kept.get(i))
            .filter(|&&k| k >> 32 == global)
            .map(|&k| (k & 0xffff_ffff) as usize)
    };
    let mut joined = vec![false; traced.len()];
    let (mut faces, mut sources) = (Vec::new(), Vec::new());
    for (t, tr) in traced.iter().enumerate() {
        if joined[t] {
            continue;
        }
        if tr.leaf == NOWHERE {
            let [a, b, c] = corners(t);
            faces.push([a, b, c, NO_VERTEX]);
            sources.push(None);
            continue;
        }
        let (l, p) = (tr.leaf as usize, lay.group[tr.leaf as usize].operand);
        let face = p.tri_face[tr.src as usize];
        let partner = (tr.untouched() && arity(p.faces[face as usize]) == 4)
            .then(|| half_partner(&p.tri_face, tr.src as usize))
            .flatten()
            .and_then(|s| output_of(lay.tris[l] + s as u64))
            .filter(|&u| u > t && traced[u].untouched() && traced[u].as_wound() == tr.as_wound());
        let quad = partner.and_then(|u| join(lay, traced, corners, positions, (t, u)));
        if let (Some(q), Some(u)) = (quad, partner) {
            joined[u] = true;
            faces.push(q);
        } else {
            let [a, b, c] = corners(t);
            faces.push([a, b, c, NO_VERTEX]);
        }
        sources.push(Some(lay.faces[l] + face));
    }
    (faces, sources)
}

/// The other triangle of the quad triangle `s` is half of: its face's triangles are adjacent.
fn half_partner(tri_face: &[u32], s: usize) -> Option<usize> {
    let face = tri_face[s];
    if tri_face.get(s + 1) == Some(&face) {
        return Some(s + 1);
    }
    s.checked_sub(1).filter(|&r| tri_face[r] == face)
}

/// Untouched output triangles `t` and `u`, the two halves of one input quad, joined into it
/// with the output's winding, if the quad splits back into exactly them.
fn join(
    lay: &Layout<'_>,
    traced: &[Traced],
    corners: &dyn Fn(usize) -> [u32; 3],
    positions: &[[f32; 3]],
    (t, u): (usize, usize),
) -> Option<[u32; 4]> {
    let tr = traced[t];
    let p = lay.group[tr.leaf as usize].operand;
    // Each input corner of the two halves and the output point at its place.
    let mut image: Vec<(u32, u32)> = Vec::with_capacity(6);
    for w in [t, u] {
        let source = p.tris[traced[w].src as usize];
        let out = corners(w);
        for k in 0..3 {
            image.push((source[traced[w].corner[k] as usize], out[k]));
        }
    }
    let quad = p.faces[p.tri_face[tr.src as usize] as usize];
    let at = |v: u32| image.iter().find(|&&(i, _)| i == v).map(|&(_, o)| o);
    let [a, b, c, d] = [at(quad[0])?, at(quad[1])?, at(quad[2])?, at(quad[3])?];
    let out = if tr.as_wound() {
        [a, b, c, d]
    } else {
        [a, d, c, b]
    };
    let split = face_triangles(out, |k| positions[out[k] as usize]);
    let turned = |x: [u32; 3]| {
        let m = (0..3).min_by_key(|&k| x[k]).unwrap_or(0);
        [x[m], x[(m + 1) % 3], x[(m + 2) % 3]]
    };
    let mut made: Vec<[u32; 3]> = split.iter().map(|s| turned(s.map(|k| out[k]))).collect();
    let mut halves = vec![turned(corners(t)), turned(corners(u))];
    made.sort_unstable();
    halves.sort_unstable();
    (made == halves).then_some(out)
}
