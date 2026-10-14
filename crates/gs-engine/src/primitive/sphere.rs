//! `primitive.sphere`, in its two forms (decision 0047, section 3).

use gs_protocol::{CommandError, Kind, PlainMesh, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::levels::{add_leveled, keeping_below, placing};
use super::{along_y, cube_grid, moved, placed, revolve, turn};
use crate::command::PartArg;
use crate::document::VertexOrder;
use crate::errors::args_invalid;
use crate::math::norm;
use crate::part::{NewPart, add_part};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, register_command};

/// How a sphere's faces are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SphereForm {
    /// A subdivided cube pushed out onto the sphere: even quads, no poles.
    Cube,
    /// Latitude and longitude: quads, with one vertex and a fan of triangles at each pole.
    Uv,
}

/// The form of a sphere whose call gives no form and none of either form's fields. Changing it
/// is the maintainer's call (decision 0047, section 3 says what moves).
const DEFAULT_FORM: SphereForm = SphereForm::Cube;

/// Add a sphere: by default a subdivided cube pushed out onto the sphere, made of quads.
///
/// The cube form with `subdivisions` n has 6n² faces and 6n² + 2 vertices, with no poles. The uv
/// form (`form` "uv") has `segments` × (`rings` − 2) quads and a fan of `segments` triangles
/// around a single vertex at each pole, on `axis`. Both are closed, so either is a good base to
/// sculpt from. With `levels`, that sphere is the base of a part with detail levels above it,
/// each made as Subdivide makes one with every new point put on the sphere, and the part starts
/// at its top level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveSphere {
    /// Name of the new part. Defaults to "sphere", or "sphere-2", "sphere-3"... if taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// "cube" or "uv". Left out: "cube" if `subdivisions` is given, "uv" if `segments` or
    /// `rings` is, else "cube".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<SphereForm>,
    /// The cube form: grid cells along each edge of the underlying cube, 1 to 256. Left out, 8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 256), extend("x-when" = { "form": "cube" }))]
    pub subdivisions: Option<u32>,
    /// The uv form: vertices around each ring, 3 to 1024. Left out, 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 3, max = 1024), extend("x-when" = { "form": "uv" }))]
    pub segments: Option<u32>,
    /// The uv form: bands from pole to pole, 2 to 512. Left out, 32.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 2, max = 512), extend("x-when" = { "form": "uv" }))]
    pub rings: Option<u32>,
    /// Detail levels above the sphere the other fields make, 0 to 8; each has four times the
    /// faces of the one below. Left out, 0.
    #[serde(default, skip_serializing_if = "super::levels::none")]
    #[schemars(range(max = 8))]
    pub levels: u32,
    /// Radius in scene units. Must be more than 0.
    #[serde(default = "one")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub radius: f32,
    /// Center in scene units.
    #[serde(default)]
    pub center: Vec3,
    /// The world axis through the uv form's poles: "x", "y" or "z". The cube form is the same
    /// for every axis.
    #[serde(default = "along_y")]
    pub axis: Axis,
}

fn one() -> f32 {
    1.0
}

/// A sphere's form with its counts settled.
enum Layout {
    Cube(u32),
    Uv { segments: u32, rings: u32 },
}

impl PrimitiveSphere {
    /// Settles the form from the fields the call gives and fills in that form's defaults, so
    /// the log says which form it is; refuses the other form's fields.
    fn resolve(&mut self) -> Result<Layout, CommandError> {
        let uv = self.segments.is_some() || self.rings.is_some();
        let form = self.form.unwrap_or(match (self.subdivisions, uv) {
            (Some(_), _) => SphereForm::Cube,
            (None, true) => SphereForm::Uv,
            (None, false) => DEFAULT_FORM,
        });
        self.form = Some(form);
        let uv_only = "belongs to the uv form; the cube form takes subdivisions";
        match form {
            SphereForm::Cube if self.segments.is_some() => Err(args_invalid("/segments", uv_only)),
            SphereForm::Cube if self.rings.is_some() => Err(args_invalid("/rings", uv_only)),
            SphereForm::Uv if self.subdivisions.is_some() => Err(args_invalid(
                "/subdivisions",
                "belongs to the cube form; the uv form takes segments and rings",
            )),
            SphereForm::Cube => Ok(Layout::Cube(*self.subdivisions.get_or_insert(8))),
            SphereForm::Uv => Ok(Layout::Uv {
                segments: *self.segments.get_or_insert(64),
                rings: *self.rings.get_or_insert(32),
            }),
        }
    }
}

impl Command for PrimitiveSphere {
    const NAME: &'static str = "primitive.sphere";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::New("sphere");
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let head = Self {
            name: Some("head".into()),
            form: Some(SphereForm::Cube),
            subdivisions: Some(32),
            segments: None,
            rings: None,
            levels: 0,
            radius: 1.0,
            center: Vec3::default(),
            axis: Axis::Y,
        };
        vec![
            Example::raw(
                "add a sphere with 384 faces, as Add > Sphere does",
                json!({}),
            ),
            Example::new("add a dense sphere named head", head),
            Example::raw(
                "add a latitude and longitude sphere, 64 by 32, with its poles on X",
                json!({ "form": "uv", "axis": "x" }),
            ),
            Example::raw(
                "add a 24-face sphere with five detail levels up to 24,576 faces",
                json!({ "subdivisions": 2, "levels": 5 }),
            ),
        ]
    }

    fn pin(&mut self, _cx: &Ctx) -> Result<(), CommandError> {
        self.resolve().map(drop)
    }

    fn run(mut self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        // Again here: `part.add` runs this command without pinning it.
        let (radius, axis, center) = (self.radius, self.axis, self.center);
        let onto = move |p: [f32; 3]| {
            let s = radius / norm(p);
            p.map(|c| c * s)
        };
        let base = match self.resolve()? {
            Layout::Cube(n) => moved(cube_grid(n), onto),
            Layout::Uv { segments, rings } => uv_sphere(f64::from(radius), segments, rings),
        };
        let at = move |p: [f32; 3]| placed(p, axis, center);
        if self.levels == 0 {
            return add_part(cx, self.name, moved(base, at), VertexOrder::Spatial);
        }
        let onto = move |p: gs_core::Vec3| gs_core::Vec3::from_array(onto(p.to_array()));
        let after = keeping_below(&base.positions, self.levels, onto);
        add_leveled(cx, self.name, base, (self.levels, &[]), after, placing(at))
    }
}
register_command!(PrimitiveSphere, "Add sphere");

/// A latitude and longitude sphere of `radius` about Y: `rings` bands from pole to pole, each
/// ring at a whole fraction of a half turn, so the two halves mirror exactly.
fn uv_sphere(radius: f64, segments: u32, rings: u32) -> PlainMesh {
    let profile: Vec<[f64; 2]> = (1..rings)
        .map(|j| {
            let [c, s] = turn(u64::from(j), 2 * u64::from(rings));
            [radius * s, radius * c]
        })
        .collect();
    revolve(radius, &profile, -radius, segments)
}
