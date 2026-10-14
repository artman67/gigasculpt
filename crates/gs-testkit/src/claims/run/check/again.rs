//! The claims that make the call again: `identity_at_zero` in the same session, from the scene
//! before, and `placement_invariant` and `keeps_symmetry` in a fresh one, on the scene's parts
//! (or their mirror image) as parts without a transform.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Made, Run, Seen};
use crate::claims::{props, shape};
use crate::compare::worst;
use crate::session::{Scene, Session};

impl Run<'_> {
    /// `identity_at_zero`: undoes the step, makes the call again with the arguments at `set`'s
    /// pointers that it has given those values, requires it to report that it changed nothing
    /// and to change no bit, and redoes the step. False, checking nothing, when the call has
    /// none of them or made no undo step.
    pub(super) fn identity(
        &mut self,
        made: &Made,
        set: &BTreeMap<String, Value>,
    ) -> Result<bool, String> {
        let mut args = made.step.args.clone();
        let mut given = false;
        for (at, value) in set {
            if let Some(slot) = args.pointer_mut(at) {
                slot.clone_from(value);
                given = true;
            }
        }
        let Some(id) = made.done.history_id.filter(|_| given) else {
            return Ok(false);
        };
        if self.s.run("history.undo", json!({ "expect": id })).is_err() {
            return Err(format!("undoing step {id} failed"));
        }
        let zero = self.s.run(&made.step.command, args.clone());
        let now = Seen::of(&mut self.s).bytes();
        let redone = self.s.run("history.redo", json!({ "expect": id })).is_ok();
        let back = redone && Seen::of(&mut self.s).bytes() == made.after.bytes();
        let why = match zero {
            Err(e) => format!("failed: {}", e.message),
            Ok(d) if d.history_id.is_some() => "made an undo step".into(),
            Ok(d) if (d.effect.as_ref()).is_none_or(|e| e.noop_reason.is_none()) => {
                "reported a change".into()
            }
            Ok(_) if now != made.before.bytes() => "changed the scene".into(),
            Ok(_) if !back => "left a redo that does not restore the result".into(),
            Ok(_) => return Ok(true),
        };
        Err(format!("the call at the identity, {args}, {why}"))
    }

    /// `placement_invariant`: the worst difference from the baked run, or `None` when the call
    /// gives the argument at `unless`.
    pub(super) fn placement(
        &self,
        made: &Made,
        pinned: &BTreeMap<String, String>,
        unless: Option<&str>,
        tolerance: f64,
    ) -> Result<Option<f64>, String> {
        let given = unless.and_then(|p| made.step.args.pointer(p));
        if given.is_some_and(|v| !v.is_null()) {
            return Ok(None);
        }
        let baked = self.baked(made, &made.before.scene, pinned)?;
        (props::matches(&made.after.scene, &baked, tolerance))
            .map(Some)
            .map_err(|m| format!("against the parts baked: {m}"))
    }

    /// `keeps_symmetry`: the call on the baked scene's mirror image across each plane it
    /// checks, mirrored back, against the call on the baked scene; the worst difference, or
    /// `None` when no plane at `planes` is on with the point at `origin` off it.
    pub(super) fn symmetry(
        &self,
        made: &Made,
        (planes, origin): (&str, &str),
        pinned: &BTreeMap<String, String>,
        tolerance: f64,
    ) -> Result<Option<f64>, String> {
        let switched = made.step.args.pointer(planes);
        let on = |axis: &str| switched.and_then(|p| p.get(axis)).and_then(Value::as_bool);
        let origin = made.done.result.pointer(origin).and_then(Value::as_array);
        let off = |a: usize| (origin.and_then(|o| o.get(a)?.as_f64())).is_some_and(|c| c != 0.0);
        let planes: Vec<(usize, &str)> = (["x", "y", "z"].into_iter().enumerate())
            .filter(|&(a, axis)| on(axis) == Some(true) && off(a))
            .collect();
        if planes.is_empty() {
            return Ok(None);
        }
        let want = self.baked(made, &made.before.scene, pinned)?;
        let mut seen: f64 = 0.0;
        for (a, axis) in planes {
            let image = self.baked(made, &shape::mirror(&made.before.scene, a), pinned)?;
            let off = (props::matches(&want, &shape::mirror(&image, a), tolerance))
                .map_err(|m| format!("across {axis}, the call on the mirror image: {m}"))?;
            seen = worst(seen, off);
        }
        Ok(Some(seen))
    }

    /// The call made again in a fresh session with the plan's seed, on `scene` as parts without
    /// a transform, with the part active before active and what the result reports at each of
    /// `pinned`'s values written into the arguments at its key: the scene it leaves.
    fn baked(
        &self,
        made: &Made,
        scene: &Scene,
        pinned: &BTreeMap<String, String>,
    ) -> Result<Scene, String> {
        let mut s = Session::headless_seeded(self.plan.seed);
        for (name, mesh) in scene {
            s.add_part(name, mesh);
        }
        if let Some(active) = &made.before.active {
            s.ok("part.select", json!({ "part": active }));
        }
        let mut args = made.step.args.clone();
        for (at, from) in pinned {
            let value = made.done.result.pointer(from).cloned();
            let Some(((parent, key), value)) = at.rsplit_once('/').zip(value) else {
                continue;
            };
            if let Some(object) = args.pointer_mut(parent).and_then(Value::as_object_mut) {
                object.insert(key.into(), value);
            }
        }
        (s.run(&made.step.command, args))
            .map_err(|e| format!("on the parts baked, the call failed: {}", e.message))?;
        Ok(s.scene())
    }
}
