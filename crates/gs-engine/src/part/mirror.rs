//! `part.mirror_copy` (decision 0028, section 3).

use std::collections::BTreeMap;

use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Kind, NO_VERTEX, PartRef, PlainMesh, corners,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, pin_copy_name, pinned_name, place_below, report};
use crate::command::PartArg;
use crate::document::{Props, VertexOrder, pinned};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Where the mirror plane of `part.mirror_copy` goes through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Center {
    /// The scene's origin.
    #[default]
    Origin,
    /// The center of the part's bounding box.
    Part,
}

/// Add a mirrored copy of a part, directly below it in the list.
///
/// The copy is mirrored across the plane square to `axis` through the origin or the part's
/// center, with its faces turned so they still face out. Every mask, face group, hidden face
/// and other column carries across: the copy's vertex i is the mirror of the part's vertex i.
/// A part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartMirrorCopy {
    #[doc = crate::command::part_doc!("The part to mirror.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The axis the mirror plane is square to: `x` mirrors left and right.
    #[serde(default = "x")]
    pub axis: Axis,
    /// Where the plane goes through.
    #[serde(default)]
    pub center: Center,
    /// Name of the copy. Defaults to the part's name with -2, -3, ... (the first that is free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
}

fn x() -> Axis {
    Axis::X
}

impl Command for PartMirrorCopy {
    const NAME: &'static str = "part.mirror_copy";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("mirror a sphere off to the right across x = 0", json!({}))
                .after("primitive.sphere", json!({ "center": [2, 0, 0] })),
            Example::raw(
                "mirror a triangle across its own middle, up and down",
                json!({ "axis": "y", "center": "part", "name": "flipped" }),
            )
            .after(
                "part.create",
                json!({ "name": "triangle", "mesh": super::triangle() }),
            ),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_copy_name(cx, self.part.as_ref(), &mut self.name)
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        crate::level::check_one_level(cx, self.part.as_ref(), "a mirror copy", false)
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        let name = pinned_name(self.name)?;
        cx.doc.single_level(&part, "a mirror copy")?;
        let axis = match self.axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        let source = cx.doc.part(&part)?;
        let plane = plane(&source, axis, self.center == Center::Part);
        let mesh = mirrored(&source, axis, plane);
        // Built from scene points across a scene plane: the copy sits at the identity
        // (decision 0029, section 4).
        let props = Props {
            id: None,
            ..cx.doc.get_part(&part)?.props().at_identity().as_copy()
        };
        cx.doc.put_mesh(&name, mesh, VertexOrder::AsGiven, props)?;
        place_below(cx, &part, std::slice::from_ref(&name))?;
        report(cx, &name)
    }
}
register_command!(PartMirrorCopy, "Mirror copy");

/// Where the plane square to `axis` crosses it: 0, or the middle of `mesh`'s bounding box.
pub(crate) fn plane(mesh: &PlainMesh, axis: usize, about_middle: bool) -> f64 {
    if !about_middle {
        return 0.0;
    }
    let along = mesh.positions.iter().map(|p| f64::from(p[axis]));
    let (lo, hi) = along.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if lo <= hi { f64::midpoint(lo, hi) } else { 0.0 }
}

/// `mesh` mirrored across the plane square to `axis` at `plane`, each face's corners reversed
/// with the first kept first and its UV corners with them; every other column as it is. At 0
/// a coordinate changes sign, exact to the bit (-0.0 too); elsewhere `2 plane - x` is rounded to
/// f32, so twice comes back within one f32 step (decision 0041, amended in step DF6).
pub(crate) fn mirrored(mesh: &PlainMesh, axis: usize, plane: f64) -> PlainMesh {
    let positions = (mesh.positions.iter())
        .map(|&p| {
            let mut q = p;
            q[axis] = if plane == 0.0 {
                -p[axis]
            } else {
                (2.0 * plane - f64::from(p[axis])) as f32
            };
            q
        })
        .collect();
    let faces = (mesh.faces.iter())
        .map(|&[a, b, c, d]| {
            if d == NO_VERTEX {
                [a, c, b, d]
            } else {
                [a, d, c, b]
            }
        })
        .collect();
    let mut attributes = mesh.attributes.clone();
    turn_uvs(&mesh.faces, &mut attributes);
    PlainMesh {
        positions,
        faces,
        attributes,
    }
}

/// Turns each face's UV corners as [`mirrored`] turns its corners: corner `j` of a face of `k`
/// takes old corner `(k - j) mod k`'s value; a triangle's unused fourth corner stays.
fn turn_uvs(faces: &[[u32; 4]], attributes: &mut BTreeMap<String, Attribute>) {
    for group in crate::level::uv_corners() {
        let old: Option<Vec<Vec<f32>>> = (group.iter())
            .map(|name| match attributes.get(name) {
                Some(Attribute {
                    domain: AttrDomain::Face,
                    values: AttrValues::F32(values),
                }) => Some(values.clone()),
                _ => None,
            })
            .collect();
        let Some(old) = old else { continue };
        for (j, name) in group.iter().enumerate().skip(1) {
            let turned = (faces.iter().enumerate())
                .map(|(f, face)| {
                    let k = corners(face).len();
                    old[if j < k { k - j } else { j }][f]
                })
                .collect();
            let values = AttrValues::F32(turned);
            let domain = AttrDomain::Face;
            attributes.insert(name.clone(), Attribute { domain, values });
        }
    }
}
