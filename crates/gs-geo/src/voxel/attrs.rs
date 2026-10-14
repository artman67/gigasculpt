//! Where a remesh's new surface came from on the old one (decision 0037, section 3): each new
//! vertex's source triangle and weights, each new face's nearest old face. gs-geo knows no
//! column kinds, so it carries no columns; the engine carries each by its kind from these.

use super::finish::{Source, nearest};
use super::grid::NONE;
use super::input::Soup;
use crate::math::V3;

/// Where each vertex and face of a remeshed surface came from on the input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    /// For each new vertex: the input vertices of the triangle under its source point (quads
    /// split as they are drawn, `gs_protocol::face_triangles`) and their weights there, each 0 or more and summing to
    /// about 1; `None` on surface made to close a hole.
    pub points: Vec<Option<([u32; 3], [f32; 3])>>,
    /// For each new face: the input face nearest its centre among its corners' sources; `None`
    /// on surface made to close a hole.
    pub faces: Vec<Option<u32>>,
}

/// The sources of every new vertex and face. `sources` are the vertices' closest points,
/// `faces` the new faces and `positions` their vertices, in grid units.
pub(crate) fn find(
    soup: &Soup,
    sources: &[Source],
    faces: &[[u32; 4]],
    positions: &[V3],
) -> Sources {
    let input_face = |tri: u32| soup.face.get(tri as usize).copied().filter(|&f| f != NONE);
    let points = gs_par::map(sources.len(), |v| {
        let s = sources[v];
        input_face(s.tri).map(|_| (soup.tris[s.tri as usize], s.bary))
    });
    let faces = gs_par::map(faces.len(), |f| {
        let corners = faces[f].map(|v| positions[v as usize]);
        let centre = std::array::from_fn(|a| corners.iter().map(|c| c[a]).sum::<f32>() / 4.0);
        let tris = faces[f].iter().map(|&v| sources[v as usize].tri);
        input_face(nearest(soup, centre, tris).tri)
    });
    Sources { points, faces }
}
