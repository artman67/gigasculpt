//! `boolean.apply` (decision 0057, section 7): each group of parts cut by their operations into
//! a new part, in one undo step. A group the live mode has settled is taken from it, uncut
//! (section 8, as amended for step LB6).

use std::sync::Arc;

use gs_core::transform::Similarity;
use gs_geo::boolean::prepare;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ladder::{self, Operand, Repair, Solver};
use super::live::{Board, Settled};
use super::{geo, groups, word};
use crate::document::{Operation, Part, Props, VertexOrder, unfit};
use crate::mesh::FIRST_RESOLUTION;
use crate::part::{NewPart, pin_ref, report};
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Cut parts by their operations into new parts, one per group, in one undo step.
///
/// Each group is cut left to right from its base, its first part: a `union` part adds to what
/// is above it, `difference` cuts it away and `intersect` keeps only what they share. Each
/// result is a new part at the end of the list, starting its own group as a union, and every
/// part cut is hidden, so one undo brings them back. The active part stays unless it was cut,
/// and then the first result is active.
///
/// The exact engine cuts, keeping every face away from the cuts as it was and carrying face
/// groups, paint, masks, other columns and UVs onto the result (hidden faces become surface).
/// An operand it cannot take as it is gets repaired on its own first, and the result and the
/// warnings say how: turned right way out, rebuilt where it crosses itself, or, when it is open
/// or not manifold, voxel remeshed alone (its UVs are lost). A group the exact engine still
/// cannot cut is voxel remeshed whole by its operations instead, with a warning.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BooleanApply {
    /// The groups, each a list of `[part, operation]` pairs whose first is the base (its
    /// operation is not used). Left out: every group of the part list with a part that shows,
    /// each its parts that show with their operations (`part.set_operation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Vec<(PartRef, Operation)>>>,
    /// `auto`: the exact engine, each operand repaired first when it needs it, and the voxel
    /// remesh when that fails; `manifold`: the exact engine alone, refusing an operand that
    /// needs a repair; `voxel`: the voxel remesh of each whole group.
    #[serde(default)]
    pub solver: Solver,
    /// Cells along the longest side of what a voxel step remeshes, 16 or more (over 2048 is
    /// refused). Left out: 128, written into the logged call unless `solver` is `manifold`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 16))]
    pub resolution: Option<u32>,
    /// The results' name: each takes the first free one of `boolean`, `boolean-2`, ...
    #[serde(default = "boolean")]
    #[schemars(length(min = 1))]
    pub name: String,
}

fn boolean() -> String {
    "boolean".to_string()
}

/// The results.
#[derive(Serialize, JsonSchema)]
pub struct Applied {
    /// One per group, in order.
    pub groups: Vec<Cut>,
}

/// One group's result.
#[derive(Serialize, JsonSchema)]
pub struct Cut {
    /// The new part.
    pub part: NewPart,
    /// The group's parts, base first.
    pub operands: Vec<String>,
    /// The path that cut it: `manifold` (the exact engine) or `voxel`.
    pub solver: Solver,
    /// Each repair made to an operand before the cut, in operand order.
    pub repairs: Vec<Repair>,
    /// The operands' columns the result does not have, by name.
    pub dropped: Vec<String>,
    /// Taken from the live mode's settled result (`view.display`'s `live_boolean`), which has
    /// the bits a cut now would give, instead of cut now.
    pub live: bool,
}

/// A group cut, before its part is reported.
struct Pending {
    name: String,
    operands: Vec<String>,
    solver: Solver,
    repairs: Vec<Repair>,
    dropped: Vec<String>,
    live: bool,
}

impl Command for BooleanApply {
    const NAME: &'static str = "boolean.apply";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const CLAIMS: &'static str = r#"["closed"]"#;
    type Output = Applied;

    fn examples() -> Vec<Example<Self>> {
        let difference = json!({ "operation": "difference" });
        vec![
            Example::raw("cut a cube out of a sphere", json!({}))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({ "center": [1, 0, 0] }))
                .after("part.set_operation", difference.clone()),
            Example::raw(
                "keep only what a sphere and a cube share, by voxels",
                json!({ "solver": "voxel", "resolution": 32 }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [0.5, 0, 0] }))
            .after("part.set_operation", json!({ "operation": "intersect" })),
            Example::raw(
                "cut a group given by hand with the exact engine alone",
                json!({
                    "groups": [[["sphere", "union"], ["cube", "difference"]]],
                    "solver": "manifold",
                }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [1, 0, 0] })),
            Example::raw("one part is nothing to cut", json!({}))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let groups = self
            .groups
            .get_or_insert_with(|| groups::shown(cx.doc.document()));
        for (i, group) in groups.iter_mut().enumerate() {
            if group.is_empty() {
                let path = format!("/groups/{i}");
                return Err(errors::args_invalid(&path, "a group needs a part"));
            }
            for (part, _) in group.iter_mut() {
                pin_ref(cx, part)?;
            }
        }
        if self.solver != Solver::Manifold {
            self.resolution.get_or_insert(FIRST_RESOLUTION);
        }
        unfit(&self.name).map_or(Ok(()), |refused| Err(errors::unavailable(refused)))
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        if !(self.groups.iter().flatten()).any(|group| group.len() >= 2) {
            return Err(
                Unavailable::new("no group has two parts, so there is nothing to cut").fix(
                    "show a second part where it overlaps the first, or give the groups by hand",
                ),
            );
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<Applied, CommandError> {
        let active = cx.doc.active();
        let resolution = self.resolution.unwrap_or(FIRST_RESOLUTION);
        let groups = self.groups.unwrap_or_default();
        let mut pending = Vec::with_capacity(groups.len());
        for (i, group) in groups.iter().enumerate() {
            let at = i as f32 / groups.len() as f32;
            cx.progress(
                Some(at),
                format!("cutting group {} of {}", i + 1, groups.len()),
            );
            pending.push(one(cx, group, self.solver, resolution, &self.name)?);
        }
        settle(cx, &pending, active)?;
        let mut cuts = Vec::with_capacity(pending.len());
        for p in pending {
            cuts.push(Cut {
                part: report(cx, &p.name)?,
                operands: p.operands,
                solver: p.solver,
                repairs: p.repairs,
                dropped: p.dropped,
                live: p.live,
            });
        }
        Ok(Applied { groups: cuts })
    }
}
register_command!(BooleanApply, "Apply boolean");

/// Cuts one group into a new part named from `base`, or takes the live mode's settled result
/// for it when it was cut as this call would cut it.
fn one(
    cx: &mut Ctx,
    group: &[(PartRef, Operation)],
    solver: Solver,
    resolution: u32,
    base: &str,
) -> Result<Pending, CommandError> {
    let from = &group[0].0.0;
    if group[0].1 != Operation::Union {
        let operation = word(group[0].1);
        cx.warn(format!(
            "'{from}' is its group's base, so its operation ({operation}) was ignored: the parts \
             below it are cut from it"
        ));
    }
    let name = cx.doc.free_name(base);
    let live = (solver == Solver::Auto && resolution == FIRST_RESOLUTION).then_some(cx.live);
    if let Some(held) = live.and_then(|board| held(cx, board, group)) {
        return Ok(taken(cx, held, group, name));
    }
    let operands = operands(cx, group)?;
    let import = |i: usize| prepare(&operands[i].local).map(Arc::new);
    let made = ladder::cut(&operands, solver, resolution, &import)?;
    for warning in made.warnings {
        cx.warn(warning);
    }
    if made.mesh.faces.is_empty() {
        return Err(nothing(from));
    }
    let dropped = dropped(&operands, &made.mesh);
    match made.kept {
        Some(kept) => cx
            .doc
            .insert_remeshed(&name, made.mesh, (kept, Similarity::IDENTITY))?,
        None => cx.doc.insert_part(&name, made.mesh, VertexOrder::Spatial)?,
    }
    Ok(Pending {
        name,
        operands: operands.into_iter().map(|o| o.name).collect(),
        solver: made.solver,
        repairs: made.repairs,
        dropped,
        live: false,
    })
}

/// The live mode's settled result for `group`, if it was cut from these very parts.
fn held<'b>(cx: &Ctx, board: &'b Board, group: &[(PartRef, Operation)]) -> Option<&'b Settled> {
    let mut key = Vec::with_capacity(group.len());
    for (PartRef(name), operation) in group {
        key.push((name.as_str(), cx.doc.get_part(name).ok()?, *operation));
    }
    board.settled(&key)
}

/// `held` as the group's new part `name`, as the cut would have put it in, with its run's
/// warnings and report.
fn taken(cx: &mut Ctx, held: &Settled, group: &[(PartRef, Operation)], name: String) -> Pending {
    for warning in &held.warnings {
        cx.warn(warning.clone());
    }
    let props = Props {
        voxel: held.kept,
        ..Props::default()
    };
    cx.doc.replace_part(&name, held.part.share_with(props));
    Pending {
        name,
        operands: group.iter().map(|(part, _)| part.0.clone()).collect(),
        solver: held.solver,
        repairs: held.repairs.clone(),
        dropped: held.dropped.clone(),
        live: true,
    }
}

/// The refusal of a group whose cut leaves no faces, `from` its base.
pub(super) fn nothing(from: &str) -> CommandError {
    let refusal = Unavailable::new(format!("the group from '{from}' cuts to nothing"))
        .fix("move the parts so they overlap as meant, or change an operation");
    errors::unavailable(refusal)
}

/// The group's parts as operands.
fn operands(cx: &Ctx, group: &[(PartRef, Operation)]) -> Result<Vec<Operand>, CommandError> {
    let read = |(part, operation): &(PartRef, Operation)| {
        operand(&part.0, cx.doc.get_part(&part.0)?, *operation)
    };
    group.iter().map(read).collect()
}

/// Part `name` as an operand, as it shows (its active detail level). Refuses a part with no
/// faces.
pub(super) fn operand(
    name: &str,
    part: &Part,
    operation: Operation,
) -> Result<Operand, CommandError> {
    let local = part.view();
    if local.faces.is_empty() {
        let refusal = Unavailable::new(format!("the part '{name}' has no faces to cut"))
            .fix("leave it out of the group, or delete it");
        return Err(errors::unavailable(refusal));
    }
    Ok(Operand {
        name: name.to_string(),
        operation: geo(operation),
        local,
        at: part.transform(),
        world: part.world(),
        defs: part.store().attrs().to_vec(),
    })
}

/// The operands' columns `result` does not have.
pub(super) fn dropped(operands: &[Operand], result: &PlainMesh) -> Vec<String> {
    let mut names: Vec<String> = (operands.iter())
        .flat_map(|o| o.world.attributes.keys())
        .filter(|name| !result.attributes.contains_key(*name))
        .cloned()
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Marks each result as starting its group, a union; hides every operand; and makes the first
/// result active if the active part was an operand (`active` read before the results came in).
fn settle(cx: &mut Ctx, pending: &[Pending], active: Option<String>) -> Result<(), CommandError> {
    let mark = |props: Props| Props {
        start: true,
        operation: Operation::Union,
        ..props
    };
    for p in pending {
        let props = cx.doc.get_part(&p.name)?.props();
        cx.doc.set_props(&p.name, mark(props))?;
    }
    let cut: Vec<&String> = pending.iter().flat_map(|p| &p.operands).collect();
    for name in &cut {
        let props = cx.doc.get_part(name)?.props();
        cx.doc.set_props(
            name,
            Props {
                hidden: true,
                ..props
            },
        )?;
    }
    match active.filter(|name| !cut.contains(&name)) {
        Some(name) => cx.doc.select(&name),
        None => {
            if let Some(first) = pending.first() {
                cx.doc.select(&first.name);
            }
        }
    }
    Ok(())
}
