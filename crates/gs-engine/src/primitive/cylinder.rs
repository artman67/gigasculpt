//! `primitive.cylinder` (decision 0047, section 2).

use gs_protocol::{CommandError, Kind, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use gs_core::{Levels, MeshStore, NodeId};

use super::levels::add_leveled;
use super::{along_y, moved, placed, revolve, ring_vertex};
use crate::command::PartArg;
use crate::document::VertexOrder;
use crate::part::{NewPart, add_part};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, register_command};

/// Add a closed cylinder made of quads, with one welded vertex at the center of each cap.
///
/// Its length runs along `axis`. The side is `segments` around by `divisions` along; each cap
/// is `cap_rings` rings of quads around a fan of `segments` triangles. When `segments` is a
/// multiple of 4, vertices sit exactly on the world axes across each cap. With `levels`, that
/// cylinder is the base of a part with detail levels above it, each made as Subdivide makes one
/// with the rims kept sharp, so the caps stay flat, and the side brought back to `radius`; the
/// part starts at its top level.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveCylinder {
    /// Name of the new part. Defaults to "cylinder", or "cylinder-2", "cylinder-3"... if taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// Radius in scene units. Must be more than 0.
    #[serde(default = "one")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub radius: f32,
    /// Length from cap to cap in scene units. Must be more than 0.
    #[serde(default = "two")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub height: f32,
    /// Vertices around each ring, 3 to 1024.
    #[serde(default = "thirty_two")]
    #[schemars(range(min = 3, max = 1024))]
    pub segments: u32,
    /// Bands of quads along the side, 1 to 1024.
    #[serde(default = "eight")]
    #[schemars(range(min = 1, max = 1024))]
    pub divisions: u32,
    /// Rings on each cap from its rim to its center, 1 to 512; the innermost band is the fan of
    /// triangles.
    #[serde(default = "four")]
    #[schemars(range(min = 1, max = 512))]
    pub cap_rings: u32,
    /// Detail levels above the cylinder the other fields make, 0 to 8; each has four times the
    /// quads of the one below. Left out, 0.
    #[serde(default, skip_serializing_if = "super::levels::none")]
    #[schemars(range(max = 8))]
    pub levels: u32,
    /// The world axis the cylinder's length runs along: "x", "y" or "z".
    #[serde(default = "along_y")]
    pub axis: Axis,
    /// Center in scene units.
    #[serde(default)]
    pub center: Vec3,
}

fn one() -> f32 {
    1.0
}

fn two() -> f32 {
    2.0
}

fn thirty_two() -> u32 {
    32
}

fn eight() -> u32 {
    8
}

fn four() -> u32 {
    4
}

impl Command for PrimitiveCylinder {
    const NAME: &'static str = "primitive.cylinder";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::New("cylinder");
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let arm = Self {
            name: Some("arm".into()),
            radius: 0.25,
            height: 3.0,
            segments: 16,
            divisions: 12,
            cap_rings: 2,
            levels: 0,
            axis: Axis::X,
            center: Vec3([1.5, 0.0, 0.0]),
        };
        vec![
            Example::raw("add a cylinder 2 units long, 512 faces", json!({})),
            Example::new("add a thin cylinder named arm along X", arm),
            Example::raw(
                "add a 96-face cylinder with three detail levels up to 5,760 faces, as the \
                 window's Cylinder button does",
                json!({ "segments": 12, "divisions": 4, "cap_rings": 2, "levels": 3 }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let half = f64::from(self.height) / 2.0;
        let base = revolve(half, &profile(&self), -half, self.segments);
        let (radius, axis, center) = (self.radius, self.axis, self.center);
        if self.levels == 0 {
            let mesh = moved(base, |p| placed(p, axis, center));
            return add_part(cx, self.name, mesh, VertexOrder::Spatial);
        }
        // Back to the radius: every distance from the axis scaled by the radius over the
        // widest, heights kept (decision 0047, section 8).
        let fit = move |stack: &mut Levels| {
            let k = f64::from(radius) / widest(stack.top());
            stack.map_positions(|p| {
                let q = [
                    (f64::from(p.x) * k) as f32,
                    p.y,
                    (f64::from(p.z) * k) as f32,
                ];
                gs_core::Vec3::from_array(placed(q, axis, center))
            });
        };
        let rims = rims(&self);
        add_leveled(cx, self.name, base, (self.levels, &rims), |_| {}, fit)
    }
}
register_command!(PrimitiveCylinder, "Add cylinder");

/// The edges of the two rims of the cylinder `revolve` builds from `profile`: the top cap's
/// outermost ring and the side's last ring.
fn rims(c: &PrimitiveCylinder) -> Vec<[u32; 2]> {
    let n = c.segments;
    [c.cap_rings - 1, c.cap_rings - 1 + c.divisions]
        .into_iter()
        .flat_map(|r| (0..n).map(move |k| [ring_vertex(n, r, k), ring_vertex(n, r, k + 1)]))
        .collect()
}

/// The largest distance from the Y axis of any point of `top`.
fn widest(top: &MeshStore) -> f64 {
    let mut widest = 0.0f64;
    for n in 0..top.node_count() as NodeId {
        for p in top.node(n).pos {
            let (x, z) = (f64::from(p.x), f64::from(p.z));
            widest = widest.max(x * x + z * z);
        }
    }
    widest.sqrt()
}

/// The rings of a cylinder along Y as (radius, height), top to bottom: the top cap's rings out
/// to the rim, the side's rings down to the bottom rim, the bottom cap's rings in. The cap
/// centers are `revolve`'s end vertices. Side heights are whole fractions of the half height,
/// so the two halves mirror exactly.
fn profile(c: &PrimitiveCylinder) -> Vec<[f64; 2]> {
    let (radius, half) = (f64::from(c.radius), f64::from(c.height) / 2.0);
    let cap = |i: u32| radius * (f64::from(i) / f64::from(c.cap_rings));
    let side = |j: u32| {
        let d = i64::from(c.divisions);
        half * ((d - 2 * i64::from(j)) as f64 / d as f64)
    };
    let top = (1..=c.cap_rings).map(|i| [cap(i), half]);
    let down = (1..=c.divisions).map(|j| [radius, side(j)]);
    let bottom = (1..c.cap_rings).rev().map(|i| [cap(i), -half]);
    top.chain(down).chain(bottom).collect()
}
