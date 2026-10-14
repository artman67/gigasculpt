//! Vertex normals: the normalized sum of the area-weighted normals of every face using the
//! vertex, in ascending face id order. That fixed order makes the result bit-identical whichever
//! path computes it and however many threads run.
//!
//! Two paths. Interior vertices (all their faces stored in their own node, with only their own
//! node's vertices) are summed node-locally, face by face in storage order, which is ascending
//! face id. Seam vertices are summed through the cross-node readers.
//!
//! This is the one vertex normal of a mesh store: every store path (strokes, topology edits,
//! relaxation) calls it. Three other vertex normals sum the same face areas by their own rule,
//! so they do not give these bits: a detail level's frames (`levels/rules.rs`, in an order of
//! the corners rather than the face ids), an extracted shell's (gs-geo `parts/extract.rs`, in
//! f64 over the region's faces) and the renderer's for a plain mesh given none (gs-render
//! `mesh.rs`, over triangles, with a rule for faces folded back to back).

use std::sync::Arc;

use crate::config::{NodeConfig, NodeId, VertId};
use crate::math::{NO, Vec3, face_cross};
use crate::view::{PosRead, TopoRead};

/// The normal of vertex `v`, read across nodes.
#[inline]
pub fn vertex_normal<T: TopoRead, P: PosRead>(t: &T, p: &P, v: VertId) -> Vec3 {
    let mut acc = Vec3::ZERO;
    for &f in t.ring(v) {
        acc += face_cross(t.face(f), |u| p.get(u));
    }
    acc.normalized()
}

/// Normals of node `n`'s interior vertices from its own faces. Seam slots are left holding
/// whatever their own faces gave: every caller recomputes them with [`seam`] right after.
pub(crate) fn interior(
    cfg: NodeConfig,
    n: NodeId,
    pos: &[Vec3],
    nrm: &mut [Vec3],
    faces: &[[u32; 4]],
) {
    nrm.fill(Vec3::ZERO);
    // A corner is this node's when its id less the node's first id (wrapping, so lower nodes'
    // ids land far above) is below the capacity, a power of two: then it is the local slot,
    // and the corners' bitwise or is below the capacity only if each is. A dead slot is all
    // `NO`, whose node is past every real node, so it is skipped too.
    let (first, cap) = (cfg.vert(n, 0), cfg.capacity());
    let local = |u: u32| u.wrapping_sub(first);
    let at = |l: u32| pos[l as usize];
    for fc in faces {
        let [a, b, c, d] = fc.map(local);
        if fc[3] == NO {
            if (a | b | c) >= cap {
                continue;
            }
            let x = face_cross([a, b, c, NO], at);
            for l in [a, b, c] {
                nrm[l as usize] += x;
            }
        } else {
            if (a | b | c | d) >= cap {
                continue;
            }
            let x = face_cross([a, b, c, d], at);
            for l in [a, b, c, d] {
                nrm[l as usize] += x;
            }
        }
    }
    for x in nrm {
        *x = x.normalized();
    }
}

/// [`seam`] for a node whose own points did not move: `nrm` is written, and so copied when an
/// open history step shares it, only where a normal turns. A neighbour's move can leave every
/// seam normal of a node as it was.
pub(crate) fn seam_turned<T: TopoRead, P: PosRead>(
    cfg: NodeConfig,
    n: NodeId,
    nrm: &mut Arc<Vec<Vec3>>,
    seam: &[u32],
    t: &T,
    p: &P,
) {
    for &l in seam {
        let x = vertex_normal(t, p, cfg.vert(n, l));
        if nrm[l as usize].bits() != x.bits() {
            Arc::make_mut(nrm)[l as usize] = x;
        }
    }
}

/// Normals of node `n`'s seam vertices, read across nodes.
pub(crate) fn seam<T: TopoRead, P: PosRead>(
    cfg: NodeConfig,
    n: NodeId,
    nrm: &mut [Vec3],
    seam: &[u32],
    t: &T,
    p: &P,
) {
    for &l in seam {
        nrm[l as usize] = vertex_normal(t, p, cfg.vert(n, l));
    }
}
