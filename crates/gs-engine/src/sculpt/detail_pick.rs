//! `live_detail.pick` (decision 0021, L4; catalog `geometry.live-detail-picker`): the Live
//! Detail size that makes new triangles as fine as those around a point, in every mode's terms.
//! The measure is gs-core's `live::typical`.

use gs_core::live::{self, LEAST_SIZE};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Length, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::aim_camera;
use crate::command::PartArg;
use crate::document::pinned;
use crate::errors::{self, error};
use crate::mesh::{resolve_length, store_diagonal};
use crate::view::{NamedView, ViewSpec};
use crate::{Command, Ctx, Example, register_command};

/// A brush's radius on screen when the call gives a camera and no size, in pixels: a stroke's.
const STROKE_PX: f32 = 64.0;
/// A brush's radius when the call gives neither, as a stroke of world points takes it.
const STROKE_SIZE: Length = Length::Percent(5.0);

/// Measure how fine the surface is around a point, as the size that makes Live Detail add
/// triangles as fine elsewhere: the mean length of the edges of the part's point nearest
/// `position`, over 0.7 (a pass leaves edges about 0.7 of its size on average, as Blender's
/// detail sampler takes it). Answers in every mode's terms, for `live_detail.set`'s `size`
/// (or a stroke's `live_detail`): scene units for constant mode, a fraction of the brush radius
/// for brush mode, pixels for screen mode. Give the brush as a stroke would (`size`, or
/// `camera` with `size_px`); screen mode needs the camera. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveDetailPick {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// A point on or near the surface, in scene units.
    pub position: Vec3,
    /// The camera the brush is seen through, as `brush.stroke` takes it: gives the size in
    /// pixels (screen mode), and the brush radius at the point for one sized in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The size of the camera's image, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The brush radius, for brush mode: scene units, or a percentage of the part's
    /// bounding-box diagonal ("5%"). Left out, `size_px` with a camera, else 5%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    /// The brush radius in pixels on the camera's image. Only with a camera. Left out (and no
    /// `size`), 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 100000.0))]
    pub size_px: Option<f32>,
}

/// The surface's fineness at the point, and the sizes that match it.
#[derive(Serialize, JsonSchema)]
pub struct DetailPicked {
    pub part: String,
    /// The point measured, the one nearest `position`, in the part's vertex numbering.
    pub vertex: u32,
    /// Where it is, in scene units.
    pub at: Vec3,
    /// The mean length of its edges, in scene units.
    pub edge: f32,
    /// The size for constant mode, in scene units: `edge` over 0.7.
    pub constant: f32,
    /// The size for brush mode, as a fraction of the brush radius; at least 0.005, the least
    /// that mode takes.
    pub brush: f32,
    /// The size for screen mode, in pixels; only with a camera.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<f32>,
}

impl Command for LiveDetailPick {
    const NAME: &'static str = "live_detail.pick";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = DetailPicked;

    fn examples() -> Vec<Example<Self>> {
        let at = |camera| Self {
            part: None,
            position: Vec3([0.0, 0.0, 1.0]),
            camera,
            viewport: None,
            size: None,
            size_px: None,
        };
        let front = Some(ViewSpec::Named(NamedView::Front));
        vec![
            Example::new("how fine a sphere is at its pole", at(None))
                .after("primitive.sphere", json!({ "subdivisions": 16 })),
            Example::new("in pixels as the front view sees it", at(front))
                .after("primitive.sphere", json!({ "subdivisions": 16 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<DetailPicked, CommandError> {
        let part = pinned(self.part.clone())?;
        let p = gs_core::Vec3::from_array(self.position.0);
        if !p.to_array().iter().all(|x| x.is_finite()) {
            return Err(errors::args_invalid("/position", "must be finite"));
        }
        if self.size.is_some() && self.size_px.is_some() {
            return Err(errors::args_invalid(
                "/size",
                "give size or size_px, not both",
            ));
        }
        if self.camera.is_none() && self.size_px.is_some() {
            let message = "a size in pixels needs a camera; give size instead";
            return Err(errors::args_invalid("/size_px", message));
        }
        let camera = (self.camera.as_ref())
            .map(|spec| aim_camera(spec, self.viewport, cx.doc.document(), cx.views))
            .transpose()?;
        let found = cx.doc.get_part(&part)?;
        let (store, t) = (found.store(), found.transform());
        let nothing = || {
            error(
                ErrorCode::Precondition,
                format!("part '{part}' has no faces"),
            )
        };
        let (v, _) = store
            .nearest_vertex(t.part_point(p))
            .found
            .ok_or_else(nothing)?;
        let (mean, target) = live::typical(store, v).ok_or_else(nothing)?;
        let (edge, constant) = (t.length(mean), t.length(target));
        let at = t.point(gs_core::PosRead::get(&store.pos(), v));
        // World units per pixel at the point, through the camera.
        let pixel = camera.map(|(c, size)| c.pixel_size(c.depth(at), size));
        let radius = match (self.size, pixel) {
            (Some(size), _) => resolve_length(size, t.length(store_diagonal(store))),
            (None, Some(px)) => self.size_px.unwrap_or(STROKE_PX) * px,
            (None, None) => resolve_length(STROKE_SIZE, t.length(store_diagonal(store))),
        };
        if !(radius > 0.0 && radius.is_finite()) {
            let message = format!("the brush size must come to a positive radius, not {radius}");
            return Err(errors::args_invalid("/size", message));
        }
        let cfg = store.config();
        Ok(DetailPicked {
            vertex: found.numbering().index(cfg.node_of(v), cfg.local_of(v)),
            part,
            at: Vec3(at.to_array()),
            edge,
            constant,
            brush: (constant / radius).max(LEAST_SIZE),
            screen: pixel.map(|px| constant / px),
        })
    }
}
register_command!(LiveDetailPick, "Pick Live Detail size");
