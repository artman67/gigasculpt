//! Primitives made with detail levels (decision 0047, sections 7 and 8): the base as the
//! primitive builds it, and `levels` more above it made by `level.subdivide`'s own code.

use std::sync::Arc;

use gs_core::{ColumnSet, Levels, NodeId, Vec3};
use gs_protocol::{CommandError, ErrorCode, NO_VERTEX, PlainMesh};

use crate::Ctx;
use crate::document::{Part, VertexOrder};
use crate::errors::{self, error};
use crate::level::{Crease, LevelSubdivide};
use crate::part::{NewPart, report};

/// The most faces a primitive's top may have, 2^25 (section 7).
const MOST_FACES: u64 = 1 << 25;

/// True for no levels, which the log leaves out.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
pub(super) fn none(levels: &u32) -> bool {
    *levels == 0
}

/// Adds part `name` made from `base` (built along Y about the origin, numbered as built) with
/// `levels` smooth levels above it and `creases`, edges of the base, sharp at every level:
/// `after` runs on the stack after each division, then `finish` once (each primitive turns
/// and moves its points there). The base goes in and the stack replaces it in this one call,
/// so the make is one undo step. A top past the ceiling is refused before anything is built.
pub(super) fn add_leveled(
    cx: &mut Ctx,
    name: Option<String>,
    base: PlainMesh,
    (levels, creases): (u32, &[[u32; 2]]),
    after: impl FnMut(&mut Levels),
    finish: impl FnOnce(&mut Levels),
) -> Result<NewPart, CommandError> {
    let name =
        name.ok_or_else(|| errors::internal("the engine did not pin the new part's name"))?;
    let faces = top_faces(&base, levels);
    if faces > MOST_FACES {
        return Err(CommandError {
            hint: Some(
                "ask for fewer levels or a lower density; level.subdivide adds more one at a time"
                    .into(),
            ),
            ..error(
                ErrorCode::Limit,
                format!(
                    "{levels} levels on this base make {faces} faces; a primitive is made with \
                     at most {MOST_FACES}"
                ),
            )
        });
    }
    let divide = LevelSubdivide {
        part: None,
        smooth: true,
        smooth_uvs: false,
        creases: (creases.iter())
            .map(|&[a, b]| Crease {
                a,
                b,
                sharpness: 255,
            })
            .collect(),
    };
    cx.doc.insert_part(&name, base, VertexOrder::AsGiven)?;
    let part = cx.doc.get_part(&name)?;
    let props = part.props();
    let (mut stack, _) = divide.divided(cx, &name, part, levels, after)?;
    finish(&mut stack);
    cx.doc.replace_part(&name, Part::leveled(stack, props));
    report(cx, &name)
}

/// The faces of the top of `base` with `levels` levels: each quad makes 4 at every level, each
/// triangle 3 at the first.
fn top_faces(base: &PlainMesh, levels: u32) -> u64 {
    let triangles = base.faces.iter().filter(|f| f[3] == NO_VERTEX).count() as u64;
    let quads = base.faces.len() as u64 - triangles;
    match levels {
        0 => quads + triangles,
        _ => (4 * quads + 3 * triangles).saturating_mul(4u64.saturating_pow(levels - 1)),
    }
}

/// An `after` for `levels` divisions of a base with points `below` (by index): every point the
/// level below had stays where it was, and each new point moves by `onto`.
pub(super) fn keeping_below<F: Fn(Vec3) -> Vec3 + Sync>(
    below: &[[f32; 3]],
    levels: u32,
    onto: F,
) -> impl FnMut(&mut Levels) + use<F> {
    let mut below: Vec<Vec3> = below.iter().map(|&p| Vec3::from_array(p)).collect();
    let mut left = levels;
    move |stack: &mut Levels| {
        let num = Arc::clone(stack.numbering());
        let store = stack.store_mut();
        let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
        store.write(&all, ColumnSet::POSITION).each(|node| {
            let n = node.id();
            for (l, p) in node.positions().iter_mut().enumerate() {
                let c = num.index(n, l as u32) as usize;
                *p = below.get(c).copied().unwrap_or_else(|| onto(*p));
            }
        });
        left = left.saturating_sub(1);
        if left > 0 {
            below = by_index(stack);
        }
    }
}

/// The active level's points by index.
fn by_index(stack: &Levels) -> Vec<Vec3> {
    let (num, store) = (stack.numbering(), stack.store());
    let mut points = vec![Vec3::ZERO; num.vertex_count()];
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        for (l, &p) in node.pos.iter().enumerate() {
            points[num.index(n, l as u32) as usize] = p;
        }
    }
    points
}

/// A `finish` that moves every point of every level by `to`.
pub(super) fn placing(to: impl Fn([f32; 3]) -> [f32; 3] + Sync) -> impl FnOnce(&mut Levels) {
    move |stack: &mut Levels| stack.map_positions(|p| Vec3::from_array(to(p.to_array())))
}
