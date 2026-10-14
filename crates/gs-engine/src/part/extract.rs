//! `part.extract` (decision 0028, section 4). The geometry is `gs_geo::parts::extract`.

use gs_geo::parts::{by_hidden, by_mask, extract};
use gs_protocol::{CommandError, ErrorCode, Kind, Length, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, half, pin_name, pinned_name, place_below, report, strip};
use crate::command::PartArg;
use crate::document::{Props, VertexOrder, pinned};
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Which faces `part.extract` pulls off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractFrom {
    /// The visible faces that are masked (the mean of their corners' mask at least
    /// `threshold`).
    Mask,
    /// Every visible face.
    Visible,
}

/// Add a thick shell made from some of a part's faces, as a new part directly below it.
///
/// The faces are moved out along their normals by `thickness` to make an outer layer, and the
/// inner layer and a rim close it into a solid. Face groups mark the three: 1 the outer layer,
/// 2 the inner one, 3 the rim. Colors and other columns carry; the new part starts with no
/// mask and every face showing. The part itself is unchanged and keeps its mask. A part's
/// active detail level is what is read.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartExtract {
    #[doc = crate::command::part_doc!("The part to extract from.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The faces to use. Left out: the masked ones if any visible face is masked, else every
    /// visible face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ExtractFrom>,
    /// 0 to 1: how masked a face must be to count as masked.
    #[serde(default = "half")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub threshold: f32,
    /// How thick the shell is: scene units, or a percentage such as "1%" of the part's
    /// bounding-box diagonal. Negative builds it inward.
    #[serde(default = "one_percent")]
    pub thickness: Length,
    /// Build the shell both ways, `thickness` out and `thickness` in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub double: bool,
    /// Name of the new part. Defaults to the part's name with -extract (the first that is
    /// free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
}

fn one_percent() -> Length {
    Length::Percent(1.0)
}

impl Command for PartExtract {
    const NAME: &'static str = "part.extract";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let strip = json!({ "name": "strip", "mesh": strip() });
        vec![
            Example::raw("extract the masked patch of a sphere", json!({}))
                .after("primitive.sphere", json!({}))
                .after(
                    "brush.stroke",
                    json!({
                        "brush": "Mask Pen",
                        "samples": [{ "position": [0.0, 0.0, 1.0] }],
                        "size": "25%",
                    }),
                ),
            Example::raw(
                "a shell 0.1 thick each way from every visible face",
                json!({ "from": "visible", "thickness": 0.1, "double": true, "name": "shell" }),
            )
            .after("part.create", strip),
            Example::raw("nothing is masked", json!({ "from": "mask" }))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let part = self.part.as_ref().map_or("part", |p| p.0.as_str());
        pin_name(cx, &mut self.name, &format!("{part}-extract"))?;
        if self.from.is_none() {
            let mesh = cx.doc.local(part)?;
            let masked = !region(&mesh, ExtractFrom::Mask, self.threshold).is_empty();
            self.from = Some(if masked {
                ExtractFrom::Mask
            } else {
                ExtractFrom::Visible
            });
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        let name = pinned_name(self.name)?;
        let from = self
            .from
            .ok_or_else(|| errors::internal("`from` was not pinned"))?;
        // In the part's own space: the shell keeps the part's transform (decision 0029).
        let mesh = cx.doc.local(&part)?;
        let faces = region(&mesh, from, self.threshold);
        if faces.is_empty() {
            let which = match from {
                ExtractFrom::Mask => "visible face is masked",
                ExtractFrom::Visible => "face is visible",
            };
            return Err(errors::unavailable(
                Unavailable::new(format!(
                    "no {which} on part '{part}', so there is nothing to extract"
                ))
                .fix("mask some faces or show them, or use another `from`"),
            ));
        }
        let thickness = cx.length(&part, self.thickness)?.part;
        let shell = extract(&mesh, &faces, thickness, self.double);
        let props = Props {
            transform: cx.doc.transform(&part)?,
            ..Props::default()
        };
        cx.doc.put_mesh(&name, shell, VertexOrder::AsGiven, props)?;
        place_below(cx, &part, std::slice::from_ref(&name))?;
        report(cx, &name)
    }
}
register_command!(PartExtract, "Extract");

/// The faces of `mesh` to extract, ascending: the visible ones, masked ones only `from` mask.
fn region(mesh: &PlainMesh, from: ExtractFrom, threshold: f32) -> Vec<u32> {
    let hidden = by_hidden(mesh);
    let masked = match from {
        ExtractFrom::Mask => by_mask(mesh, threshold),
        ExtractFrom::Visible => vec![1; hidden.len()],
    };
    (0..hidden.len() as u32)
        .filter(|&f| hidden[f as usize] == 0 && masked[f as usize] != 0)
        .collect()
}
