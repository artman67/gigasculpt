//! What every part must stay, checked on every change, so f32 can hold it: coordinates in
//! range and the part near enough to the origin.
//!
//! The policy for f32's limits: a change is refused, and the scene left exactly as it was,
//! when it would leave a part with an attribute value that is not a finite number (a NaN mask
//! turns the next stroke over it into a dead vertex, review 0002 M5), or
//!
//! - with a coordinate past ±[`MAX_COORD`], infinite or NaN. Squared distances between points
//!   within that range still fit in an f32, so brushes and measurements keep working; or
//! - farther from the origin than [`MAX_REACH`] times its own size (its bounding-box diagonal).
//!   f32 keeps 24 significant bits, so within that reach rounding moves no vertex by more than
//!   2^-10 of the part's size, about 0.1%. Moving a part much farther flattens or collapses it;
//!   tiny parts stay fine anywhere near the scene they belong to.
//!
//! A part with a transform (decision 0029) keeps its points in its own space; its scene
//! points, the ones `part.get` shows, obey the same two rules (see [`check_placed`]).
//!
//! Faces of zero area are legal (review 0002, M2): sculpting makes them on purpose (Clip Curve
//! and flattening brushes press faces into a plane), imported files contain them, and every
//! consumer handles them (normals use the shared near-zero rule, voxel remesh and the renderer
//! skip their area). `mesh.validate` still reports them.

use gs_core::transform::Similarity;
use gs_core::{Aabb, AttrVec, MeshStore, NodeId, PosRead};
use gs_protocol::{CommandError, ErrorCode, PlainMesh};

use super::Changed;
use crate::errors::error;
use crate::math::{dot, sub};

/// The largest coordinate a part may have, in scene units. Three squared differences of
/// coordinates this large add up to about 1.2e37, inside f32's range of 3.4e38.
const MAX_COORD: f32 = 1e18;

/// How far from the origin a part may be, in multiples of its size: 2^14.
const MAX_REACH: f64 = 16384.0;

/// Finite and within ±`MAX_COORD` on every axis.
fn in_range(p: [f32; 3]) -> bool {
    p.iter().all(|c| c.abs() <= MAX_COORD)
}

fn refused(message: String, hint: &str) -> CommandError {
    CommandError {
        hint: Some(hint.into()),
        ..error(ErrorCode::ArgsInvalid, message)
    }
}

fn wide(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

/// The verdict on a change to part `part` that left its coordinates `in_range` (or not) and
/// its bounds at `bounds`.
fn verdict(part: &str, in_range: bool, bounds: Aabb) -> Result<(), CommandError> {
    if !in_range {
        let message = format!(
            "the result would put part '{part}' out of range: coordinates must be finite and \
             within ±{MAX_COORD:e}"
        );
        return Err(refused(message, "use smaller values"));
    }
    let (lo, hi) = (wide(bounds.lo.to_array()), wide(bounds.hi.to_array()));
    let reach = (lo.iter().chain(&hi)).fold(0.0, |far: f64, c| far.max(c.abs()));
    let size = dot(sub(hi, lo), sub(hi, lo)).sqrt();
    if reach > MAX_REACH * size {
        let message = format!(
            "the result would put part '{part}' {reach} from the origin, more than {MAX_REACH} \
             times its size ({size}), where f32 rounding would flatten its shape"
        );
        return Err(refused(
            message,
            "move it closer to the origin, or scale it up first",
        ));
    }
    Ok(())
}

/// Checks a whole part, given as plain mesh data with valid indices.
pub(crate) fn check_mesh(mesh: &PlainMesh, part: &str) -> Result<(), CommandError> {
    let mut bounds = Aabb::EMPTY;
    for &p in &mesh.positions {
        bounds.grow(gs_core::Vec3::from_array(p));
    }
    verdict(part, mesh.positions.iter().all(|&p| in_range(p)), bounds)
}

/// The part's bounds, from its nodes'.
fn bounds(store: &MeshStore) -> Aabb {
    (0..store.node_count() as NodeId)
        .map(|n| store.bounds(n))
        .fold(Aabb::EMPTY, Aabb::union)
}

fn not_finite(part: &str, column: &str) -> CommandError {
    refused(
        format!("the result would give part '{part}' a {column} value that is not a finite number"),
        "give every value as a finite number",
    )
}

/// Checks what an in-place edit changed: the vertices it moved, the values it wrote, and the
/// part's bounds afterwards.
pub(crate) fn check_moved(
    store: &MeshStore,
    changed: &Changed,
    part: &str,
) -> Result<(), CommandError> {
    if let Some(column) = &changed.not_finite {
        return Err(not_finite(part, column));
    }
    let pos = store.pos();
    let moved = &changed.moved;
    verdict(
        part,
        moved.iter().all(|&v| in_range(pos.get(v).to_array())),
        bounds(store),
    )
}

/// Checks a whole part as its store holds it: every live position and every attribute value.
/// For a part a call added or replaced, without exporting it.
pub(crate) fn check_store(store: &MeshStore, part: &str) -> Result<(), CommandError> {
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        for d in store.attrs() {
            let values = store.attr_id(&d.name).and_then(|id| node.attr(id));
            if let Some(AttrVec::F32(v)) = values
                && v.iter().any(|x| !x.is_finite())
            {
                return Err(not_finite(part, &d.name));
            }
        }
    }
    let live = (0..store.node_count() as NodeId).flat_map(|n| store.node(n).pos.iter());
    let ranged = live
        .filter(|p| !p.is_dead())
        .all(|p| in_range(p.to_array()));
    verdict(part, ranged, bounds(store))
}

/// Checks where a part placed by `t` puts its points in the scene: the box of its node bounds
/// mapped, which holds every scene point, must be in range and within reach. The box of a
/// turned part is at most √3 times larger than its points' own, so this errs toward refusing,
/// by that much, a part turned far from the origin. Nothing to check at the identity, where
/// the other checks already read scene points.
pub(crate) fn check_placed(
    store: &MeshStore,
    t: &Similarity,
    part: &str,
) -> Result<(), CommandError> {
    if t.is_identity() {
        return Ok(());
    }
    let b = t.bounds(bounds(store));
    if b == Aabb::EMPTY {
        return Ok(());
    }
    verdict(
        part,
        in_range(b.lo.to_array()) && in_range(b.hi.to_array()),
        b,
    )
}
