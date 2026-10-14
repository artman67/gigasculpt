//! `pick.nearest`.

use gs_core::PosRead;
use gs_protocol::{CommandError, Cost, Kind, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{normal, numbered, parts};
use crate::{Command, Ctx, Example, errors, register_command};

/// The vertex nearest a point, across every part or in one.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickNearest {
    /// The point, in scene units.
    pub position: Vec3,
    /// The part to look in. Left out, every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// The nearest vertex.
#[derive(Serialize, JsonSchema)]
pub struct NearestVertex {
    pub part: String,
    /// Its index in the part's vertex numbering.
    pub vertex: u32,
    /// Where it is.
    pub position: Vec3,
    /// Its normal.
    pub normal: Vec3,
    /// How far it is from the point.
    pub distance: f32,
}

/// What the search found.
#[derive(Serialize, JsonSchema)]
pub struct Nearest {
    /// The nearest vertex; left out when there are no vertices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<NearestVertex>,
}

impl Command for PickNearest {
    const NAME: &'static str = "pick.nearest";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Nearest;

    fn examples() -> Vec<Example<Self>> {
        let above = Self {
            position: Vec3([0.1, 1.5, 0.0]),
            part: None,
        };
        vec![
            Example::new("the vertex nearest a point above a sphere", above)
                .after("primitive.sphere", json!({ "subdivisions": 8 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Nearest, CommandError> {
        let p = gs_core::Vec3::from_array(self.position.0);
        if !p.to_array().iter().all(|x| x.is_finite()) {
            return Err(errors::args_invalid("/position", "must be finite"));
        }
        let doc = cx.doc.document();
        // Each part is searched around the point mapped into its own space; a similarity keeps
        // which vertex is nearest, and distances compare in the scene (decision 0029).
        let near = (parts(doc, self.part.as_ref())?.into_iter())
            .filter_map(|(name, part)| {
                let t = part.transform();
                let (v, distance) = part.store().nearest_vertex(t.part_point(p)).found?;
                Some((t.length(distance), name, part, v))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let found = near.map(|(distance, name, part, v)| {
            let t = part.transform();
            let at = t.point(part.store().pos().get(v));
            NearestVertex {
                part: name.to_string(),
                vertex: numbered(part, v),
                position: Vec3(at.to_array()),
                normal: Vec3(t.direction(normal(part.store(), v)).to_array()),
                distance,
            }
        });
        Ok(Nearest { found })
    }
}
register_command!(PickNearest, "Pick nearest point");
