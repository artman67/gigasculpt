//! `level.subdivide`.

use gs_core::{Levels, Settings};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{LevelState, refused, uv_corners};
use crate::command::PartArg;
use crate::document::{Part, pinned};
use crate::errors::{self, error};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// A crease on an edge of the base, for the first subdivide.
#[derive(Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Crease {
    /// One end of the edge: a vertex index in the part's numbering.
    pub a: u32,
    /// The other end.
    pub b: u32,
    /// How many levels the edge stays sharp: 1 to 254, or 255 for always.
    #[schemars(range(min = 1, max = 255))]
    pub sharpness: u8,
}

/// Add a detail level on top of the part's highest level and make it active (Subdivide).
///
/// Each quad becomes four quads and each triangle three, by Catmull-Clark rules (or flat, with
/// `smooth` off: new points on the old faces, old points kept). Vertex i of every level keeps
/// its index and new points come after, so the new level shares its numbering with ZTL
/// files. Edits made on a lower level are carried up first. Creased edges stay sharp for
/// as many levels as their sharpness says; boundaries stay sharp always. One undo step.
///
/// UVs follow decision 0006's corner rule (edge midpoints, face centres), so island borders stay
/// put. With `smooth_uvs` on, a smooth step smooths the layout with the points inside each
/// island and keeps every island border's values (decision 0014, section 14); the level it was
/// made from keeps showing the layout it had.
///
/// A masked part divides whole, every face, and the mask comes up by division's rule (each new
/// point takes the mean of the points it is made from, settled on the mask's grid like every
/// stored mask value), so the masked area stays protected. A part with hidden faces is refused
/// for now: dividing only the visible region is not built yet, and dividing the hidden faces
/// too would change what was set aside.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelSubdivide {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Catmull-Clark smoothing; off adds a flat level.
    #[serde(default = "crate::command::yes")]
    pub smooth: bool,
    /// Smooth the UV layout with the points, island borders kept (needs `smooth`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub smooth_uvs: bool,
    /// Creases on edges of the part as it is, when it has no levels yet (they belong to the
    /// base): each edge's ends and its sharpness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creases: Vec<Crease>,
}

impl Command for LevelSubdivide {
    const NAME: &'static str = "level.subdivide";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let smooth = Self {
            part: None,
            smooth: true,
            smooth_uvs: false,
            creases: Vec::new(),
        };
        let flat = Self {
            part: Some(PartRef("cube".into())),
            smooth: false,
            smooth_uvs: false,
            creases: Vec::new(),
        };
        vec![
            Example::new("add a smooth level to a cube", smooth).after("primitive.cube", json!({})),
            Example::new("add a flat level, new points on the old faces", flat)
                .after("primitive.cube", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(mut self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        let name = pinned(self.part.take())?;
        let part = cx.doc.get_part(&name)?;
        let top = part.levels().map_or(part.store(), |l| l.top());
        if top.face_count() == 0 {
            let state = LevelState::of(&name, part, 0);
            cx.noop_reason(format!("part '{name}' has no faces to subdivide"));
            return Ok(state);
        }
        if part.levels().is_some() && !self.creases.is_empty() {
            return Err(CommandError {
                hint: Some("leave creases out, or delete the higher levels first".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!(
                        "part '{name}' has levels already; creases are set on its base when the first level is added"
                    ),
                )
            });
        }
        let props = part.props();
        let (levels, carried) = self.divided(cx, &name, part, 1, |_| {})?;
        let next = Part::leveled(levels, props);
        let state = LevelState::of(&name, &next, carried);
        cx.doc.replace_part(&name, next);
        Ok(state)
    }
}
register_command!(LevelSubdivide, "Subdivide");

impl LevelSubdivide {
    /// Part `name` (`part`) as a level stack with `times` levels added on top, each as this
    /// command adds one, and `after` run on the stack after each; with how many points were
    /// carried up first. `level.subdivide` adds one level, and a primitive made with levels
    /// adds them all (decision 0047, section 7).
    pub(crate) fn divided(
        &self,
        cx: &Ctx,
        name: &str,
        part: &Part,
        times: u32,
        mut after: impl FnMut(&mut Levels),
    ) -> Result<(Levels, usize), CommandError> {
        let settings = Settings {
            creases: (self.creases.iter())
                .map(|c| (c.a, c.b, c.sharpness))
                .collect(),
            pin_corners: false,
            corners: uv_corners(),
        };
        let mut levels = part
            .to_levels(&settings)
            .map_err(|e| errors::args_invalid("/creases", e.to_string()))?;
        // Pending edits on a lower level carry up first, so the check reads the top with them
        // and a mask just painted below comes up through the division (review 0004, m1).
        let carried = levels.set(levels.top_level()).map_err(|e| refused(&e))?;
        if levels.top().any_hidden() {
            return Err(CommandError {
                hint: Some(
                    "run visibility.show_all; from any level it acts on the top level, which \
                     this check reads"
                        .into(),
                ),
                ..error(
                    ErrorCode::Precondition,
                    format!(
                        "part '{name}' has hidden faces; dividing only the visible region is not \
                         built yet"
                    ),
                )
            });
        }
        for _ in 0..times {
            cx.progress(None, format!("subdividing {name}"));
            (levels.divide_uvs(self.smooth, self.smooth_uvs)).map_err(|e| refused(&e))?;
            after(&mut levels);
        }
        Ok((levels, carried.points))
    }
}
