//! What sculpting from the screen needs from gs-core: the camera's rays agree with its
//! projection, ray casts and nearest-point searches agree with brute force and stay near the
//! ray, and a stroke inside a step its caller opened joins that step. Differential tests against
//! slow `f64` references that share only data, plus work-counter limits (ARCHITECTURE 10.1).

mod camera;
mod deformer_handles;
mod inside_step;
mod navigation;
mod pick;
mod similarity;

use gs_core::{MeshStore, NodeConfig, Vec3};
use gs_protocol::PlainMesh;

/// `unwrap` for test helpers.
pub fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    r.unwrap_or_else(|e| panic!("{e:?}"))
}

/// Small nodes, so a test mesh of a few thousand vertices has dozens of nodes.
pub fn store(m: &PlainMesh) -> MeshStore {
    ok(MeshStore::from_plain(m, ok(NodeConfig::new(7, 64))))
}

/// A point in the cube [-s, s]^3 from a generator.
pub fn point(rng: &mut gs_testkit::SplitMix64, s: f32) -> Vec3 {
    let mut c = || gs_testkit::rng::signed_unit(rng.next_u64()) * s;
    Vec3::new(c(), c(), c())
}

pub fn wide(p: Vec3) -> [f64; 3] {
    p.to_array().map(f64::from)
}
