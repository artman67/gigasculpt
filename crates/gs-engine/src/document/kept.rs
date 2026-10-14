//! Voxel Remesh's kept cell size as JSON (gs-geo has no serde).

use gs_geo::voxel::Kept;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize)]
struct Fields {
    voxel: f64,
    limit: u32,
}

#[expect(
    clippy::ref_option,
    reason = "serde's `with` passes the field by reference"
)]
pub fn serialize<S: Serializer>(k: &Option<Kept>, s: S) -> Result<S::Ok, S::Error> {
    k.map(|k| Fields {
        voxel: k.voxel,
        limit: k.limit,
    })
    .serialize(s)
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Kept>, D::Error> {
    let fields = Option::<Fields>::deserialize(d)?;
    Ok(fields.map(|f| Kept {
        voxel: f.voxel,
        limit: f.limit,
    }))
}
