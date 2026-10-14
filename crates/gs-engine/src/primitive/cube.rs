//! `primitive.cube`.

use std::collections::BTreeSet;

use gs_protocol::{CommandError, Kind, PlainMesh, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::levels::{add_leveled, placing};
use super::{along_y, cube_grid, moved, placed};
use crate::command::PartArg;
use crate::document::VertexOrder;
use crate::part::{NewPart, add_part};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, register_command};

/// Add an axis-aligned cube or box made of quads, each side split into a grid.
///
/// With `subdivisions` n it has 6n² faces and 6n² + 2 vertices. With `levels`, that cube is the
/// base of a part with detail levels above it, each made as Subdivide makes one with the cube's
/// 12 edges kept sharp, so every level is the cube with twice the grid of the one below; the
/// part starts at its top level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveCube {
    /// Name of the new part. Defaults to "cube", or "cube-2", "cube-3"... if taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// Grid cells along each edge, 1 to 256.
    #[serde(default = "one_cell")]
    #[schemars(range(min = 1, max = 256))]
    pub subdivisions: u32,
    /// Edge length in scene units, or a box's three sides along world x, y and z. Each must be
    /// more than 0.
    #[serde(default = "two")]
    #[schemars(schema_with = "size_schema")]
    pub size: Size,
    /// Detail levels above the cube the other fields make, 0 to 8; each has four times the
    /// faces of the one below. Left out, 0.
    #[serde(default, skip_serializing_if = "super::levels::none")]
    #[schemars(range(max = 8))]
    pub levels: u32,
    /// Center in scene units.
    #[serde(default)]
    pub center: Vec3,
    /// The world axis the cube is built along: "x", "y" or "z". Every axis gives the same cube;
    /// all primitives take it.
    #[serde(default = "along_y")]
    pub axis: Axis,
}

/// A cube's edge, or a box's three sides along world x, y and z.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Size {
    Edge(f32),
    Sides([f32; 3]),
}

impl Size {
    /// Half the side along each axis of the cube as built along Y, for a cube turned to `axis`.
    fn halves(self, axis: Axis) -> [f32; 3] {
        let [x, y, z] = match self {
            Size::Edge(edge) => [edge; 3],
            Size::Sides(sides) => sides,
        };
        // The inverse of `placed`'s cyclic swap: X puts the built x on world z, Z on world y.
        let built = match axis {
            Axis::X => [z, x, y],
            Axis::Y => [x, y, z],
            Axis::Z => [y, z, x],
        };
        built.map(|side| side / 2.0)
    }
}

/// One number or three, each above 0: one type, not `anyOf`, so the form draws it as a field.
fn size_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["number", "array"],
        "exclusiveMinimum": 0,
        "items": { "type": "number", "exclusiveMinimum": 0 },
        "minItems": 3,
        "maxItems": 3,
    })
}

fn one_cell() -> u32 {
    1
}

fn two() -> Size {
    Size::Edge(2.0)
}

impl Command for PrimitiveCube {
    const NAME: &'static str = "primitive.cube";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::New("cube");
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let beside = Self {
            name: None,
            subdivisions: 4,
            size: Size::Edge(2.0),
            levels: 0,
            center: Vec3([3.0, 0.0, 0.0]),
            axis: Axis::Y,
        };
        vec![
            Example::raw("add a cube with edges 1 unit long", json!({ "size": 1 })),
            Example::new("add a second, subdivided cube beside the first", beside)
                .after("primitive.cube", json!({})),
            Example::raw(
                "add a box 3 wide, 2 tall and 1 deep",
                json!({ "size": [3, 2, 1] }),
            ),
            Example::raw(
                "add a six-face cube with five detail levels up to 6,144 faces, as the window's \
                 Cube button does",
                json!({ "subdivisions": 1, "levels": 5 }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let base = cube_grid(self.subdivisions);
        let (half, axis, center) = (self.size.halves(self.axis), self.axis, self.center);
        let at = move |p: [f32; 3]| placed(std::array::from_fn(|i| p[i] * half[i]), axis, center);
        if self.levels == 0 {
            return add_part(cx, self.name, moved(base, at), VertexOrder::Spatial);
        }
        let edges = cube_edges(&base);
        add_leveled(
            cx,
            self.name,
            base,
            (self.levels, &edges),
            |_| {},
            placing(at),
        )
    }
}
register_command!(PrimitiveCube, "Add cube");

/// The edges of `cube_grid`'s cube that lie on its 12 edges: both ends share two coordinates
/// at -1 or 1.
#[expect(
    clippy::float_cmp,
    reason = "cube_grid puts its sides exactly at -1 and 1"
)]
fn cube_edges(cube: &PlainMesh) -> Vec<[u32; 2]> {
    let mut edges = BTreeSet::new();
    for face in &cube.faces {
        for i in 0..4 {
            let (a, b) = (face[i], face[(i + 1) % 4]);
            let (p, q) = (cube.positions[a as usize], cube.positions[b as usize]);
            if (0..3)
                .filter(|&j| p[j].abs() == 1.0 && p[j] == q[j])
                .count()
                == 2
            {
                edges.insert([a.min(b), a.max(b)]);
            }
        }
    }
    edges.into_iter().collect()
}
