//! The fold (decision 0057 section 3): operands, each a solid of its own, combined left to
//! right by their operations on one grid.
//!
//! Operands next to each other with the same operation make a **run**, and the first operand
//! counts as a union whatever it is marked: it is where the fold starts. A run is read as one
//! level set, as [`super::remesh_parts`] reads its parts: the distance to the nearest of its
//! triangles, inside where any of its solids is (a run of unions or of differences, since
//! cutting by each in turn cuts by their union) or where all are (a run of intersects). Parts
//! that share a point bit for bit are one solid only within a run (`super::solid_ranges`), so
//! a cutter touching the body is never read as part of it. A fold of unions is one run and
//! remeshes as `remesh_parts` always did, bit for bit.
//!
//! Runs then fold left to right as `OpenVDB`'s level sets do: a union is the minimum of the
//! signed distances (negative inside), an intersect the maximum, a difference the maximum with
//! the run's distance negated. Comparisons are [`f32::total_cmp`], which puts -0 below +0, so
//! the sign of the result is exactly the fold of the insides ("or", "and", "and not"), and the
//! sign a uniform brick reads from one sample ([`super::sign`]) agrees with the bricks around
//! it. A sample keeps the nearest triangle of the run its value came from, so `Sources` name
//! the operand whose surface each new point came from. Ties keep the earlier run, as the seam
//! rule (decision 0057 section 6) gives a point on a seam to the earlier operand.
//!
//! A run is only signed where it can change the result: a union where the result so far is
//! not already deeper inside than the run's distance, an intersect or a difference where it is
//! not already farther outside. So a cutter costs winding work near itself, not everywhere.

use std::ops::Range;

use super::band::split;
use super::grid::{BRICK, Brick, Field, NONE, sample};
use super::sign::Sign;
use super::{Solid, Work};
use crate::operation::Operation;

/// One run of the fold: how its inside is read.
pub(crate) struct Run {
    pub(crate) op: Operation,
    /// Some of its solids are closed or capped, read by the rows.
    pub(crate) rows: bool,
    /// Its solids read by the generalized winding number, as a range of `Source::gwn`.
    pub(crate) gwn: Range<usize>,
    /// An intersect run with an operand that has no triangles: inside nowhere.
    pub(crate) never: bool,
}

/// The run of each operand, and each run's operation.
pub(crate) fn runs(operands: &[(usize, Operation)]) -> (Vec<usize>, Vec<Operation>) {
    let mut ops: Vec<Operation> = Vec::new();
    let mut of = Vec::with_capacity(operands.len());
    for (k, &(_, op)) in operands.iter().enumerate() {
        let op = if k == 0 { Operation::Union } else { op };
        if ops.last() != Some(&op) {
            ops.push(op);
        }
        of.push(ops.len() - 1);
    }
    (of, ops)
}

/// For each run, whether it is inside nowhere: an intersect run with a part (`parts`, each
/// part's triangles; `run`, each part's run) that has no triangles.
pub(crate) fn never(parts: &[Range<usize>], run: &[usize], ops: &[Operation]) -> Vec<bool> {
    let mut never: Vec<bool> = vec![false; ops.len()];
    for (range, &r) in parts.iter().zip(run) {
        never[r] |= ops[r] == Operation::Intersect && range.is_empty();
    }
    never
}

/// The run of each of `tris` triangles, from the solids that own them.
pub(super) fn triangle_runs(solids: &[Solid], tris: usize) -> Vec<u32> {
    let mut of = vec![0u32; tris];
    for s in solids {
        for r in &s.tris {
            of[r.clone()].fill(s.run as u32);
        }
    }
    of
}

/// True if a run with operation `op` can change whether a sample is inside, given whether it
/// is inside the fold so far: a union only outside, an intersect or a difference only inside.
pub(crate) fn matters(op: Operation, so_far: bool) -> bool {
    (op == Operation::Union) != so_far
}

/// Whether a sample is inside after a run that `matters`, from whether it is inside the run.
pub(crate) fn after(op: Operation, inside: bool) -> bool {
    inside != (op == Operation::Difference)
}

/// Signs the given bricks of a fold of several runs; `super::sign::apply` signs those of one.
pub(crate) fn apply(field: &mut Field, sign: &Sign, slots: &[usize], work: &mut Work) {
    let runs = |slot: usize| field.runs.get(slot).map_or(&[][..], |r| &r[..]);
    let done = gs_par::map(slots.len(), |k| {
        brick(sign, &field.bricks[slots[k]], runs(slots[k]))
    });
    for (&slot, (value, near, terms)) in slots.iter().zip(done) {
        work.sign_queries += BRICK as u64;
        work.winding_terms += terms;
        let brick = &mut field.bricks[slot];
        brick.value = value;
        brick.near = near;
        if let Some(runs) = field.runs.get_mut(slot) {
            *runs = Box::default();
        }
    }
}

/// One brick's signed values and nearest triangles, and the winding terms summed. A brick the
/// band measured holds each run's nearest triangles (`runs`, see `Field::runs`) and folds their
/// signed distances; any other (allocated for a film or a repair, every sample at plus or minus
/// the far value) takes the fold's sign, as do samples of the grid's outer layer.
fn brick(sign: &Sign, brick: &Brick, runs: &[u64]) -> (Box<[f32; BRICK]>, Box<[u32; BRICK]>, u64) {
    let (mut value, mut near) = (brick.value.clone(), brick.near.clone());
    let mut terms = 0;
    for i in 0..BRICK {
        let p = sample(brick.at, i);
        let (v, n, t) = if runs.is_empty() || sign.outer(p) {
            let (inside, t) = sign.inside(p);
            let v = if inside {
                -value[i].abs()
            } else {
                value[i].abs()
            };
            (v, near[i], t)
        } else {
            fold(sign, p, |k| runs[k * BRICK + i])
        };
        (value[i], near[i]) = (v, n);
        terms += t;
    }
    (value, near, terms)
}

/// The fold at sample `p` of the runs' signed distances, `key(k)` being run `k`'s nearest
/// triangle as the band keys it: the value, the nearest triangle of the run it came from, and
/// the winding terms summed.
fn fold(sign: &Sign, p: [u32; 3], key: impl Fn(usize) -> u64) -> (f32, u32, u64) {
    let (mut value, mut near, mut terms) = (f32::INFINITY, NONE, 0);
    for (k, run) in sign.runs().iter().enumerate() {
        let (d, tri) = split(key(k));
        let union = run.op == Operation::Union;
        // Deeper inside than `d` already, a union cannot move the value; farther outside, an
        // intersect or a difference cannot.
        let reaches = if union {
            value.total_cmp(&-d).is_gt()
        } else {
            value.total_cmp(&d).is_lt()
        };
        if !reaches {
            continue;
        }
        let (inside, t) = sign.within(p, k);
        terms += t;
        let mine = if after(run.op, inside) { -d } else { d };
        let wins = if union {
            mine.total_cmp(&value).is_lt()
        } else {
            mine.total_cmp(&value).is_gt()
        };
        if wins {
            (value, near) = (mine, tri);
        }
    }
    (value, near, terms)
}
