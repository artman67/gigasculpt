//! `mask.make`: a mask made from the surface (decision 0026, 7), joined with the mask already
//! there by one combine step.

use gs_core::mask::{Combine, MaskMaker, NewMask};
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::MaskReport;
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, errors, register_command};

/// What `mask.make` measures. Only visible points get a value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "lowercase", deny_unknown_fields)]
pub enum MaskMakerArg {
    /// How recessed the surface is: grooves and pits masked, flat and raised surface free. Each
    /// point is compared with the points out to `rings` rings around it, 1 to 10 (left out, 3).
    /// `contrast`, -100 to 100 (left out, 50), sets how hard the cut is; a negative one flips
    /// the mask, so raised and flat surface is masked instead.
    Cavity {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = -100.0, max = 100.0))]
        contrast: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 1, max = 10))]
        rings: Option<u32>,
    },
    /// Mask every point but those touching a visible face of the face groups in `keep` (0 is no
    /// group). A point where a kept group meets another stays free.
    Groups {
        #[schemars(length(min = 1))]
        keep: Vec<u32>,
    },
    /// Mask the points on the chosen features: `border` (left out, on), an edge only one visible
    /// face uses, so open edges and the edge of the hidden region; and `groups` (left out, on),
    /// where visible faces of different face groups meet. Left out, the mode is `max`, so the
    /// rest of the mask stays.
    Features {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        border: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        groups: Option<bool>,
    },
}

/// How the made mask joins the mask already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModeArg {
    /// The made mask.
    Replace,
    /// The old mask plus the made one.
    Add,
    /// The old mask less the made one.
    Subtract,
    /// The old mask times the made one.
    Multiply,
    /// The lower of the two.
    Min,
    /// The higher of the two.
    Max,
}

/// Make a mask from the surface: by cavity, by face groups (mask all but these groups) or by
/// features (borders and where groups meet), then join it with the mask already there.
///
/// Each visible point gets the maker's value, blurred `blur` times if asked, scaled by
/// `intensity`; `mode` combines it with the old mask (clamped to 0..1) and the point moves
/// `blend` of the way there. Hidden points keep their mask. On a part with detail levels it acts
/// on the top level from any level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskMake {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The maker: `{"by": "cavity"}`, `{"by": "groups", "keep": [2]}` or `{"by": "features"}`,
    /// with their settings.
    pub maker: MaskMakerArg,
    /// How the made mask joins the old one. Left out, `replace`; `max` for features.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ModeArg>,
    /// Percent of the made value written, 0 to 100 (left out, 100): 50 masks at most half.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub intensity: Option<f32>,
    /// Percent of the way each point moves from its old mask to the new one, 0 to 100 (left
    /// out, 100): 100 replaces, 50 averages old and new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub blend: Option<f32>,
    /// Blur passes over the made mask before it is joined, 0 to 20 (left out, 0): each moves a
    /// point half way to the mean of its neighbours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 20))]
    pub blur: Option<u32>,
}

impl MaskMake {
    /// The core maker and combine settings, checked.
    fn resolve(&self) -> Result<(MaskMaker, NewMask), CommandError> {
        let within = |x: f32, lo: f32, hi: f32, path: &str| {
            if (lo..=hi).contains(&x) {
                Ok(x)
            } else {
                let name = path.rsplit('/').next().unwrap_or(path);
                Err(errors::args_invalid(
                    path,
                    format!("{name} must be from {lo} to {hi}"),
                ))
            }
        };
        let maker = match &self.maker {
            MaskMakerArg::Cavity { contrast, rings } => {
                let contrast = within(contrast.unwrap_or(50.0), -100.0, 100.0, "/maker/contrast")?;
                let rings = rings.unwrap_or(3);
                if !(1..=10).contains(&rings) {
                    let message = "rings must be from 1 to 10";
                    return Err(errors::args_invalid("/maker/rings", message));
                }
                MaskMaker::Cavity { contrast, rings }
            }
            MaskMakerArg::Groups { keep } => MaskMaker::Groups { keep: keep.clone() },
            MaskMakerArg::Features { border, groups } => {
                let (border, groups) = (border.unwrap_or(true), groups.unwrap_or(true));
                if !border && !groups {
                    let message = "turn on border or groups: with neither no point is picked";
                    return Err(errors::args_invalid("/maker", message));
                }
                MaskMaker::Features { border, groups }
            }
        };
        let features = matches!(maker, MaskMaker::Features { .. });
        let mode = match self.mode {
            None if features => Combine::Max,
            None | Some(ModeArg::Replace) => Combine::Replace,
            Some(ModeArg::Add) => Combine::Add,
            Some(ModeArg::Subtract) => Combine::Subtract,
            Some(ModeArg::Multiply) => Combine::Multiply,
            Some(ModeArg::Min) => Combine::Min,
            Some(ModeArg::Max) => Combine::Max,
        };
        let blur = self.blur.unwrap_or(0);
        if blur > 20 {
            return Err(errors::args_invalid("/blur", "blur must be from 0 to 20"));
        }
        let new = NewMask {
            mode,
            intensity: within(self.intensity.unwrap_or(100.0), 0.0, 100.0, "/intensity")? / 100.0,
            blend: within(self.blend.unwrap_or(100.0), 0.0, 100.0, "/blend")? / 100.0,
            blur,
        };
        Ok((maker, new))
    }
}

impl Command for MaskMake {
    const NAME: &'static str = "mask.make";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = MaskReport;

    fn examples() -> Vec<Example<Self>> {
        let make = |maker, mode, blend| Self {
            part: None,
            maker,
            mode,
            intensity: None,
            blend,
            blur: None,
        };
        let samples = [[-0.4, 0.0, 0.92], [0.0, 0.0, 1.0], [0.4, 0.0, 0.92]];
        let groove = json!({
            "brush": "Draw",
            "invert": true,
            "size": "20%",
            "strength": 1,
            "samples": samples.map(|p| json!({ "position": p })),
        });
        let cavity = MaskMakerArg::Cavity {
            contrast: None,
            rings: None,
        };
        let features = MaskMakerArg::Features {
            border: None,
            groups: None,
        };
        vec![
            Example::new("mask the groove carved in a sphere, blurred once", {
                let mut groove = make(cavity, None, None);
                groove.blur = Some(1);
                groove
            })
            .after("primitive.sphere", json!({ "subdivisions": 16 }))
            .after("brush.stroke", groove),
            Example::new(
                "mask all but face group 2, half blended into a full mask",
                make(MaskMakerArg::Groups { keep: vec![2] }, None, Some(50.0)),
            )
            .after("primitive.cube", json!({}))
            .after("group.make", json!({ "maker": { "by": "normals" } }))
            .after("mask.set", json!({ "value": 1 })),
            Example::new(
                "mask where groups meet and along the hidden region's edge",
                make(features, None, None),
            )
            .after("primitive.cube", json!({}))
            .after("group.make", json!({ "maker": { "by": "normals" } }))
            .after(
                "visibility.hide",
                json!({ "select": { "by": "groups", "ids": [1] } }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaskReport, CommandError> {
        let (maker, new) = self.resolve()?;
        let part = pinned(self.part)?;
        cx.noop_reason("the made mask left every visible point's mask as it was");
        crate::level::whole_part(cx, &part, |s| s.mask_make(&maker, new))?;
        Ok(MaskReport::of(&part, cx.doc.get_part(&part)?.store()))
    }
}
register_command!(MaskMake, "Make mask");
