//! `pick.ray`.

use gs_core::{FaceId, MeshStore, PosRead, TopoRead, VertId};
use gs_protocol::{CommandError, Cost, Kind, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{first_hit, numbered, parts};
use crate::sculpt::aim_camera;
use crate::view::{NamedView, ViewSpec};
use crate::{Command, Ctx, Example, errors, register_command};

/// What a ray hits: the first visible surface along it, across every part or in one.
///
/// Give the ray as a point on a camera's image (`camera` and `at`, in pixels from the
/// image's top-left corner, as `brush.stroke` takes screen samples and as `view.snapshot`
/// draws them), or as `origin` and `direction` in scene units. A miss says how close the ray
/// came.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickRay {
    /// The part to look at. Left out, every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The camera, as `view.snapshot` takes it: a standard view ("front"), a session view
    /// ("main"), an exact camera or an orbit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The camera's image size, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The point on the camera's image, `[x, y]` in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
    /// Without a camera: where the ray starts, in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Vec3>,
    /// Without a camera: the direction it goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Vec3>,
}

/// What the ray hit.
#[derive(Serialize, JsonSchema)]
pub struct RayHit {
    pub part: String,
    /// Where, in scene units.
    pub position: Vec3,
    /// The surface normal there: the triangle's, by its winding.
    pub normal: Vec3,
    /// How far along the ray.
    pub distance: f32,
    /// The face's corner nearest the hit, in the part's vertex numbering.
    pub vertex: u32,
}

/// The ray cast, and what it found.
#[derive(Serialize, JsonSchema)]
pub struct Picked {
    /// What the ray hit; left out when it missed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit: Option<RayHit>,
    /// Where the ray starts.
    pub origin: Vec3,
    /// Its unit direction.
    pub direction: Vec3,
    /// When it missed: how far it passed from the nearest vertex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_by: Option<f32>,
}

impl PickRay {
    /// The ray this asks for.
    pub(super) fn ray(&self, cx: &Ctx) -> Result<(gs_core::Vec3, gs_core::Vec3), CommandError> {
        let world = |v: Option<Vec3>| v.map(|Vec3(p)| gs_core::Vec3::from_array(p));
        match (
            &self.camera,
            self.at,
            world(self.origin),
            world(self.direction),
        ) {
            (Some(spec), Some([x, y]), None, None) => {
                let (camera, size) = aim_camera(spec, self.viewport, cx.doc.document(), cx.views)?;
                Ok(camera.ray(x, y, size))
            }
            (None, None, Some(o), Some(d)) if d.len2() > 0.0 && d.len2().is_finite() => {
                Ok((o, d.normalized()))
            }
            _ => Err(errors::args_invalid(
                "",
                "give camera and at, or origin and a direction that is not zero",
            )),
        }
    }
}

/// The corner of face `f` nearest `p`.
pub(crate) fn nearest_corner(store: &MeshStore, f: FaceId, p: gs_core::Vec3) -> VertId {
    let face = store.topo().face(f);
    let pos = store.pos();
    let corners = face.into_iter().filter(|&v| v != gs_core::NO);
    let dist = |v: &VertId| (pos.get(*v) - p).len2();
    corners
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .unwrap_or(face[0])
}

impl Command for PickRay {
    const NAME: &'static str = "pick.ray";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Picked;

    fn examples() -> Vec<Example<Self>> {
        let center = Self {
            part: None,
            camera: Some(ViewSpec::Named(NamedView::Front)),
            viewport: None,
            at: Some([512.0, 512.0]),
            origin: None,
            direction: None,
        };
        let sphere = json!({ "subdivisions": 8 });
        vec![
            Example::new("what the middle of a front snapshot shows", center)
                .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "straight down onto the top of a sphere",
                json!({ "origin": [0.2, 5.0, 0.1], "direction": [0.0, -1.0, 0.0] }),
            )
            .after("primitive.sphere", sphere),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Picked, CommandError> {
        let (o, d) = self.ray(cx)?;
        let doc = cx.doc.document();
        let parts = parts(doc, self.part.as_ref())?;
        let best = first_hit(&parts, o, d);
        let missed_by = best.is_none().then(|| {
            (parts.iter())
                .filter_map(|(_, p)| {
                    let t = p.transform();
                    let (po, pd) = t.part_ray(o, d);
                    p.store()
                        .nearest_to_ray(po, pd)
                        .found
                        .map(|f| t.length(f.1))
                })
                .min_by(f32::total_cmp)
        });
        let hit = best.map(|(distance, hit, name, part)| {
            let t = part.transform();
            RayHit {
                part: name.to_string(),
                position: Vec3(t.point(hit.pos).to_array()),
                normal: Vec3(t.direction(hit.normal).to_array()),
                distance,
                vertex: numbered(part, nearest_corner(part.store(), hit.face, hit.pos)),
            }
        });
        Ok(Picked {
            hit,
            origin: Vec3(o.to_array()),
            direction: Vec3(d.to_array()),
            missed_by: missed_by.flatten(),
        })
    }
}
register_command!(PickRay, "Pick along a ray");
