//! The claims about what a step leaves alone, as plain functions of the scene before and
//! after: `mask_holds`, `hidden_holds` and `positions_only`; and the mirror image
//! `keeps_symmetry` makes the call on.

use gs_protocol::{AttrValues, MASK, PlainMesh, corners};
use serde_json::Value;

use crate::compare::canonical;
use crate::session::Scene;
use crate::symmetry::CubeSymmetry;

/// The face column of hidden flags, as `part.get` names it.
const HIDE: &str = "hide";

/// `mask_holds`: every vertex masked 1 or more, or not a finite number, keeps its position bit
/// for bit, unless the argument at `switch` is `false`. Whether a vertex was held, so whether
/// anything was checked.
pub fn mask_holds(
    before: &Scene,
    after: &Scene,
    args: &Value,
    switch: Option<&str>,
) -> Result<bool, String> {
    if switch.and_then(|p| args.pointer(p)) == Some(&Value::Bool(false)) {
        return Ok(false);
    }
    let masked = |m: &PlainMesh| {
        mask(m)
            .into_iter()
            .map(|v| !v.is_finite() || v >= 1.0)
            .collect()
    };
    held(before, after, "masked", masked)
}

/// `hidden_holds`: every vertex all of whose faces are hidden keeps its position bit for bit.
/// Whether a vertex was held.
pub fn hidden_holds(before: &Scene, after: &Scene) -> Result<bool, String> {
    held(before, after, "hidden", hidden)
}

/// Every vertex `which` picks out of a part before keeps its position bit for bit, in each
/// part that keeps its vertex count (`preserves_topology` speaks for the rest). Whether one
/// was picked.
fn held(
    before: &Scene,
    after: &Scene,
    what: &str,
    which: impl Fn(&PlainMesh) -> Vec<bool>,
) -> Result<bool, String> {
    let mut any = false;
    for (name, old) in before {
        let same = |m: &&PlainMesh| m.positions.len() == old.positions.len();
        let Some(new) = after.get(name).filter(same) else {
            continue;
        };
        for (i, picked) in which(old).into_iter().enumerate() {
            any |= picked;
            let bits = |m: &PlainMesh| m.positions[i].map(f32::to_bits);
            if picked && bits(old) != bits(new) {
                return Err(format!("vertex {i} of '{name}', {what}, moved"));
            }
        }
    }
    Ok(any)
}

/// `positions_only`: the same parts, each with the same vertex count, faces and attributes,
/// bit for bit.
pub fn positions_only(before: &Scene, after: &Scene) -> Result<(), String> {
    if before.keys().ne(after.keys()) {
        let names = |s: &Scene| s.keys().cloned().collect::<Vec<_>>();
        return Err(format!(
            "parts {:?} became {:?}",
            names(before),
            names(after)
        ));
    }
    let rest = |m: &PlainMesh| {
        let positions = vec![[0.0; 3]; m.positions.len()];
        canonical(&PlainMesh {
            positions,
            ..m.clone()
        })
    };
    for (name, old) in before {
        if rest(old) != rest(&after[name]) {
            return Err(format!("part '{name}' changed more than its positions"));
        }
    }
    Ok(())
}

/// Every part of `scene` mirrored across the plane through 0 across axis `a`: each vertex's
/// coordinate on that axis negated, exactly, and each face's corners reversed so its outside
/// stays outside. Its own inverse; vertex and face order, masks and hidden faces are kept.
pub fn mirror(scene: &Scene, a: usize) -> Scene {
    let g = CubeSymmetry {
        axes: [0, 1, 2],
        flips: [a == 0, a == 1, a == 2],
    };
    (scene.iter())
        .map(|(name, mesh)| (name.clone(), g.mesh(mesh)))
        .collect()
}

/// A part's vertex mask, 0 where it has none.
fn mask(mesh: &PlainMesh) -> Vec<f32> {
    match mesh.attributes.get(MASK).map(|a| &a.values) {
        Some(AttrValues::F32(v)) if v.len() == mesh.positions.len() => v.clone(),
        _ => vec![0.0; mesh.positions.len()],
    }
}

/// Each face's hidden flag.
fn hide(mesh: &PlainMesh) -> Vec<bool> {
    let n = mesh.faces.len();
    match mesh.attributes.get(HIDE).map(|a| &a.values) {
        Some(AttrValues::U32(v)) if v.len() == n => v.iter().map(|&h| h != 0).collect(),
        _ => vec![false; n],
    }
}

/// Each vertex: whether it has a face and all of its faces are hidden.
fn hidden(mesh: &PlainMesh) -> Vec<bool> {
    let n = mesh.positions.len();
    let (mut faced, mut shown) = (vec![false; n], vec![false; n]);
    for (face, hid) in mesh.faces.iter().zip(hide(mesh)) {
        for &v in corners(face) {
            if let (Some(f), Some(s)) = (faced.get_mut(v as usize), shown.get_mut(v as usize)) {
                *f = true;
                *s |= !hid;
            }
        }
    }
    faced.iter().zip(&shown).map(|(&f, &s)| f && !s).collect()
}
