//! Picking (ARCHITECTURE 7.5): `pick.ray`, what a ray or an image point hits, `pick.color`, the
//! color stored there, and `pick.nearest`, the vertex nearest a point. They search gs-core's node
//! tree (`gs_core::pick`, the one home of ray casting), so they cost what lies near the ray or
//! point, and they aim through the same cameras as `brush.stroke` and `view.snapshot`.

mod color;
mod nearest;
mod ray;

pub(crate) use color::paint_at;
pub(crate) use ray::nearest_corner;

use gs_core::{Hit, MeshStore, VertId};
use gs_protocol::{CommandError, PartRef};

use crate::document::{Document, Part};

/// The parts to search: the one named, shown or not, or every part that shows (decision 0028).
fn parts<'d>(
    doc: &'d Document,
    part: Option<&PartRef>,
) -> Result<Vec<(&'d str, &'d Part)>, CommandError> {
    match part {
        Some(PartRef(r)) => {
            let found = doc.resolve(r).and_then(|name| Some((name, doc.get(name)?)));
            let missing = || crate::errors::not_found("part", r, doc.names());
            Ok(vec![found.ok_or_else(missing)?])
        }
        None => Ok((doc.shown()).map(|(n, p)| (n.as_str(), p)).collect()),
    }
}

/// The first surface among `parts` along the scene ray from `o` along unit `d`: its scene
/// distance, the hit in its part's own space, and the part. Each part is searched along the ray
/// mapped into its own space, and hits compare by scene distance (decision 0029, section 4).
pub(crate) fn first_hit<'d>(
    parts: &[(&'d str, &'d Part)],
    o: gs_core::Vec3,
    d: gs_core::Vec3,
) -> Option<(f32, Hit, &'d str, &'d Part)> {
    let hits = (parts.iter()).filter_map(|&(name, part)| {
        let (distance, hit) = part.hit(o, d)?;
        Some((distance, hit, name, part))
    });
    hits.min_by(|a, b| a.0.total_cmp(&b.0))
}

/// A store vertex in its part's numbering.
fn numbered(part: &Part, v: VertId) -> u32 {
    let cfg = part.store().config();
    part.numbering().index(cfg.node_of(v), cfg.local_of(v))
}

/// The vertex normal of a store vertex, in its part's own space.
fn normal(store: &MeshStore, v: VertId) -> gs_core::Vec3 {
    use gs_core::PosRead;
    store.nrm().get(v)
}
