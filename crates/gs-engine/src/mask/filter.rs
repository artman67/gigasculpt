//! `mask.filter`.

use gs_core::mask::{MaskFilter, PointMap};
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::MaskReport;
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, errors, register_command};

/// A mask filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FilterOp {
    /// Soften the mask's edges: each point moves half way to the mean of its neighbours. Each
    /// pass spreads it by about one ring of points, so it is narrower on a dense part.
    Blur,
    /// Harden soft edges, roughly undoing a blur: each point moves away from the mean of its
    /// neighbours (`m + strength * (m - mean)`, clamped). A uniform mask does not change.
    Sharpen,
    /// Spread the mask outward by about one ring with a soft edge; nothing masked loses mask.
    Grow,
    /// Pull the mask's edge inward by about one ring; nothing free gains mask.
    Shrink,
    /// Raise partial values toward fully masked without spreading the mask: each point moves
    /// toward `sqrt(m)`, so 0.25 becomes 0.5 and 0 and 1 stay.
    Boost,
    /// Lower partial values toward free without shrinking the mask: each point moves toward
    /// `m * m`, so 0.5 becomes 0.25 and 0 and 1 stay.
    Dilute,
    /// Stretch the mask's range to 0..1: the lowest mask of the visible points becomes 0, the
    /// highest 1, in order. A flat mask stays.
    Normalize,
}

/// Filter the mask over the surface: blur, sharpen, grow or shrink it, one ring of points per
/// pass, or boost, dilute or normalize its values point by point.
///
/// Neighbours are taken across visible faces only, and hidden points keep their mask. Boost,
/// dilute and normalize act on the top level from any detail level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskFilterCmd {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The filter.
    pub op: FilterOp,
    /// How many passes, 1 to 100. Left out, 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub passes: Option<u32>,
    /// How far each pass goes, 0 to 1. Left out, 1 (a full pass).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub strength: Option<f32>,
}

impl Command for MaskFilterCmd {
    const NAME: &'static str = "mask.filter";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = MaskReport;

    fn examples() -> Vec<Example<Self>> {
        let masked_front = json!({
            "brush": "Mask Pen",
            "samples": [{ "position": [0.0, 0.0, 1.0] }],
            "size": "25%",
        });
        let filter = |op, passes| Self {
            part: None,
            op,
            passes,
            strength: None,
        };
        vec![
            Example::new(
                "blur a mask painted on the front",
                filter(FilterOp::Blur, Some(3)),
            )
            .after("primitive.sphere", json!({ "subdivisions": 8 }))
            .after("brush.stroke", masked_front.clone()),
            Example::new("sharpen it back", filter(FilterOp::Sharpen, None))
                .after("primitive.sphere", json!({ "subdivisions": 8 }))
                .after("brush.stroke", masked_front.clone())
                .after("mask.filter", json!({ "op": "blur", "passes": 2 })),
            Example::new("grow it by two rings", filter(FilterOp::Grow, Some(2)))
                .after("primitive.sphere", json!({ "subdivisions": 8 }))
                .after("brush.stroke", masked_front.clone()),
            Example::new("boost a blurred mask", filter(FilterOp::Boost, Some(2)))
                .after("primitive.sphere", json!({ "subdivisions": 8 }))
                .after("brush.stroke", masked_front.clone())
                .after("mask.filter", json!({ "op": "blur", "passes": 2 })),
            Example::new("stretch a faint mask to the full range", {
                let mut f = filter(FilterOp::Normalize, None);
                f.strength = Some(0.5);
                f
            })
            .after("primitive.sphere", json!({ "subdivisions": 8 }))
            .after("brush.stroke", masked_front)
            .after("mask.filter", json!({ "op": "dilute", "strength": 0.5 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaskReport, CommandError> {
        let part = pinned(self.part)?;
        let passes = self.passes.unwrap_or(1);
        let strength = self.strength.unwrap_or(1.0);
        if !(1..=100).contains(&passes) {
            return Err(errors::args_invalid(
                "/passes",
                "passes must be from 1 to 100",
            ));
        }
        if !(0.0..=1.0).contains(&strength) {
            return Err(errors::args_invalid(
                "/strength",
                "strength must be from 0 to 1",
            ));
        }
        cx.noop_reason("the filter left every mask value as it was (no mask, or a uniform one)");
        let map = match self.op {
            FilterOp::Boost => Some(PointMap::Boost),
            FilterOp::Dilute => Some(PointMap::Dilute),
            FilterOp::Normalize => Some(PointMap::Normalize),
            _ => None,
        };
        if let Some(map) = map {
            crate::level::whole_part(cx, &part, |s| {
                let mut written: Vec<gs_core::NodeId> = Vec::new();
                for _ in 0..passes {
                    written.extend(s.mask_map(map, strength));
                }
                written.sort_unstable();
                written.dedup();
                written
            })?;
            return Ok(MaskReport::of(&part, cx.doc.get_part(&part)?.store()));
        }
        let filter = match self.op {
            FilterOp::Blur => MaskFilter::Blur,
            FilterOp::Sharpen => MaskFilter::Sharpen,
            FilterOp::Grow => MaskFilter::Grow,
            _ => MaskFilter::Shrink,
        };
        let store = cx.doc.edit_store(&part, |s| {
            let mut written: Vec<gs_core::NodeId> = Vec::new();
            for _ in 0..passes {
                written.extend(s.mask_filter(filter, strength));
            }
            written.sort_unstable();
            written.dedup();
            written
        })?;
        Ok(MaskReport::of(&part, &store))
    }
}
register_command!(MaskFilterCmd, "Filter mask");
