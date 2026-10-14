//! `level.describe`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// Describe a part's detail levels: how many, which is active, each level's size, whether the
/// active level holds edits not carried up yet, and how many are frozen.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelDescribe {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// One detail level.
#[derive(Serialize, JsonSchema)]
pub struct LevelSize {
    /// Its number, from 1 (the base).
    pub level: u32,
    pub vertices: u64,
    pub faces: u64,
    /// Made from the level below by Catmull-Clark smoothing (false: a flat level). True for the
    /// base.
    pub smooth: bool,
    /// Keeps its own UVs: the step above it smoothed them, so the levels above no longer hold
    /// them and it shows the layout it had. False for the top and a part with one level.
    pub own_uvs: bool,
}

/// A part's detail levels.
#[derive(Serialize, JsonSchema)]
pub struct LevelsDescribed {
    pub part: String,
    /// How many levels, the base included (1 for a part without levels).
    pub count: u32,
    /// The active level, from 1.
    pub active: u32,
    /// Every level, the base first.
    pub levels: Vec<LevelSize>,
    /// The active level holds edits not carried up to the top yet (they are carried at the
    /// next level change).
    pub pending: bool,
    /// How many levels `level.freeze` set aside, for `level.unfreeze` to bring back (0 when the
    /// part is not frozen; a frozen part shows only its base above).
    pub frozen: u32,
}

impl Command for LevelDescribe {
    const NAME: &'static str = "level.describe";
    const KIND: Kind = Kind::Query;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelsDescribed;

    fn examples() -> Vec<Example<Self>> {
        vec![super::subdivided_cube(
            "describe a subdivided cube's levels",
            Self { part: None },
        )]
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelsDescribed, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let size = |level: usize, vertices: usize, faces: usize, smooth: bool| LevelSize {
            level: level as u32 + 1,
            vertices: vertices as u64,
            faces: faces as u64,
            smooth,
            own_uvs: false,
        };
        let frozen = part.frozen().map_or(0, |r| r.count() as u32);
        Ok(match part.levels() {
            None => LevelsDescribed {
                part: name,
                count: 1,
                active: 1,
                levels: vec![size(
                    0,
                    part.store().vertex_count(),
                    part.store().face_count(),
                    true,
                )],
                pending: false,
                frozen,
            },
            Some(l) => LevelsDescribed {
                count: l.count() as u32,
                active: l.active() as u32 + 1,
                levels: (l.info().iter().enumerate())
                    .map(|(j, i)| LevelSize {
                        own_uvs: i.own_uvs,
                        ..size(j, i.vertices, i.faces, i.smooth)
                    })
                    .collect(),
                pending: l.pending(),
                frozen,
                part: name,
            },
        })
    }
}
register_command!(LevelDescribe, "Describe levels");
