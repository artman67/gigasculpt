//! The properties behind each claim, as plain functions of what one step saw: the scene before,
//! the scene after, and the arguments. Each returns `Err` with a plain sentence when the
//! property is broken, so the runner can report it and `tests/` can prove each one fails on a
//! known-bad step.

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, Effect, ErrorCode, Outcome, PlainMesh};
use serde_json::Value;

use crate::compare::{canonical, vertex_diff, within, worst};
use crate::measure::{bounds, norm, stats, sub};
use crate::reference;
use crate::session::Scene;
use crate::symmetry::select;
use crate::validate::{Expect, validate};

/// Every part is structurally sound after the step. Faces of zero area are legal (sculpting
/// flattens faces on purpose), so they are not defects.
pub fn sound(_before: &Scene, after: &Scene) -> Result<(), String> {
    for (name, mesh) in after {
        if let Err(defects) = validate(mesh, Expect::ANY) {
            return Err(format!("part '{name}' is broken: {defects:?}"));
        }
    }
    Ok(())
}

/// An internal error is a crash, allowed only for a command whose examples say it crashes.
pub fn no_crash(outcome: &Outcome, allowed: bool) -> Result<(), String> {
    match outcome {
        Outcome::Error(e) if e.code == ErrorCode::Internal && !allowed => {
            Err(format!("the engine crashed: {}", e.message))
        }
        _ => Ok(()),
    }
}

/// Undo or redo succeeded and brought back exactly `want` (a scene's bytes and hash).
pub fn restored(ok: bool, got: &[u8], want: &[u8]) -> Result<(), String> {
    if ok && got == want {
        return Ok(());
    }
    Err(format!(
        "it {} and did not restore the scene exactly",
        if ok { "succeeded" } else { "failed" }
    ))
}

/// `history.list` reports `undoable` and `redoable` steps.
pub fn counts_match(history: &Value, undoable: usize, redoable: usize) -> Result<(), String> {
    let got = (history["undoable"].as_u64(), history["redoable"].as_u64());
    let want = (Some(undoable as u64), Some(redoable as u64));
    if got == want {
        return Ok(());
    }
    Err(format!(
        "undoable and redoable are {got:?}, the model says {want:?}"
    ))
}

/// The engine's report of an edit matches an independent diff of the scene, and it gives a
/// no-op reason exactly when the scene's hash did not change (`changed`): an edit can change
/// what `part.get` does not show, a part's detail levels, and then no vertex counts.
pub fn effect_matches(
    reported: Option<&Effect>,
    before: &Scene,
    after: &Scene,
    changed: bool,
) -> Result<(), String> {
    let Some(reported) = reported else {
        return Err("the edit reported no effect".into());
    };
    let want = reference::effect(before, after);
    let bbox = reported.bbox.map(|[lo, hi]| [lo.0, hi.0]);
    let bits = |b: Option<[[f32; 3]; 2]>| b.map(|c| c.map(|p| p.map(f32::to_bits)));
    let got = (
        &reported.parts,
        reported.topology_changed,
        reported.verts_touched,
    );
    if got != (&want.parts, want.topology_changed, want.verts_touched)
        || bits(bbox) != bits(want.bbox)
    {
        return Err(format!(
            "the edit reported {reported:?}, but the scene shows {want:?}"
        ));
    }
    if reported.noop_reason.is_some() == changed {
        return Err(format!(
            "a no-op reason must come exactly with no change: {reported:?}"
        ));
    }
    Ok(())
}

/// `preserves_topology`: the same parts, each with the same faces, vertex count and
/// attribute layout.
pub fn preserves_topology(before: &Scene, after: &Scene) -> Result<(), String> {
    let names = |s: &Scene| s.keys().cloned().collect::<Vec<_>>();
    if names(before) != names(after) {
        return Err(format!(
            "parts {:?} became {:?}",
            names(before),
            names(after)
        ));
    }
    for (name, old) in before {
        let new = &after[name];
        if old.faces != new.faces || old.positions.len() != new.positions.len() {
            return Err(format!("part '{name}' was rewired"));
        }
        if layout(old) != layout(new) {
            return Err(format!("part '{name}' changed its attribute layout"));
        }
    }
    Ok(())
}

/// Each attribute's name, domain, value type and length.
fn layout(mesh: &PlainMesh) -> Vec<(&String, AttrDomain, bool, usize)> {
    let shape = |values: &AttrValues| match values {
        AttrValues::F32(v) => (true, v.len()),
        AttrValues::U32(v) => (false, v.len()),
    };
    mesh.attributes
        .iter()
        .map(|(name, a)| {
            let (float, len) = shape(&a.values);
            (name, a.domain, float, len)
        })
        .collect()
}

/// `local`: only the target part changed, and with a support, only its vertices within
/// `radius` of a center moved.
pub fn local(
    before: &Scene,
    after: &Scene,
    args: &Value,
    part: &str,
    support: Option<(&str, &str)>,
) -> Result<(), String> {
    let mut args = args.clone();
    let named = select(&mut args, part)
        .first()
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let only = (before.len() == 1)
        .then(|| before.keys().next().cloned())
        .flatten();
    let target = named.or(only);
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for name in names {
        let same = before.get(name).map(canonical) == after.get(name).map(canonical);
        if !same && Some(name) != target.as_ref() {
            return Err(format!(
                "part '{name}' changed, but the command works on {target:?}"
            ));
        }
    }
    let (Some(target), Some((center, radius))) = (target, support) else {
        return Ok(());
    };
    let (Some(old), Some(new)) = (before.get(&target), after.get(&target)) else {
        return Err(format!("part '{target}' did not survive a local edit"));
    };
    if old.positions.len() != new.positions.len() {
        return Err(format!(
            "part '{target}' changed its vertex count in a local edit"
        ));
    }
    let centers: Vec<[f64; 3]> = select(&mut args, center)
        .into_iter()
        .filter_map(|v| vec3(v))
        .collect();
    let Some(r) = select(&mut args, radius)
        .first()
        .and_then(|v| length(v, old))
    else {
        return Ok(());
    };
    for (i, (p, q)) in old.positions.iter().zip(&new.positions).enumerate() {
        let d = centers
            .iter()
            .map(|c| distance(*p, *c))
            .fold(f64::INFINITY, f64::min);
        if d <= r || p.map(f32::to_bits) == q.map(f32::to_bits) {
            continue;
        }
        return Err(format!(
            "vertex {i} of '{target}' moved, {d} from the nearest center, past the radius {r}"
        ));
    }
    Ok(())
}

fn vec3(value: &Value) -> Option<[f64; 3]> {
    let numbers: Vec<f64> = value.as_array()?.iter().filter_map(Value::as_f64).collect();
    numbers.try_into().ok()
}

/// A length argument: a number, or a percentage of `mesh`'s bounding-box diagonal.
fn length(value: &Value, mesh: &PlainMesh) -> Option<f64> {
    let percent = value
        .as_str()
        .and_then(|s| s.strip_suffix('%')?.parse::<f64>().ok());
    let Some(percent) = percent else {
        return value.as_f64();
    };
    let [lo, hi] = bounds(&mesh.positions)?;
    let diagonal = distance(lo, hi.map(f64::from));
    Some(percent / 100.0 * diagonal)
}

fn distance(p: [f32; 3], c: [f64; 3]) -> f64 {
    norm(sub(p.map(f64::from), c))
}

/// `volume_non_decreasing`: every part that is closed before and after keeps at least its
/// volume, less `tolerance` of it and less what rounding its points to `f32` can change it by
/// (`rounding`), before and after: each volume is off by up to its own rounding, so the two
/// add (review 0008, S3). Returns how many parts were checked and the worst relative shrink seen past
/// the rounding (0 when none shrank).
///
/// The rounding counts because a part can be flat: flattened to zero thickness (a zero scale
/// factor), its volume is zero or rounding noise of either sign, and a move that re-rounds its
/// points changes that noise by far more than any fraction of it.
pub fn volume_non_decreasing(
    before: &Scene,
    after: &Scene,
    tolerance: f64,
) -> Result<(usize, f64), String> {
    let closed = Expect {
        manifold: false,
        closed: true,
        euler: None,
    };
    let (mut checked, mut shrink) = (0, 0.0);
    for (name, old) in before {
        let Some(new) = after.get(name) else { continue };
        if validate(old, closed).is_err() || validate(new, closed).is_err() {
            continue;
        }
        let (s0, s1) = (stats(old), stats(new));
        let (v0, v1) = (s0.volume, s1.volume);
        let noise = rounding(old, s0.area) + rounding(new, s1.area);
        // Dividing only a loss past the noise keeps a zero volume from giving 0 / 0.
        let past = (v0 - v1 - noise).max(0.0);
        let lost = if past > 0.0 { past / v0.abs() } else { 0.0 };
        if !within(lost, tolerance) {
            return Err(format!(
                "part '{name}' shrank from volume {v0} to {v1}, past the rounding {noise}"
            ));
        }
        checked += 1;
        shrink = worst(shrink, lost);
    }
    Ok((checked, shrink))
}

/// How much rounding every point of `mesh`, of surface `area`, to `f32` once can change its
/// volume: each coordinate moves at most half a unit in its last place, so each point less
/// than the largest coordinate `m` times `f32::EPSILON`, and a closed surface whose points
/// move `d` gains or loses at most its area times `d`.
fn rounding(mesh: &PlainMesh, area: f64) -> f64 {
    let m = (mesh.positions.iter().flatten()).fold(0.0_f32, |m, c| m.max(c.abs()));
    area * f64::from(m) * f64::from(f32::EPSILON)
}

/// `mirror_equivariant`, comparing: `got` has the parts and faces of `want`, with every vertex
/// within `tolerance` times the largest coordinate of `want` (at least 1). Returns the worst
/// difference in those units.
pub fn matches(want: &Scene, got: &Scene, tolerance: f64) -> Result<f64, String> {
    let names = |s: &Scene| s.keys().cloned().collect::<Vec<_>>();
    if names(want) != names(got) {
        return Err(format!(
            "parts {:?} instead of {:?}",
            names(got),
            names(want)
        ));
    }
    let mut seen: f64 = 0.0;
    for (name, w) in want {
        let g = &got[name];
        if w.faces != g.faces {
            return Err(format!("part '{name}' has different faces"));
        }
        let scale = w
            .positions
            .iter()
            .flatten()
            .fold(1.0f64, |m, c| m.max(f64::from(c.abs())));
        let diff = vertex_diff(w, g);
        let relative = diff.max / scale;
        if !within(relative, tolerance) {
            return Err(format!(
                "part '{name}' vertex {:?} is off by {relative} (relative)",
                diff.vertex
            ));
        }
        seen = worst(seen, relative);
    }
    Ok(seen)
}
