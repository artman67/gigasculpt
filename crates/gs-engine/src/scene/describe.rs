//! `scene.describe`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::mesh::Bounds;
use crate::{Command, Ctx, Example, register_command};

/// A short plain-text summary of the session: parts with their sizes, history, checkpoints and
/// whether agents may edit. Run it first in any session.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneDescribe {}

/// The summary.
#[derive(Serialize, JsonSchema)]
pub struct Description {
    /// A few lines of plain text.
    pub text: String,
}

impl Command for SceneDescribe {
    const NAME: &'static str = "scene.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Description;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("describe a scene with one sphere", Self {})
                .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Description, CommandError> {
        let parts = cx.doc.document().list();
        let mut lines = vec![
            format!(
                "session '{}' rev {} seed {}",
                cx.config.session, cx.rev, cx.config.seed
            ),
            format!("parts ({}), as #id @place name:", parts.len()),
        ];
        for (place, (name, part)) in parts.iter().enumerate() {
            let mesh = part.world();
            let size = Bounds::of(&mesh.positions)
                .map(|b| format!(" size {:?} center {:?}", b.size.0, b.center.0));
            let (vertices, faces) = (mesh.positions.len(), mesh.faces.len());
            let id = part.props().id.unwrap_or_default();
            lines.push(format!(
                "  #{id} @{place} {name}: {vertices} vertices, {faces} faces{}",
                size.unwrap_or_default()
            ));
        }
        let history = cx.history.listing(&cx.doc);
        lines.push(format!(
            "history: {} undoable, {} redoable, {} of {} bytes",
            history.undoable, history.redoable, history.bytes, history.budget_bytes
        ));
        let names: Vec<&str> = (history.checkpoints.iter())
            .map(|(n, _)| n.as_str())
            .collect();
        lines.push(format!(
            "checkpoints: {} ({} more bytes)",
            names.join(", "),
            history.checkpoint_bytes
        ));
        lines.push(format!(
            "agents paused: {}, human gesture: {}",
            cx.control.paused, cx.control.gesture
        ));
        let text = lines.join("\n");
        Ok(Description { text })
    }
}
register_command!(SceneDescribe, "Describe scene");
