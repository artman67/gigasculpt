//! `group.make`.

use gs_core::groups::GroupMaker;
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Grouped;
use crate::command::PartArg;
use crate::document::pinned;
use crate::view::ViewSpec;
use crate::visibility::camera_view;
use crate::{Command, Ctx, Example, errors, register_command};

/// How `group.make` cuts the part into groups. Hidden faces never join a group and keep theirs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "lowercase", deny_unknown_fields)]
pub enum MakerArg {
    /// One group per connected piece: faces sharing an edge are in the same piece.
    Shells,
    /// Pieces split wherever two faces sharing an edge turn more than `max_angle` degrees, 1 to
    /// 180 (left out, 30). A cube gives six groups; a smooth sphere gives one.
    Normals {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 1.0, max = 180.0))]
        max_angle: Option<f32>,
    },
    /// The visible faces in one new group, or with `coverage` below 1 (0 to 1, left out 1) a
    /// patchy part of them: that share of the visible faces, in patches that grow larger as
    /// `cluster` rises (0 to 1, left out 0.5). The same `seed` (left out, 0) gives the same
    /// patches.
    Visible {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 1.0))]
        coverage: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 1.0))]
        cluster: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },
    /// The visible masked faces (the mean of their corners' mask at least 0.5) in one new group,
    /// or every visible face when nothing is masked. With `clear_mask` (left out, false) the
    /// mask is then cleared, in the same undo step.
    Masked {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clear_mask: Option<bool>,
    },
    /// The visible faces turned toward the call's `camera` within `angle` degrees, 0 to 90 (left
    /// out, 90), in one new group. Nothing tests what is in front, so inner surfaces facing the
    /// camera count. With `additive` (left out, false) they join the largest group instead: the
    /// one the last front call made, if nothing made a group since.
    Front {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 90.0))]
        angle: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additive: Option<bool>,
    },
}

impl MakerArg {
    /// gs-core's maker, seen from `view` (the call's camera, which only `front` takes).
    fn core(&self, view: Option<gs_core::View>) -> Result<GroupMaker, CommandError> {
        let check = |name: &str, x: f32, lo: f32, hi: f32| {
            if (lo..=hi).contains(&x) {
                Ok(x)
            } else {
                let message = format!("{name} must be from {lo} to {hi}");
                Err(errors::args_invalid(&format!("/maker/{name}"), message))
            }
        };
        if view.is_some() && !matches!(self, MakerArg::Front { .. }) {
            let message = "only the front maker takes a camera";
            return Err(errors::args_invalid("/camera", message));
        }
        Ok(match self {
            MakerArg::Shells => GroupMaker::Shells,
            MakerArg::Normals { max_angle } => GroupMaker::Normals {
                max_angle: check("max_angle", max_angle.unwrap_or(30.0), 1.0, 180.0)?,
            },
            MakerArg::Visible {
                coverage,
                cluster,
                seed,
            } => GroupMaker::Visible {
                coverage: check("coverage", coverage.unwrap_or(1.0), 0.0, 1.0)?,
                cluster: check("cluster", cluster.unwrap_or(0.5), 0.0, 1.0)?,
                seed: seed.unwrap_or(0),
            },
            MakerArg::Masked { clear_mask } => GroupMaker::Masked {
                clear_mask: clear_mask.unwrap_or(false),
            },
            MakerArg::Front { angle, additive } => {
                let Some(view) = view else {
                    let message = "the front maker needs a camera";
                    return Err(errors::args_invalid("/camera", message));
                };
                GroupMaker::Front {
                    view,
                    angle: check("angle", angle.unwrap_or(90.0), 0.0, 90.0)?,
                    additive: additive.unwrap_or(false),
                }
            }
        })
    }
}

/// Make face groups from the part's shape, what is visible, the mask, or a camera.
///
/// Makers number their groups from one above the largest id any face holds, in the store's face
/// order, so the same call on the same part always gives the same ids, and answer `E_LIMIT`,
/// changing nothing, when they would pass the largest id a face may hold (2147483647). On a
/// part with detail levels it works on the top level and shows the result on the level you see.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupMake {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// How to cut the part: `{"by": "shells"}`, `{"by": "normals", "max_angle": 30}`,
    /// `{"by": "visible", "coverage": 0.5, "cluster": 0.5, "seed": 0}`,
    /// `{"by": "masked", "clear_mask": false}` or `{"by": "front", "angle": 90, "additive":
    /// false}` with a `camera`.
    pub maker: MakerArg,
    /// The camera the front maker looks from, as `view.snapshot` takes it: a standard view
    /// ("front"), a session view ("main", pinned to its exact camera), an exact camera or an
    /// orbit. Only with the front maker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The size of the camera's image, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
}

impl GroupMake {
    fn by(maker: MakerArg) -> GroupMake {
        GroupMake {
            part: None,
            maker,
            camera: None,
            viewport: None,
        }
    }
}

impl Command for GroupMake {
    const NAME: &'static str = "group.make";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = Grouped;

    fn examples() -> Vec<Example<Self>> {
        let sphere = json!({ "subdivisions": 8 });
        let mask_pen = json!({
            "brush": "Mask Pen",
            "samples": [{ "position": [0.0, 0.0, 1.0] }],
            "size": "25%",
        });
        let patches = MakerArg::Visible {
            coverage: Some(0.3),
            cluster: None,
            seed: Some(4),
        };
        vec![
            Example::new("one group per connected piece", Self::by(MakerArg::Shells))
                .after("primitive.cube", json!({})),
            Example::new(
                "split a cube into its six sides",
                Self::by(MakerArg::Normals { max_angle: None }),
            )
            .after("primitive.cube", json!({})),
            Example::new("group patches over 30% of the faces", Self::by(patches))
                .after("primitive.sphere", sphere.clone()),
            Example::new(
                "group the masked faces and clear the mask",
                Self::by(MakerArg::Masked {
                    clear_mask: Some(true),
                }),
            )
            .after("primitive.sphere", sphere.clone())
            .after("brush.stroke", mask_pen),
            Example::raw(
                "group the faces turned toward the front view",
                json!({ "maker": { "by": "front", "angle": 60 }, "camera": "front" }),
            )
            .after("primitive.sphere", sphere),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Grouped, CommandError> {
        let part = pinned(self.part)?;
        let view = camera_view(cx, &part, self.camera.as_ref(), self.viewport)?;
        let maker = self.maker.core(view)?;
        cx.noop_reason("the maker found no visible face to group");
        let mut ids = Ok(None);
        crate::level::whole_part(cx, &part, |s| match s.make_groups(maker) {
            Ok(made) => {
                ids = Ok(made.ids);
                made.nodes
            }
            // Nothing was written.
            Err(full) => {
                ids = Err(full);
                Vec::new()
            }
        })?;
        let ids = ids.map_err(super::no_fresh_id)?;
        Ok(Grouped::of(&part, cx.doc.get_part(&part)?, ids))
    }
}
register_command!(GroupMake, "Make face groups");
