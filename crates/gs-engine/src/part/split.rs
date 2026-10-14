//! `part.split` (decision 0028, section 4). The geometry is `gs_geo::parts`.

use gs_geo::parts::{by_groups, by_hidden, by_mask, by_shells, split};
use gs_protocol::{CommandError, ErrorCode, HIDE, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, half, place_below, report, strip};
use crate::command::PartArg;
use crate::document::{Props, VertexOrder, pinned};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// What `part.split` cuts a part by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitBy {
    /// The hidden faces from the visible ones.
    Hidden,
    /// One piece per face group.
    Groups,
    /// One piece per connected piece: faces sharing an edge stay together.
    Shells,
    /// The masked faces (the mean of their corners' mask at least `threshold`) from the rest.
    Mask,
}

/// Cut a part into several parts.
///
/// The first piece stays in the part's place with its name, id and properties: by hidden the
/// visible faces, by mask the unmasked ones, else the piece holding the part's first face. The
/// others become new parts directly below it, named `<part>-hidden`, `-masked`, `-group-<id>`
/// or `-shell-<n>` (or the first free name after those). With `keep` the part stays whole and
/// every piece is a new part below it, the first named `-visible`, `-unmasked`, `-group-<id>`
/// or `-shell-1`. Every column carries, except that both pieces of a split by hidden show every
/// face. A part that is all one piece is left alone; a part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartSplit {
    #[doc = crate::command::part_doc!("The part to split.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// What to cut it by.
    pub by: SplitBy,
    /// 0 to 1: how masked a face must be to count as masked (by mask only).
    #[serde(default = "half")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub threshold: f32,
    /// Leave the part whole and add every piece as a new part.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

/// The parts a split, merge or extract made or changed.
#[derive(Serialize, JsonSchema)]
pub struct Pieces {
    /// Each part, in list order: the part that was split first unless `keep` was given.
    pub parts: Vec<NewPart>,
}

impl Command for PartSplit {
    const NAME: &'static str = "part.split";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = Pieces;

    fn examples() -> Vec<Example<Self>> {
        let strip = json!({ "name": "strip", "mesh": strip() });
        vec![
            Example::raw("split a cube into its six sides", json!({ "by": "groups" }))
                .after("primitive.cube", json!({}))
                .after("group.make", json!({ "maker": { "by": "normals" } })),
            Example::raw("split off the hidden face", json!({ "by": "hidden" }))
                .after("part.create", strip.clone()),
            Example::raw(
                "add the masked and unmasked faces as two new parts, keeping the strip",
                json!({ "part": "strip", "by": "mask", "threshold": 0.25, "keep": true }),
            )
            .after("part.create", strip),
            Example::raw(
                "a part with detail levels is refused",
                json!({ "by": "shells" }),
            )
            .after("primitive.cube", json!({}))
            .after("level.subdivide", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        crate::level::check_one_level(cx, self.part.as_ref(), "a split", false)
    }

    fn run(self, cx: &mut Ctx) -> Result<Pieces, CommandError> {
        let part = pinned(self.part)?;
        cx.doc.single_level(&part, "a split")?;
        // In the part's own space: every piece keeps the part's transform (decision 0029).
        let mesh = cx.doc.local(&part)?;
        let labels = match self.by {
            SplitBy::Hidden => by_hidden(&mesh),
            SplitBy::Groups => by_groups(&mesh),
            SplitBy::Shells => by_shells(&mesh),
            SplitBy::Mask => by_mask(&mesh, self.threshold),
        };
        let mut pieces = split(&mesh, &labels);
        if matches!(self.by, SplitBy::Hidden | SplitBy::Mask) {
            // The visible or unmasked faces (label 0) stay.
            pieces.sort_by_key(|piece| piece.label);
        }
        if pieces.len() < 2 {
            cx.noop_reason(format!("part '{part}' is all one piece"));
            return Ok(Pieces {
                parts: vec![report(cx, &part)?],
            });
        }
        let suffix = |label: u32| match self.by {
            SplitBy::Hidden => ["visible", "hidden"][usize::from(label != 0)].to_string(),
            SplitBy::Mask => ["unmasked", "masked"][usize::from(label != 0)].to_string(),
            SplitBy::Groups => format!("group-{label}"),
            SplitBy::Shells => format!("shell-{}", u64::from(label) + 1),
        };
        let props = cx.doc.get_part(&part)?.props();
        let (mut all, mut new) = (Vec::new(), Vec::new());
        for (k, piece) in pieces.into_iter().enumerate() {
            let mut mesh = piece.mesh;
            if self.by == SplitBy::Hidden {
                mesh.attributes.remove(HIDE);
            }
            if k == 0 && !self.keep {
                cx.doc.put_mesh(&part, mesh, VertexOrder::AsGiven, props)?;
                all.push(part.clone());
                continue;
            }
            let name = cx.doc.free_name(&format!("{part}-{}", suffix(piece.label)));
            let props = Props {
                id: None,
                start: false,
                ..props
            };
            cx.doc.put_mesh(&name, mesh, VertexOrder::AsGiven, props)?;
            all.push(name.clone());
            new.push(name);
        }
        place_below(cx, &part, &new)?;
        let parts = all.iter().map(|name| report(cx, name));
        Ok(Pieces {
            parts: parts.collect::<Result<_, _>>()?,
        })
    }
}
register_command!(PartSplit, "Split part");
