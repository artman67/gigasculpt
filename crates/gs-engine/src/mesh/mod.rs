//! Mesh observation commands (stats, bounds, hash, validate, sample, diff), two temporary edit commands
//! (translate, jitter), whole-part rebuilds (reorder, voxel remesh, quad remesh, decimate,
//! refine, symmetrize, flip), Project Detail, and the mesh helpers they share.

mod bounds;
mod decimate;
mod diff;
mod flip;
mod hash;
mod jitter;
mod project_detail;
mod quad_remesh;
mod refine;
mod reorder;
mod sample;
pub(crate) mod smooth;
mod stats;
mod symmetrize;
mod translate;
mod validate;
mod voxel_mode;
mod voxel_remesh;

use std::sync::Arc;

use gs_protocol::{CommandError, NO_VERTEX, PartRef, PlainMesh, face_triangles};

pub(crate) use bounds::{Bounds, bounds_of, diagonal, per_axis, resolve_length, store_diagonal};
pub(crate) use hash::{digest, document_digest};
pub(crate) use validate::{problems, value_problems};
pub use voxel_mode::VoxelSettings;
pub(crate) use voxel_mode::session_closing;
pub(crate) use voxel_remesh::{
    FIRST_RESOLUTION, carry as carry_columns, carry_uv_corners, refusal as remesh_refusal,
};

use crate::document::DocTxn;
pub(crate) use crate::level::drop_uvs;
use crate::math::{cross, dot, sub};

pub(crate) use gs_protocol::corners;

/// The triangles of a face with corners `points` (three or four, a part's f32 positions in
/// f64), split as the face is drawn (decision 0040, section 2). Area and volume use the same
/// split.
pub(crate) fn drawn_triangles(points: &[[f64; 3]]) -> impl Iterator<Item = [[f64; 3]; 3]> + '_ {
    let face = [0, 1, 2, if points.len() == 4 { 3 } else { NO_VERTEX }];
    let tris = face_triangles(face, |k| points[k].map(|c| c as f32));
    tris.iter().map(|t| t.map(|k| points[k]))
}

/// The area of a face with corners `points`.
pub(crate) fn face_area(points: &[[f64; 3]]) -> f64 {
    let area = |[a, b, c]: [[f64; 3]; 3]| {
        let n = cross(sub(b, a), sub(c, a));
        dot(n, n).sqrt() / 2.0
    };
    drawn_triangles(points).map(area).sum()
}

/// The edges around a face, in winding order: (0, 1), (1, 2), ... and back to the first corner.
pub(crate) fn face_edges(corners: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    corners
        .iter()
        .copied()
        .zip(corners.iter().copied().cycle().skip(1))
}

/// The parts a query looks at, by name: the one `part` refers to, or every part when it is
/// omitted.
pub(crate) fn selected(
    doc: &DocTxn,
    part: Option<&PartRef>,
) -> Result<Vec<(String, Arc<PlainMesh>)>, CommandError> {
    match part {
        Some(PartRef(part)) => {
            let mesh = doc.part(part)?;
            Ok(vec![(
                doc.resolve(part).unwrap_or_else(|| part.clone()),
                mesh,
            )])
        }
        None => Ok(doc.document().views().into_iter().collect()),
    }
}
