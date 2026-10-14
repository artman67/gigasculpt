//! Booleans on manifold-rust's exact engine (decision 0057, sections 1, 2, 5 and 6): a group of
//! operands, each a `PlainMesh` placed by its `Similarity`, folded left to right by their
//! [`Operation`]s into one `PlainMesh` with [`Sources`] naming where each point and face came
//! from, as `voxel::remesh` hands back.
//!
//! - **The repair ladder** ([`prepare`]), each operand on its own. Step 1 checks it: every face
//!   names points it has, every point is somewhere, every directed edge is used once with its
//!   reverse present, and the library imports it. Step 2 uses a clean operand as it is. Step 3
//!   turns a shell that is inside out with the library's `repair_orientation`, which keeps real
//!   cavities. Step 4 rebuilds an operand that crosses itself with `rebuild_solid`, "inside at
//!   least once", carrying each corner's face number through as a constant channel. An operand
//!   the ladder cannot use comes back as [`BooleanError::Operand`], naming it and why, for the
//!   caller's step 5 or voxel fold (LB3); nothing is guessed here.
//! - **One expression.** The group is one lazy CSG tree: the first operand is the base (its
//!   operation is not read), and each run of the same operation after it is one node, so the
//!   library unions a run of unions or intersections in one batch and turns `(A - B) - C` into
//!   `A - (B union C)`. Each operand's transform rides on its leaf; no point is edited. The
//!   library runs its parallel steps on the caller's rayon pool, as `gs-par` does.
//! - **Sources.** Each operand goes in with its triangles (quads split by `quad_diagonal`) and
//!   each triangle's index as its face id; the original id the import gives it names the operand.
//!   An output face names the input face its triangle came from. An output point bit-equal to a
//!   corner of its source triangle is that corner; any other gets barycentric weights there. On
//!   a seam the earlier operand owns the point. Two output triangles that are both halves of one
//!   untouched input quad are joined back into it.
//! - **The export rule** (amended in step LB2): before the result is rounded to 32 bits, edges
//!   shorter than [`TOLERANCE`] times its bounding box diagonal are collapsed by the library's
//!   own `set_tolerance`, with the mesh's epsilon raised to that tolerance first, since outside a
//!   boolean the library collapses only up to its epsilon. The 32-bit result is then imported
//!   again and scanned once ([`Booleaned::crosses`]).

mod check;
mod finish;
mod repair;
mod tree;

use std::fmt;

use gs_core::transform::Similarity;
use gs_protocol::PlainMesh;

use crate::operation::Operation;
use crate::voxel::Sources;

pub use repair::{Prepared, prepare};

/// The export rule's tolerance, times the result's bounding box diagonal: edges shorter than it
/// are collapsed before the result is rounded to 32 bits (decision 0057's amendment of
/// 2026-10-12, step LB2).
pub const TOLERANCE: f64 = 1e-6;

/// One operand of a group, as it comes in.
#[derive(Clone, Copy, Debug)]
pub struct Operand<'a> {
    /// The operand's mesh, in its own space.
    pub mesh: &'a PlainMesh,
    /// Where the operand sits in the group's space.
    pub at: Similarity,
    /// How it joins the result so far; the first operand's is not read.
    pub operation: Operation,
}

/// One operand of a group, already through the ladder, so a caller can keep it for the next
/// cut while the operand's body is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct Leaf<'a> {
    /// The operand, through the ladder.
    pub operand: &'a Prepared,
    /// Where the operand sits in the group's space.
    pub at: Similarity,
    /// How it joins the result so far; the first operand's is not read.
    pub operation: Operation,
}

/// A group's result.
#[derive(Clone, Debug, Default)]
pub struct Booleaned {
    /// The result, in the group's space, closed and manifold, or empty.
    pub mesh: PlainMesh,
    /// Where each point and face came from. A point's triangle and a face's number index the
    /// group's operands laid end to end in order: operand 1's first point comes after operand
    /// 0's last, and its first face after operand 0's last.
    pub sources: Sources,
    /// Each operand the ladder repaired, in order, with its step.
    pub repaired: Vec<Repaired>,
    /// Whether the library's scan found the 32-bit result crossing itself after the export rule
    /// (no measured cut did). The next cut that takes it rebuilds it by step 4.
    pub crosses: bool,
}

/// An operand the ladder repaired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repaired {
    /// Its place in the group.
    pub operand: usize,
    /// The step that repaired it; its `Display` says why.
    pub step: Step,
}

/// A repair the ladder made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Step 3: a shell was inside out and was turned right way out.
    TurnedOut,
    /// Step 4: the operand crossed itself and was rebuilt as the space it wraps at least once.
    Rebuilt,
}

impl Step {
    /// The step's number on the ladder (decision 0057, section 2).
    pub fn number(self) -> u8 {
        match self {
            Step::TurnedOut => 3,
            Step::Rebuilt => 4,
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self {
            Step::TurnedOut => "a shell was inside out, so it was turned right way out",
            Step::Rebuilt => "it crossed itself, so it was rebuilt as the space it wraps",
        };
        write!(f, "step {}: {why}", self.number())
    }
}

/// Why the ladder cannot use an operand: the caller's step 5 (a voxel remesh of that operand)
/// or the voxel fold takes it from here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// This face names a point the mesh does not have.
    BadFace(usize),
    /// A point is at no place (not finite).
    NotFinite,
    /// This many edges are used more than once in one direction, or join a point to itself.
    NotManifold {
        /// The edges that break the rule.
        edges: usize,
    },
    /// This many edges have no face on their other side.
    Open {
        /// The edges with one side.
        edges: usize,
    },
    /// The library refused it at this step, for this reason.
    Refused {
        /// The ladder's step.
        step: u8,
        /// The library's reason.
        why: String,
    },
}

impl fmt::Display for Unfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unfit::BadFace(face) => write!(f, "face {face} names a point the mesh does not have"),
            Unfit::NotFinite => write!(f, "a point is not at a finite place"),
            Unfit::NotManifold { edges } => write!(f, "{edges} edges are not manifold"),
            Unfit::Open { edges } => write!(f, "it is open along {edges} edges"),
            Unfit::Refused { step, why } => write!(f, "step {step} failed: {why}"),
        }
    }
}

/// Why a group has no exact result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BooleanError {
    /// The ladder cannot use this operand (its place in the group).
    Operand {
        /// The operand's place in the group.
        operand: usize,
        /// Why.
        why: Unfit,
    },
    /// The library's cut failed, for this reason.
    Engine(String),
}

impl fmt::Display for BooleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BooleanError::Operand { operand, why } => write!(f, "operand {operand}: {why}"),
            BooleanError::Engine(why) => write!(f, "the boolean failed: {why}"),
        }
    }
}

impl std::error::Error for BooleanError {}

/// The group folded left to right: each operand through the ladder, then the cut.
pub fn fold(group: &[Operand<'_>]) -> Result<Booleaned, BooleanError> {
    let prepared = (group.iter().enumerate())
        .map(|(operand, o)| prepare(o.mesh).map_err(|why| BooleanError::Operand { operand, why }))
        .collect::<Result<Vec<_>, _>>()?;
    let leaves: Vec<Leaf<'_>> = (group.iter().zip(&prepared))
        .map(|(o, operand)| Leaf {
            operand,
            at: o.at,
            operation: o.operation,
        })
        .collect();
    fold_prepared(&leaves)
}

/// The group folded left to right, its operands already through the ladder. One operand may
/// stand at several places in the group.
pub fn fold_prepared(group: &[Leaf<'_>]) -> Result<Booleaned, BooleanError> {
    let repaired = (group.iter().enumerate())
        .flat_map(|(operand, l)| {
            (l.operand.steps().iter()).map(move |&step| Repaired { operand, step })
        })
        .collect();
    let (result, leaves) = tree::evaluate(group)?;
    let mut out = finish::finish(result, &leaves, group);
    out.repaired = repaired;
    Ok(out)
}
