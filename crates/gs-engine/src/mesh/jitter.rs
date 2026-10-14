//! `mesh.jitter`, a temporary edit command that exists to exercise per-command random numbers
//! until real noise tools arrive. On a part with detail levels it jitters the shown level and
//! the next switch carries it: its offsets are drawn per point number, not read from the
//! position, so it is not a map the top could run (decision 0041, amended 2026-10-13).

use gs_protocol::{CommandError, Kind, Length, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, CommandRng, Ctx, Example, errors, register_command};

/// Move every vertex of a part by a random offset of up to `amount` along each axis.
///
/// Repeatable: the same seed gives the same offsets. Without a seed, one is derived from the
/// session seed and the command's sequence number, and written into the log. Temporary: real
/// noise tools will replace it.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshJitter {
    #[doc = crate::command::part_doc!("The part to jitter.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Largest offset along each axis: scene units, or a percentage such as "1%" of the part's
    /// bounding-box diagonal.
    pub amount: Length,
    /// Random seed. Vertex `i` moves by `amount * r` along axis `a`, where `r` is value
    /// `3 * i + a` of the seeded stream, in [-1, 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// What was jittered.
#[derive(Serialize, JsonSchema)]
pub struct Jittered {
    /// The part.
    pub part: String,
    /// The seed used.
    pub seed: u64,
    /// The largest offset, in scene units.
    pub amount: f32,
}

impl Command for MeshJitter {
    const NAME: &'static str = "mesh.jitter";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const RANDOM: bool = true;
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}}]"#;
    type Output = Jittered;

    fn examples() -> Vec<Example<Self>> {
        let rough = Self {
            part: None,
            amount: Length::Percent(1.0),
            seed: Some(7),
        };
        vec![
            Example::new("roughen a sphere by 1% of its size", rough)
                .after("primitive.sphere", json!({ "subdivisions": 6 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Jittered, CommandError> {
        let part = pinned(self.part)?;
        let seed =
            (self.seed).ok_or_else(|| errors::internal("the engine did not pin the seed"))?;
        let rng = CommandRng::from_seed(seed);
        let amount = cx.length(&part, self.amount)?;
        cx.noop_reason(format!(
            "offsets of up to {} left every vertex where it was",
            amount.scene
        ));
        // Offsets are drawn in the part's own space, scaled to it (decision 0029).
        let reach = amount.part;
        let mut store = cx.doc.write_part(&part)?;
        let (nodes, numbering) = (store.all_nodes(), store.numbering());
        store.write_positions(&nodes).each(|node| {
            let id = node.id();
            for (local, p) in (0..).zip(node.positions()) {
                let first = 3 * u64::from(numbering.index(id, local));
                let offset = |axis: u64| reach * rng.signed_unit_at(first + axis);
                *p += gs_core::Vec3::new(offset(0), offset(1), offset(2));
            }
        });
        Ok(Jittered {
            part,
            seed,
            amount: amount.scene,
        })
    }
}
register_command!(MeshJitter, "Jitter");
