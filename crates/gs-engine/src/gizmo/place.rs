//! `gizmo.place` and `gizmo.reset` (decision 0027, section 2): where a part's gizmo sits and
//! how it acts. View commands: logged, never undone, never touching the document.

use gs_core::gizmo::{Centre, Pose, along};
use gs_core::{Quat, Vec3};
use gs_protocol::{CommandError, Cost, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{PoseInfo, pose_of, set_pose};
use crate::command::PartArg;
use crate::document::pinned;
use crate::transform::{finite, unit_quat};
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Put a part's gizmo somewhere, turn it, or set how it acts.
///
/// Give `origin` and `normal` (a click on the surface: Z along the normal), `origin` and
/// `toward` (a drag: Z along the line), `origin` and `axes`, `origin` alone (keeps the axes),
/// `axes` alone (keeps the origin), or a `rule` that moves the origin. When only Z is given, X
/// is the scene axis least aligned with it, so the same click always gives the same axes.
/// `locked`, `sticky` and `follow` can be set with any of these, or alone. The gizmo is session
/// state: placing it is not an undo step and does not change the scene.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoPlace {
    #[doc = crate::command::part_doc!("The part whose gizmo it is.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The gizmo's new centre, in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<gs_protocol::Vec3>,
    /// With `origin`: the surface normal there; the gizmo's Z points along it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<gs_protocol::Vec3>,
    /// With `origin`: a second point; the gizmo's Z points from `origin` toward it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<gs_protocol::Vec3>,
    /// The gizmo's axes as a quaternion `[x, y, z, w]`, made unit length; it must not be zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<[f32; 4]>,
    /// Moves the centre by a rule and keeps the axes: `unmasked_center` (the centre of the
    /// unmasked visible points' bounding box), `visible_center` or `world_origin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    /// Drags transform the part (true) or only move the gizmo (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// The gizmo stays put after a transform (true) or follows what moved (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky: Option<bool>,
    /// The gizmo follows the mask (true): it goes to the centre of the unmasked visible points
    /// now, unless `origin` or `rule` says where, and again by itself whenever an edit, undo or
    /// redo moves that centre. Off (false) by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<bool>,
}

/// Where a rule puts the gizmo's centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The centre of the bounding box of the visible points whose mask is below 1.
    UnmaskedCenter,
    /// The centre of the bounding box of the visible points.
    VisibleCenter,
    /// The scene's origin.
    WorldOrigin,
}

/// Where a gizmo is now.
#[derive(Serialize, JsonSchema)]
pub struct Placed {
    /// The part whose gizmo it is.
    pub part: String,
    /// The gizmo.
    pub pose: PoseInfo,
}

impl Command for GizmoPlace {
    const NAME: &'static str = "gizmo.place";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    type Output = Placed;

    fn examples() -> Vec<Example<Self>> {
        let sphere = || json!({ "subdivisions": 3 });
        let click = Self {
            origin: Some(gs_protocol::Vec3([0.0, 1.0, 0.0])),
            normal: Some(gs_protocol::Vec3([0.0, 1.0, 0.0])),
            ..Self::default()
        };
        let free = Self {
            locked: Some(false),
            ..Self::default()
        };
        let follow = Self {
            follow: Some(true),
            ..Self::default()
        };
        vec![
            Example::new("put the gizmo on the top of a sphere, Z up", click)
                .after("primitive.sphere", sphere()),
            Example::raw(
                "centre it on the unmasked points",
                json!({ "rule": "unmasked_center" }),
            )
            .after("primitive.sphere", sphere()),
            Example::new("unlock it, so a drag moves only the gizmo", free)
                .after("primitive.sphere", sphere()),
            Example::new(
                "follow the mask: to the unmasked centre now and whenever it moves",
                follow,
            )
            .after("primitive.sphere", sphere()),
            Example::raw(
                "a zero normal is refused",
                json!({ "origin": [0, 0, 0], "normal": [0, 0, 0] }),
            )
            .after("primitive.sphere", sphere())
            .fails_with(gs_protocol::ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Placed, CommandError> {
        let part = pinned(self.part.clone())?;
        let (now, _) = pose_of(cx, &part)?;
        let (mut origin, axes) = self.place(cx, &part, now)?;
        let follow = self.follow.unwrap_or(now.follow);
        if follow && !now.follow && self.origin.is_none() && self.rule.is_none() {
            origin = super::follow::unmasked(cx.doc.get_part(&part)?).unwrap_or(origin);
        }
        let pose = Pose {
            origin,
            axes,
            locked: self.locked.unwrap_or(now.locked),
            sticky: self.sticky.unwrap_or(now.sticky),
            follow,
        };
        set_pose(cx, &part, pose)?;
        Ok(Placed {
            part,
            pose: pose.into(),
        })
    }
}
register_command!(GizmoPlace, "Place gizmo");

impl GizmoPlace {
    /// The new origin and axes, from the pose `now`.
    fn place(&self, cx: &Ctx, part: &str, now: Pose) -> Result<(Vec3, Quat), CommandError> {
        let point = |path: &str, v: Option<gs_protocol::Vec3>| {
            v.map(|v| finite(path, v.0).map(Vec3::from_array))
                .transpose()
        };
        let origin = point("/origin", self.origin)?;
        let normal = point("/normal", self.normal)?;
        let toward = point("/toward", self.toward)?;
        let axes = (self.axes)
            .map(|q| {
                let q = unit_quat(q).ok_or_else(|| {
                    errors::args_invalid("/axes", "axes must be a quaternion that is not zero")
                })?;
                Ok(Quat::from_array(q))
            })
            .transpose()?;
        let z = |path: &str, z: Vec3, what: &str| {
            along(z).ok_or_else(|| errors::args_invalid(path, format!("{what} must not be zero")))
        };
        let settings = self.locked.is_some() || self.sticky.is_some() || self.follow.is_some();
        Ok(match (origin, normal, toward, axes, self.rule) {
            (Some(o), Some(n), None, None, None) => (o, z("/normal", n, "the normal")?),
            (Some(o), None, Some(t), None, None) => (o, z("/toward", t - o, "the drag")?),
            (Some(o), None, None, axes, None) => (o, axes.unwrap_or(now.axes)),
            (None, None, None, Some(q), None) => (now.origin, q),
            (None, None, None, None, Some(rule)) => (centre(cx, part, rule)?, now.axes),
            (None, None, None, None, None) if settings => (now.origin, now.axes),
            _ => {
                let message = "give origin with normal, toward or axes; origin, axes or a rule \
                               alone; or only locked, sticky and follow";
                return Err(errors::args_invalid("", message));
            }
        })
    }
}

/// Where `rule` puts part `part`'s gizmo.
fn centre(cx: &Ctx, part: &str, rule: Rule) -> Result<Vec3, CommandError> {
    let found = cx.doc.get_part(part)?;
    let (store, placed) = (found.store(), found.transform());
    Ok(match rule {
        Rule::WorldOrigin => Vec3::ZERO,
        Rule::VisibleCenter => store.centre(Centre::Visible, &placed).unwrap_or(Vec3::ZERO),
        Rule::UnmaskedCenter => store.centre(Centre::Unmasked, &placed).ok_or_else(|| {
            let reason = Unavailable::new(format!("every visible point of {part} is masked"));
            errors::unavailable(reason.fix("clear some of the mask, or use visible_center"))
        })?,
    })
}

/// Turn a part's gizmo back to the scene's axes, keeping its centre.
///
/// Only the gizmo turns: the part does not move.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoReset {
    #[doc = crate::command::part_doc!("The part whose gizmo it is.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for GizmoReset {
    const NAME: &'static str = "gizmo.reset";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    type Output = Placed;

    fn examples() -> Vec<Example<Self>> {
        let tilted = json!({ "origin": [0.0, 0.0, 0.0], "toward": [1.0, 1.0, 0.0] });
        vec![
            Example::new("square up a tilted gizmo", Self::default())
                .after("primitive.sphere", json!({ "subdivisions": 3 }))
                .after("gizmo.place", tilted),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Placed, CommandError> {
        let part = pinned(self.part)?;
        let (now, _) = pose_of(cx, &part)?;
        let pose = Pose {
            axes: Quat::IDENTITY,
            ..now
        };
        set_pose(cx, &part, pose)?;
        Ok(Placed {
            part,
            pose: pose.into(),
        })
    }
}
register_command!(GizmoReset, "Reset gizmo");
