//! Detail levels (decision 0014). Kinds from ARCHITECTURE 10.1: differential tests against a
//! slow f64 Catmull-Clark written from the textbook rules, round trips (a lower level is the
//! top's first points, a switch with no edit changes no bit), locality (points an edit cannot
//! reach keep their bits), metamorphic relations (a rigid move of a coarse level moves the top
//! rigidly), determinism across thread counts, work counters (no switch or divide sorts), and
//! the store invariant oracle on every level.

mod carry;
mod divide;
mod file;
mod held;
mod project;
mod reference;
mod region;
mod switch;
mod uvs;
mod write_uvs;

use std::sync::Arc;

use gs_core::{Levels, MeshStore, NodeConfig, Numbering, PosRead, Settings, Vec3};
use gs_protocol::PlainMesh;

use crate::common::{self, ok};

/// A part's level stack over `mesh`, numbered as given, with small nodes so tests have many.
pub fn stack(mesh: &PlainMesh, settings: &Settings) -> Levels {
    stack_with(mesh, settings, common::small_nodes())
}

pub fn stack_with(mesh: &PlainMesh, settings: &Settings, cfg: NodeConfig) -> Levels {
    let (store, placed) = ok(MeshStore::import(mesh.clone(), cfg));
    let numbering = Numbering::from_placement(&store, placed);
    ok(Levels::new(store, Arc::new(numbering), settings))
}

/// The active level's positions in canonical order.
pub fn positions(l: &Levels) -> Vec<Vec3> {
    let num = l.numbering();
    let p = l.store().pos();
    (0..num.vertex_count() as u32)
        .map(|c| p.get(num.id(c)))
        .collect()
}

/// The top's positions in canonical order.
pub fn top_positions(l: &Levels) -> Vec<Vec3> {
    let num = l.level_numbering(l.top_level());
    let p = l.top().pos();
    (0..num.vertex_count() as u32)
        .map(|c| p.get(num.id(c)))
        .collect()
}

/// Bit patterns, for exact comparison.
pub fn bits(p: &[Vec3]) -> Vec<[u32; 3]> {
    p.iter().map(|q| q.bits()).collect()
}

/// The active level's faces in canonical order, as canonical indices.
pub fn faces(l: &Levels) -> Vec<[u32; 4]> {
    l.numbering().canonical_faces(l.store())
}
