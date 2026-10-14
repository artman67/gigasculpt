//! `mesh.bounds`, and the bounding-box helpers other commands use (lengths given as a
//! percentage, effect boxes, scene summaries).

use gs_core::{Aabb, MeshStore};
use gs_protocol::{CommandError, Kind, Length, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Measure the axis-aligned bounding box of a part, or of every part together.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshBounds {
    /// The part to measure. Omit to measure every part together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// An axis-aligned bounding box in scene units.
#[derive(Serialize, JsonSchema)]
pub struct Bounds {
    /// Smallest x, y and z.
    pub min: Vec3,
    /// Largest x, y and z.
    pub max: Vec3,
    /// Middle of the box.
    pub center: Vec3,
    /// Width, height and depth.
    pub size: Vec3,
    /// Length of the box's diagonal. Percent lengths (`"5%"`) are relative to this.
    pub diagonal: f32,
}

impl Bounds {
    /// The bounds of some points, or `None` if there are none.
    pub(crate) fn of(points: &[[f32; 3]]) -> Option<Self> {
        let [min, max] = bounds_of(points.iter().copied())?;
        Some(Self {
            min: Vec3(min),
            max: Vec3(max),
            center: Vec3(std::array::from_fn(|i| f32::midpoint(min[i], max[i]))),
            size: Vec3(std::array::from_fn(|i| max[i] - min[i])),
            diagonal: diagonal([min, max]),
        })
    }
}

impl Command for MeshBounds {
    const NAME: &'static str = "mesh.bounds";
    const KIND: Kind = Kind::Query;
    type Output = Bounds;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("measure every part together", Self { part: None })
                .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Bounds, CommandError> {
        let parts = super::selected(&cx.doc, self.part.as_ref())?;
        let points: Vec<[f32; 3]> = parts
            .iter()
            .flat_map(|(_, mesh)| mesh.positions.clone())
            .collect();
        Bounds::of(&points).ok_or_else(|| {
            let reason = Unavailable::new("there are no vertices to measure")
                .fix("add a part, for example with primitive.sphere");
            crate::errors::unavailable(reason)
        })
    }
}
register_command!(MeshBounds, "Measure bounds");

/// Smallest and largest corner of some points, or `None` if there are none.
pub(crate) fn bounds_of(points: impl IntoIterator<Item = [f32; 3]>) -> Option<[[f32; 3]; 2]> {
    let mut points = points.into_iter();
    let first = points.next()?;
    Some(points.fold([first, first], |[min, max], p| {
        [per_axis(min, p, f32::min), per_axis(max, p, f32::max)]
    }))
}

/// `pick` applied to each axis of `a` and `b`.
pub(crate) fn per_axis(a: [f32; 3], b: [f32; 3], pick: fn(f32, f32) -> f32) -> [f32; 3] {
    [pick(a[0], b[0]), pick(a[1], b[1]), pick(a[2], b[2])]
}

/// Length of a box's diagonal.
pub(crate) fn diagonal([min, max]: [[f32; 3]; 2]) -> f32 {
    (0..3)
        .map(|i| (max[i] - min[i]) * (max[i] - min[i]))
        .sum::<f32>()
        .sqrt()
}

/// A length as given, or the percentage of `diagonal`. A command's length argument for a part
/// goes through [`crate::Ctx::length`], which divides it by the part's scale too; this is for
/// lengths that stay in the scene (`mesh.translate`'s distance, `transform.apply`'s reach) and
/// for the stroke code, which gets its size in the part's own units already
/// (`sculpt/placed.rs`).
pub(crate) fn resolve_length(length: Length, diagonal: f32) -> f32 {
    match length {
        Length::Units(units) => units,
        Length::Percent(percent) => percent / 100.0 * diagonal,
    }
}

/// The bounding-box diagonal of a part, from its store's node bounds: the same number as from
/// its vertices (min and max do not depend on the order they are taken in), without visiting
/// them. Zero for a part without vertices. The boxes are joined with [`Aabb::union`], since
/// node upkeep can leave a node empty and an empty box's corners lie at infinity.
pub(crate) fn store_diagonal(store: &MeshStore) -> f32 {
    let b = (0..store.node_count() as u32)
        .map(|n| store.bounds(n))
        .fold(Aabb::EMPTY, Aabb::union);
    if b.lo.x > b.hi.x {
        return 0.0;
    }
    diagonal([b.lo.to_array(), b.hi.to_array()])
}
