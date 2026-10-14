//! `level.reconstruct` (decision 0014, section 12).

use gs_core::{MeshError, Settings};
use gs_protocol::{CommandError, Cost, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{LevelState, refused, uv_corners};
use crate::command::PartArg;
use crate::document::{Part, pinned};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Build one detail level below the base (Reconstruct Subdiv): finds the coarser mesh the base
/// is one Catmull-Clark division of, so the old base becomes level 2 and every level's number
/// goes up by one, the active one too. A part with one level gets two. Run it again for another
/// level. The top level does not change. One undo step.
///
/// Refused, the part unchanged, when the base is not one division of a coarser mesh (it must be
/// all quads; the coarse corners are found as Blender's Unsubdivide finds them), when a
/// face-group border runs through a coarse face, or when the base has creases. Masks and hidden
/// faces are ignored and kept.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelReconstruct {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for LevelReconstruct {
    const NAME: &'static str = "level.reconstruct";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let rebuilt = Example::new(
            "rebuild the level a divided cube came from",
            Self { part: None },
        );
        vec![
            rebuilt
                .after("primitive.cube", json!({}))
                .after("level.subdivide", json!({}))
                .after("level.delete_lower", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let settings = Settings {
            creases: Vec::new(),
            pin_corners: false,
            corners: uv_corners(),
        };
        let levels = part.to_levels(&settings).map_err(|e| refused(&e))?;
        let props = part.props();
        cx.progress(None, format!("reconstructing a level below {name}'s base"));
        let next = levels.reconstruct().map_err(|e| match e {
            MeshError::Input(m) => CommandError {
                message: format!("part '{name}' has no level to rebuild below its base: {m}"),
                hint: Some(
                    "reconstruct works on a base made by dividing a coarser mesh; the message \
                     names the first piece that is not"
                        .into(),
                ),
                ..refused(&MeshError::Input(String::new()))
            },
            other => refused(&other),
        })?;
        let next = Part::leveled(next, props);
        let state = LevelState::of(&name, &next, 0);
        cx.doc.replace_part(&name, next);
        Ok(state)
    }
}
register_command!(LevelReconstruct, "Reconstruct lower level");
