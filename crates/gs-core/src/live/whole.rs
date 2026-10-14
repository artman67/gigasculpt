//! The pass over a whole part or a selected region, again until it changes nothing: what
//! `mesh.refine` runs (decision 0021, L4), Blender's Detail Flood Fill.
//!
//! Each pass is [`super::refine`]'s with one sphere around the whole part standing for the dab,
//! so nothing about a brush limits it, and the region is the visible faces the selector picks,
//! found again before each pass (and after its node cuts): faces a pass makes count by their
//! own mask and group. Unlike a dab's pass it also splits every quad no split needed, so the
//! region comes out all triangles. A pass that changes nothing would change
//! nothing again, so it ends the run. The nodes are then maintained as at the end of a
//! refining stroke.

use crate::config::{FaceId, NodeId};
use crate::math::{Aabb, NO, Vec3};
use crate::par::merge;
use crate::select::Selector;
use crate::store::MeshStore;

use super::{LiveDetail, LiveReport, Region, run_pass};

/// Run [`super::refine`]'s pass over every visible face `sel` picks (every visible face when
/// `None`) until a pass changes nothing or `cap` passes ran, then maintain the nodes as a
/// refining stroke's end does. The part's bounding sphere stands for the dab, so a relative
/// target is that many times its radius and no target is under [`super::LEAST_SIZE`] of it;
/// `ld`'s budget bounds each pass. Returns the nodes written, ascending, and whether a pass
/// came to rest.
pub fn refine_all(
    store: &mut MeshStore,
    sel: Option<&Selector>,
    ld: &LiveDetail,
    mirrored: [bool; 3],
    cap: u32,
    report: &mut LiveReport,
) -> (Vec<NodeId>, bool) {
    let Some(sphere) = enclosing(store) else {
        return (Vec::new(), true);
    };
    let targets = ld.lengths(sphere.1);
    let spheres = [sphere];
    let (mut written, mut rest) = (Vec::new(), false);
    for _ in 0..cap {
        let mut pass = LiveReport::default();
        let w = run_pass(store, &spheres, targets, ld, mirrored, &mut pass, |s| {
            picked(s, sel)
        });
        written = merge(&written, &w);
        report.add(&pass);
        if !pass.changed() {
            rest = true;
            break;
        }
    }
    (merge(&written, &store.maintain_nodes()), rest)
}

/// The sphere around every live point of `store`, padded far past rounding, centered on its
/// box: a part symmetric across a plane through the origin gets a sphere symmetric too.
/// `None` for a store without points.
fn enclosing(store: &MeshStore) -> Option<(Vec3, f32)> {
    let mut b = Aabb::EMPTY;
    for n in 0..store.node_count() as NodeId {
        let nb = store.bounds(n);
        if nb.lo.x <= nb.hi.x {
            b = b.union(nb);
        }
    }
    if b.lo.x > b.hi.x {
        return None;
    }
    let (c, r) = ((b.lo + b.hi) * 0.5, (b.hi - b.lo).len2().sqrt() * 0.5);
    Some((c, r + r / 4096.0 + f32::MIN_POSITIVE))
}

/// The visible faces `sel` picks (every visible face when `None`), ascending; a selector's
/// region holds its corners, so no point off it moves.
fn picked(store: &MeshStore, sel: Option<&Selector>) -> Region {
    let set = sel.map(|s| store.select(s));
    let (cfg, vis) = (store.config(), store.visible());
    let faces: Vec<FaceId> = (0..store.node_count() as NodeId)
        .flat_map(|n| {
            let node = store.node(n);
            (0..node.faces.len() as u32)
                .filter(move |&j| node.faces[j as usize][0] != NO)
                .map(move |j| node.first_face + j)
        })
        .filter(|&f| vis.face(f) && set.as_ref().is_none_or(|s| s.contains(cfg, f)))
        .collect();
    let mut region = Region::of(store, faces);
    region.whole = true;
    if sel.is_some() {
        region.held(store)
    } else {
        region
    }
}
