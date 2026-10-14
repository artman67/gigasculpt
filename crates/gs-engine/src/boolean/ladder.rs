//! The ladder behind `boolean.apply` (decision 0057, section 2) and the carry onto its result
//! (section 6): each operand through gs-geo's steps 1 to 4, step 5 (a voxel remesh of that
//! operand alone) here, then the exact cut of the whole group; a group the exact path cannot
//! cut goes down the voxel fold with a warning. The live mode (`live.rs`) runs the same ladder
//! off the engine thread, so it reads no `Ctx`: its warnings come back in the result.

use std::sync::Arc;

use gs_core::AttrDef;
use gs_core::transform::Similarity;
use gs_geo::boolean::{Booleaned, Leaf, Prepared, Step, Unfit, fold_prepared, prepare};
use gs_geo::operation::Operation;
use gs_geo::parts::merge;
use gs_geo::voxel::{
    CellSize, CloseHoles, Kept, RemeshError, Settings, remesh_fold, repair_operand,
};
use gs_protocol::{CommandError, HIDE, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::level::is_uv_corner;
use crate::mesh::{carry_columns, carry_uv_corners, remesh_refusal};
use crate::part::empty;
use crate::{Unavailable, errors};

/// Which path cuts a group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Solver {
    /// The exact engine, each operand that needs it repaired on its own first; the voxel fold
    /// when a repair or the cut fails.
    #[default]
    Auto,
    /// The exact engine alone: an operand that needs a repair is refused by name.
    Manifold,
    /// The voxel fold, for the whole group.
    Voxel,
}

/// A repair the ladder made to an operand.
#[derive(Clone, Serialize, JsonSchema)]
pub struct Repair {
    /// The operand.
    pub part: String,
    /// The ladder's step: 3 (turned right way out), 4 (rebuilt) or 5 (remeshed alone).
    pub step: u8,
    /// Why, as "step N: ...".
    pub why: String,
}

/// One operand of a group, read from its part.
pub(super) struct Operand {
    pub name: String,
    pub operation: Operation,
    /// The part's own mesh, which the exact engine places by `at`.
    pub local: Arc<PlainMesh>,
    pub at: Similarity,
    /// The part in the scene, which the voxel steps and the carry read.
    pub world: Arc<PlainMesh>,
    /// The kinds of its columns.
    pub defs: Vec<AttrDef>,
}

/// A group's result, in the scene.
pub(super) struct Made {
    pub mesh: PlainMesh,
    /// The cell size to keep, when the voxel fold made it.
    pub kept: Option<Kept>,
    /// The path that cut it: `Manifold` or `Voxel`.
    pub solver: Solver,
    /// The repairs, in operand order.
    pub repairs: Vec<Repair>,
    /// The sentences for the call's warnings: each repair, and a group sent down the fold.
    pub warnings: Vec<String>,
    /// Each face's operand, by the cut's face sources (none from a fold without columns): the
    /// live mode's walls.
    pub owners: Vec<u32>,
}

/// Each face's operand, its source in `faces` past the faces `counts` of the operands before
/// it; the first operand's for a face with none.
fn owners(counts: impl Iterator<Item = usize>, faces: &[Option<u32>]) -> Vec<u32> {
    let mut end = 0;
    let ends: Vec<usize> = counts
        .map(|n| {
            end += n;
            end
        })
        .collect();
    let owner = |f: &Option<u32>| f.map_or(0, |f| ends.partition_point(|&e| e <= f as usize));
    faces.iter().map(|f| owner(f) as u32).collect()
}

/// Operand `i`'s import (steps 1 to 4): `boolean.apply` imports each anew, the live mode
/// keeps one per body.
pub(super) type Import<'a> = dyn Fn(usize) -> Result<Arc<Prepared>, Unfit> + 'a;

/// An operand ready for the exact cut.
struct Ready {
    prepared: Arc<Prepared>,
    at: Similarity,
    /// The mesh the result's sources index for this operand, in the scene, with its columns: the
    /// part, or its step 5 remesh.
    carried: Arc<PlainMesh>,
    /// Step 5's repair, if it ran.
    remeshed: Option<Repair>,
}

/// Cuts `group` as `solver` says, voxel steps at `resolution` cells along the longest side,
/// each operand imported by `import`.
pub(super) fn cut(
    group: &[Operand],
    solver: Solver,
    resolution: u32,
    import: &Import<'_>,
) -> Result<Made, CommandError> {
    let mut sent = None;
    if solver != Solver::Voxel {
        match exact(group, solver, resolution, import)? {
            Ok(made) => return Ok(made),
            Err(why) => sent = Some(why),
        }
    }
    let mut made = fold(group, resolution)?;
    made.warnings.extend(sent.map(|why| {
        format!(
            "the group from '{}' was cut by the voxel fold instead, as {why}",
            group[0].name
        )
    }));
    Ok(made)
}

/// The exact path, or why the group goes down the voxel fold instead (never with `manifold`,
/// which refuses then).
fn exact(
    group: &[Operand],
    solver: Solver,
    resolution: u32,
    import: &Import<'_>,
) -> Result<Result<Made, String>, CommandError> {
    let mut readied = Vec::with_capacity(group.len());
    for (i, operand) in group.iter().enumerate() {
        match ready(operand, import(i), solver, resolution)? {
            Ok(ready) => readied.push(ready),
            Err(why) => return Ok(Err(why)),
        }
    }
    let leaves: Vec<Leaf<'_>> = (readied.iter().zip(group))
        .map(|(r, o)| Leaf {
            operand: &r.prepared,
            at: r.at,
            operation: o.operation,
        })
        .collect();
    let why = match fold_prepared(&leaves) {
        Ok(cut) if !cut.crosses => return carried(group, readied, cut).map(Ok),
        Ok(_) => "its exact result crosses itself".to_string(),
        Err(e) => e.to_string(),
    };
    if solver == Solver::Manifold {
        let base = &group[0].name;
        let refusal = Unavailable::new(format!(
            "the group from '{base}' has no clean exact result: {why}"
        ))
        .fix("use solver auto or voxel");
        return Err(errors::unavailable(refusal));
    }
    Ok(Err(why))
}

/// `o` through steps 1 to 4 (`imported`), or with `auto` step 5 when those cannot use it; or
/// why the group goes down the voxel fold instead.
fn ready(
    o: &Operand,
    imported: Result<Arc<Prepared>, Unfit>,
    solver: Solver,
    resolution: u32,
) -> Result<Result<Ready, String>, CommandError> {
    let unfit = match imported {
        Ok(prepared) if solver == Solver::Auto || prepared.steps().is_empty() => {
            return Ok(Ok(Ready {
                prepared,
                at: o.at,
                carried: Arc::clone(&o.world),
                remeshed: None,
            }));
        }
        Ok(prepared) => (prepared.steps().iter().map(|&s| needs(s)))
            .collect::<Vec<_>>()
            .join(" and "),
        Err(why) => why.to_string(),
    };
    if solver == Solver::Manifold {
        let refusal = Unavailable::new(format!(
            "'{}' cannot be cut exactly as it is: {unfit}",
            o.name
        ))
        .fix("use solver auto, which repairs it first, or voxel");
        return Err(errors::unavailable(refusal));
    }
    let settings = voxels(resolution);
    let repaired = match repair_operand(&o.world, &settings) {
        Ok(repaired) => repaired,
        Err(e) => {
            return Ok(Err(format!(
                "'{}' could not be remeshed alone: {e}",
                o.name
            )));
        }
    };
    let mut mesh = repaired.remeshed.mesh;
    let sources = &repaired.remeshed.sources;
    carry_columns(&o.world, (&mut mesh, sources), &o.defs, |n| n != HIDE);
    let prepared = match prepare(&mesh) {
        Ok(prepared) => Arc::new(prepared),
        Err(why) => {
            return Ok(Err(format!(
                "'{}' remeshed alone is still unfit: {why}",
                o.name
            )));
        }
    };
    let turned = if repaired.turned {
        ", turned right way out"
    } else {
        ""
    };
    let why = format!(
        "step 5: {unfit}, so it was remeshed alone by voxels with its holes closed{turned}"
    );
    let remeshed = Some(Repair {
        part: o.name.clone(),
        step: 5,
        why,
    });
    Ok(Ok(Ready {
        prepared,
        at: Similarity::IDENTITY,
        carried: Arc::new(mesh),
        remeshed,
    }))
}

/// What a repair the `manifold` solver does not make would fix.
fn needs(step: Step) -> &'static str {
    match step {
        Step::TurnedOut => "a shell is inside out (step 3)",
        Step::Rebuilt => "it crosses itself (step 4)",
    }
}

fn voxels(resolution: u32) -> Settings {
    Settings {
        cell: CellSize::Resolution(resolution),
        project: false,
        blur: 0.0,
    }
}

/// The exact result with its operands' columns carried (UV corners included), its repairs, and
/// a warning for each repair and for each step 5 operand whose UVs were dropped.
fn carried(
    group: &[Operand],
    mut readied: Vec<Ready>,
    cut: Booleaned,
) -> Result<Made, CommandError> {
    let olds: Vec<&PlainMesh> = readied.iter().map(|r| r.carried.as_ref()).collect();
    let merged =
        merge(&olds, empty).map_err(|e| errors::unavailable(Unavailable::new(e.to_string())))?;
    let mut mesh = cut.mesh;
    carry_columns(&merged, (&mut mesh, &cut.sources), &defs(group), |n| {
        n != HIDE
    });
    carry_uv_corners(&merged, &mut mesh, &cut.sources.faces);
    let owners = owners(olds.iter().map(|m| m.faces.len()), &cut.sources.faces);
    let mut repairs: Vec<(usize, Repair)> = (cut.repaired.iter())
        .map(|r| {
            (
                r.operand,
                Repair {
                    part: group[r.operand].name.clone(),
                    step: r.step.number(),
                    why: r.step.to_string(),
                },
            )
        })
        .collect();
    let remeshed = readied.iter_mut().enumerate();
    repairs.extend(remeshed.filter_map(|(i, r)| r.remeshed.take().map(|repair| (i, repair))));
    repairs.sort_by_key(|(i, repair)| (*i, repair.step));
    let mut warnings = Vec::new();
    for (i, repair) in &repairs {
        warnings.push(format!(
            "'{}' was repaired before the cut: {}",
            repair.part, repair.why
        ));
        let uvs = group[*i].world.attributes.keys().any(|n| is_uv_corner(n));
        if repair.step == 5 && uvs {
            warnings.push(format!(
                "'{}' was remeshed alone by voxels before the cut (step 5), so its UVs were \
                 dropped",
                repair.part
            ));
        }
    }
    Ok(Made {
        mesh,
        kept: None,
        solver: Solver::Manifold,
        repairs: repairs.into_iter().map(|(_, repair)| repair).collect(),
        warnings,
        owners,
    })
}

/// The voxel fold of the whole group, holes closed, its columns carried (never the UV corners).
fn fold(group: &[Operand], resolution: u32) -> Result<Made, CommandError> {
    let worlds: Vec<&PlainMesh> = group.iter().map(|o| o.world.as_ref()).collect();
    let merged =
        merge(&worlds, empty).map_err(|e| errors::unavailable(Unavailable::new(e.to_string())))?;
    let operands: Vec<(usize, Operation)> = (group.iter())
        .map(|o| (o.world.faces.len(), o.operation))
        .collect();
    let settings = voxels(resolution);
    let mut out =
        remesh_fold(&merged, &operands, &settings, CloseHoles::Large).map_err(|e| refusal(&e))?;
    carry_columns(&merged, (&mut out.mesh, &out.sources), &defs(group), |n| {
        n != HIDE
    });
    Ok(Made {
        owners: owners(worlds.iter().map(|m| m.faces.len()), &out.sources.faces),
        mesh: out.mesh,
        kept: Some(out.kept),
        solver: Solver::Voxel,
        repairs: Vec::new(),
        warnings: Vec::new(),
    })
}

/// The command error for a fold that could not run: over the cell limit as `mesh.voxel_remesh`
/// says it, anything else a precondition.
fn refusal(e: &RemeshError) -> CommandError {
    match e {
        RemeshError::TooManyCells => remesh_refusal(e),
        _ => errors::unavailable(Unavailable::new(format!("the voxel fold failed: {e}"))),
    }
}

/// Every operand's column kinds, the first operand's where two name one column.
fn defs(group: &[Operand]) -> Vec<AttrDef> {
    let mut out: Vec<AttrDef> = Vec::new();
    for def in group.iter().flat_map(|o| &o.defs) {
        if !out.iter().any(|d| d.name == def.name) {
            out.push(def.clone());
        }
    }
    out
}
