//! `mesh.stats`: counts, area and volume of each part.

use std::collections::BTreeMap;

use gs_protocol::{CommandError, Kind, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{corners, drawn_triangles, face_area, face_edges};
use crate::math::{cross, dot};
use crate::{Command, Ctx, Example, register_command};

/// Count the vertices, faces and edges of a part, or of every part, and measure its surface
/// area and volume.
///
/// Area and volume treat each quad as the two triangles it is drawn as (its shorter diagonal).
/// Volume is signed: positive for a closed surface wound counter-clockwise seen from
/// outside, and only meaningful for closed surfaces.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshStats {
    /// The part to measure. Omit to measure every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// Statistics for each measured part.
#[derive(Serialize, JsonSchema)]
pub struct StatsOutput {
    /// One entry per part, sorted by name.
    pub parts: Vec<PartStats>,
}

/// Statistics of one part.
#[derive(Serialize, JsonSchema)]
pub struct PartStats {
    /// Part name.
    pub part: String,
    /// Number of vertices.
    pub vertices: u64,
    /// Number of faces.
    pub faces: u64,
    /// Faces with three corners.
    pub triangles: u64,
    /// Faces with four corners.
    pub quads: u64,
    /// Distinct edges.
    pub edges: u64,
    /// Edges used by only one face: the rim of any hole or open border.
    pub boundary_edges: u64,
    /// Surface area in square scene units.
    pub area: f64,
    /// Signed volume in cubic scene units.
    pub volume: f64,
    /// Names of the attribute columns.
    pub attributes: Vec<String>,
}

impl Command for MeshStats {
    const NAME: &'static str = "mesh.stats";
    const KIND: Kind = Kind::Query;
    type Output = StatsOutput;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("measure every part", Self { part: None })
                .after("primitive.cube", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<StatsOutput, CommandError> {
        let parts = super::selected(&cx.doc, self.part.as_ref())?;
        Ok(StatsOutput {
            parts: parts
                .into_iter()
                .map(|(name, mesh)| stats(name, &mesh))
                .collect(),
        })
    }
}
register_command!(MeshStats, "Mesh statistics");

fn stats(part: String, mesh: &PlainMesh) -> PartStats {
    let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let (mut area, mut volume, mut triangles) = (0.0, 0.0, 0);
    for face in &mesh.faces {
        let corners = corners(face);
        triangles += u64::from(corners.len() == 3);
        for (a, b) in face_edges(corners) {
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
        let points: Vec<[f64; 3]> = (corners.iter())
            .map(|&v| mesh.positions[v as usize].map(f64::from))
            .collect();
        area += face_area(&points);
        for [a, b, c] in drawn_triangles(&points) {
            volume += dot(a, cross(b, c)) / 6.0;
        }
    }
    let faces = mesh.faces.len() as u64;
    PartStats {
        part,
        vertices: mesh.positions.len() as u64,
        faces,
        triangles,
        quads: faces - triangles,
        edges: edges.len() as u64,
        boundary_edges: edges.values().filter(|&&uses| uses == 1).count() as u64,
        area,
        volume,
        attributes: mesh.attributes.keys().cloned().collect(),
    }
}
