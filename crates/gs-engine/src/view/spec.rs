//! How commands name a camera (decision 0017): a standard view by name, a session view by name
//! (its camera, which does not move), the exact form the log and `view.get` write, or a
//! friendly orbit. [`ViewSpec::resolve`] is the one place a spec becomes an exact camera, so a
//! pixel of a look, a screen sample of a stroke and a pick at that pixel mean the same point.

use gs_core::Quat;
use gs_core::camera::{Camera, Fit, Lens, Preset, Projection};
use gs_protocol::{CommandError, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::frame::{corners, fitted};
use super::views::Views;
use crate::document::Document;
use crate::errors;

/// A standard view. Front looks at the model's front (its +Z side), right from +X, top from
/// above with the back at the top of the image, iso from the front, right and above.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NamedView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    #[default]
    Iso,
}

impl NamedView {
    /// The camera preset this view starts from.
    pub fn preset(self) -> Preset {
        match self {
            NamedView::Front => Preset::Front,
            NamedView::Back => Preset::Back,
            NamedView::Left => Preset::Left,
            NamedView::Right => Preset::Right,
            NamedView::Top => Preset::Top,
            NamedView::Bottom => Preset::Bottom,
            NamedView::Iso => Preset::Iso,
        }
    }

    /// Its name, as written in arguments.
    pub fn name(self) -> &'static str {
        self.preset().name()
    }
}

/// A session view: `main` (the window's), `agent:<client name>` (an agent's own), or the
/// reserved `ref:<n>` and `widget`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ViewName(
    #[schemars(regex(pattern = r"^(main|widget|agent:.+|ref:[0-9]+)$"))] pub String,
);

/// A camera: a standard view name, a session view's name, the exact form, or an orbit.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ViewSpec {
    /// A standard view that frames every part, for example "front".
    Named(NamedView),
    /// A session view's camera, as it is now, for example "main"; it does not move.
    View(ViewName),
    /// A camera given exactly, as `view.get` reports it.
    Exact(ExactCamera),
    /// A camera orbiting the parts. Every field is optional.
    Orbit(Orbit),
}

/// A camera given exactly: what `view.get` reports and the command log writes, so a camera read
/// back is the same to the bit.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExactCamera {
    /// The view's turn, view space to world, as a unit quaternion `[x, y, z, w]`.
    /// `[0, 0, 0, 1]` is the front view: looking down -Z with +Y up.
    pub orientation: [f32; 4],
    /// The point looked at, in scene units.
    pub target: Vec3,
    /// From the eye to the target, in scene units.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub distance: f32,
    /// Left out: perspective.
    #[serde(default)]
    pub projection: ProjectionArg,
    /// Left out: the default lens, 45 degrees from top to bottom.
    #[serde(default)]
    pub lens: LensArg,
}

/// Perspective or orthographic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectionArg {
    #[default]
    Perspective,
    /// Parallel rays; shows the height a perspective camera sees at the target.
    Orthographic,
}

/// A lens on a 36 by 24 mm frame.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensArg {
    /// Focal length in millimetres. 29 is wide (the default, 45 degrees from top to bottom);
    /// 85 and longer flatten the perspective.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub focal_mm: f32,
    /// Which side of the image the frame's spans: `vertical` (24 mm across the height) or
    /// `horizontal` (36 mm across the width). Left out: vertical.
    #[serde(default)]
    pub fit: FitArg,
    /// Where the view axis meets the image, from its centre, in image heights: `[right, up]`.
    /// Left out: `[0, 0]`.
    #[serde(default)]
    pub shift: [f32; 2],
}

/// Which side of the image a lens's frame spans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FitArg {
    #[default]
    Vertical,
    Horizontal,
}

impl Default for LensArg {
    fn default() -> Self {
        Self::from(Lens::default())
    }
}

impl From<Lens> for LensArg {
    fn from(l: Lens) -> Self {
        Self {
            focal_mm: l.focal_mm,
            fit: match l.fit {
                Fit::Vertical => FitArg::Vertical,
                Fit::Horizontal => FitArg::Horizontal,
            },
            shift: l.shift,
        }
    }
}

impl LensArg {
    /// The lens, or why it is no lens.
    pub(crate) fn lens(self, path: &str) -> Result<Lens, CommandError> {
        if !(self.focal_mm > 0.0 && self.focal_mm.is_finite()) {
            let message = "the focal length must be a positive number of millimetres";
            return Err(errors::args_invalid(&format!("{path}/focal_mm"), message));
        }
        if !self.shift.iter().all(|s| s.is_finite()) {
            let message = "the shift must be finite";
            return Err(errors::args_invalid(&format!("{path}/shift"), message));
        }
        Ok(Lens {
            focal_mm: self.focal_mm,
            fit: match self.fit {
                FitArg::Vertical => Fit::Vertical,
                FitArg::Horizontal => Fit::Horizontal,
            },
            shift: self.shift,
        })
    }
}

impl From<ProjectionArg> for Projection {
    fn from(p: ProjectionArg) -> Self {
        match p {
            ProjectionArg::Perspective => Projection::Perspective,
            ProjectionArg::Orthographic => Projection::Orthographic,
        }
    }
}

impl From<Projection> for ProjectionArg {
    fn from(p: Projection) -> Self {
        match p {
            Projection::Perspective => ProjectionArg::Perspective,
            Projection::Orthographic => ProjectionArg::Orthographic,
        }
    }
}

impl Default for ExactCamera {
    /// The front view of the origin from distance 1.
    fn default() -> Self {
        Camera::preset(Preset::Front).into()
    }
}

impl From<Camera> for ExactCamera {
    fn from(c: Camera) -> Self {
        Self {
            orientation: c.orientation.to_array(),
            target: Vec3(c.target),
            distance: c.distance,
            projection: c.projection.into(),
            lens: c.lens.into(),
        }
    }
}

impl ExactCamera {
    /// The camera, or why it is none: a turn that is not a rotation, a distance that is not
    /// positive, a lens that is no lens. A turn within a rounding of unit length is kept as it
    /// is, so a camera read back is the same to the bit.
    pub fn camera(self, path: &str) -> Result<Camera, CommandError> {
        let q = Quat::from_array(self.orientation);
        let n = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
        if !(n - 1.0).abs().le(&1e-3) {
            let message = "the orientation must be a unit quaternion [x, y, z, w]";
            return Err(errors::args_invalid(
                &format!("{path}/orientation"),
                message,
            ));
        }
        if !self.target.0.iter().all(|t| t.is_finite()) {
            let message = "the target must be finite";
            return Err(errors::args_invalid(&format!("{path}/target"), message));
        }
        if !(self.distance > 0.0 && self.distance.is_finite()) {
            let message = "the distance must be positive";
            return Err(errors::args_invalid(&format!("{path}/distance"), message));
        }
        Ok(Camera {
            orientation: q,
            target: self.target.0,
            distance: self.distance,
            lens: self.lens.lens(&format!("{path}/lens"))?,
            projection: self.projection.into(),
        })
    }
}

/// A camera orbiting the parts.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    /// The named view to start from. Left out: front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<NamedView>,
    /// Degrees around the vertical axis: 0 looks from the front (+Z), 90 from the right (+X).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azimuth_deg: Option<f32>,
    /// Degrees above the horizon: 90 looks straight down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation_deg: Option<f32>,
    /// Degrees the picture turns counterclockwise about the view direction. Left out: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll_deg: Option<f32>,
    /// Left out: perspective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionArg>,
    /// Focal length in millimetres on a 36 by 24 mm frame, fitted to the image's height. Left
    /// out: about 29, 45 degrees from top to bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub focal_mm: Option<f32>,
    /// Distance from the target, in scene units. Left out: just far enough to frame every
    /// part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.000_001))]
    pub distance: Option<f32>,
    /// The point looked at. Left out: the center of every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec3>,
}

impl Orbit {
    /// The camera: the view's preset, the angles given in place of its own, the roll, the
    /// projection and the lens, framing `points` for an image `aspect` wide per high, then the
    /// distance and target given in place of the framing's.
    fn camera(&self, points: &[[f32; 3]], aspect: f32) -> Result<Camera, CommandError> {
        let preset = self.view.unwrap_or(NamedView::Front).preset();
        let mut camera = Camera::preset(preset);
        if self.azimuth_deg.is_some() || self.elevation_deg.is_some() {
            let (azimuth, elevation) = preset.angles();
            let azimuth = self.azimuth_deg.map_or(azimuth, f32::to_radians);
            let elevation = self.elevation_deg.map_or(elevation, f32::to_radians);
            camera.orientation = Camera::turned(azimuth, elevation);
        }
        if let Some(roll) = self.roll_deg {
            let target = gs_core::Vec3::from_array(camera.target);
            camera.roll(target, roll.to_radians());
        }
        camera.projection = self.projection.unwrap_or_default().into();
        if let Some(focal_mm) = self.focal_mm {
            let lens = LensArg {
                focal_mm,
                ..LensArg::default()
            };
            camera.lens = lens.lens("")?;
        }
        camera.fit(points, aspect);
        camera.distance = self.distance.unwrap_or(camera.distance);
        camera.target = self.target.map_or(camera.target, |t| t.0);
        Ok(camera)
    }

    /// How a contact sheet labels it: the view and the angles given.
    fn label(&self) -> String {
        let view = self.view.unwrap_or(NamedView::Front).name();
        let angles = [
            ("az", self.azimuth_deg),
            ("el", self.elevation_deg),
            ("roll", self.roll_deg),
        ];
        let angles = (angles.iter()).filter_map(|(w, d)| d.map(|d| format!("{w} {d}")));
        std::iter::once(view.to_string())
            .chain(angles)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl ViewSpec {
    /// The exact camera this spec means for an image `aspect` wide per high: a standard view or
    /// an orbit framed to every part of `doc`, a session view's camera as it is, or the exact
    /// camera given.
    pub(crate) fn resolve(
        &self,
        doc: &Document,
        views: &Views,
        aspect: f32,
    ) -> Result<Camera, CommandError> {
        match self {
            Self::Named(view) => Ok(fitted(view.preset(), doc, aspect)),
            Self::View(name) => views.camera_of(&name.0, doc),
            Self::Exact(exact) => exact.camera(""),
            Self::Orbit(orbit) => orbit.camera(&corners(doc), aspect),
        }
    }

    /// How a contact sheet labels it.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Named(view) => view.name().to_string(),
            Self::View(name) => name.0.clone(),
            Self::Exact(_) => "camera".into(),
            Self::Orbit(orbit) => orbit.label(),
        }
    }
}
