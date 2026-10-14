//! Landing a finished job (decision 0010): on the engine thread, check that every part the job
//! read or wrote is still as it was when the job started, then apply every change at once, as
//! the step the call would have made in place.
//!
//! The job's copy of the document tells what it started from: a part it only read is still
//! the share it took; a part it replaced or removed is the one its transaction kept; a part it
//! edited in place holds one step whose before side is the part as it started. The live part is
//! unchanged if it still holds those very columns (see `Part::same_as`, `MeshStore::is_base_of`).
//! An edit of another part does not stop a job; only what it depended on.

use std::collections::BTreeSet;

use super::stale;
use super::work::{Done, Output};
use crate::document::{Document, Part, PartChange, Reads};
use crate::engine::{self, Engine};
use crate::history::Change;
use crate::view::own_view;

impl Engine {
    /// Commits a finished job as one step, exactly as if its call had run here, or ends it
    /// with `E_STALE` if something it depended on changed since it started. A cancelled job's
    /// work is dropped. Hands the work back when the session policy blocks the commit right now
    /// (agents paused, a human mid-gesture), to be tried again after the next call.
    pub(crate) fn finish(&mut self, done: Done) -> Option<Done> {
        done.cell.returned();
        if done.cell.cancelled() {
            return None;
        }
        // A job that changed something, or that will be logged (a replay runs it at its
        // commit), depends on what it read; one that only queried does not.
        let lands = (done.result.as_ref())
            .is_ok_and(|out| !out.changes.is_empty() || out.ran.iter().any(|r| r.spec.logged));
        // A job holding the scene (opening one, or the window's edit) was admitted under the
        // policy, which now refuses it; holding it back would hold every edit back with it.
        let holds = self.holds(done.cell.id);
        let control = self.state.control;
        let blocked = |s| engine::policy(control, &done.origin, s).is_err();
        if lands && !holds && done.specs.iter().copied().any(blocked) {
            return Some(done);
        }
        self.released(done.cell.id);
        let Done {
            origin,
            call,
            cell,
            first,
            concurrent_changes,
            result,
            ..
        } = done;
        let reply = result.map_err(|e| (Some(first), e)).and_then(|out| {
            let Output {
                ran,
                changes,
                reads,
                mut doc,
                last_edit,
            } = out;
            if lands && let Err(why) = unchanged(&self.state.doc, &doc, &changes, &reads) {
                return Err((Some(first), stale(&cell.command, &why)));
            }
            let active = self.state.doc.active_id();
            let changes = land(&mut self.state.doc, &mut doc, changes);
            self.state.doc.take_ids(&doc);
            let mut done = engine::Done {
                ran,
                changes,
                doc_replaced: false,
                reads,
                log_as: None,
                active,
            };
            let replay = (self.land_all(&mut done)).map_err(|e| (Some(first), e))?;
            if let Some(at) = last_edit {
                self.state.views.edited(&own_view(&origin), at);
            }
            let mut success = self.commit(&origin, &call, first, done, true);
            self.read_folders_again(&origin);
            self.replay(replay, &mut success);
            Ok((Some(first), success))
        });
        if let Some(seen) = self.seen.get_mut(&origin.client_id) {
            *seen = self.rev;
        }
        // The window got the job's id at once and follows it: a failure reaches it whole
        // (`JobWatch::refusal`), said in a person's words, or asked for when it names an
        // argument the step lacks (decision 0016, section 17).
        let response = self.respond(&origin, &call, concurrent_changes, reply);
        cell.end(response);
        self.jobs.release();
        None
    }
}

/// Checks that everything a job depended on is as it was when the job started: each part it
/// read or wrote, the list of names if it read that, every part if it read the whole document.
/// `copy` is the job's document after its run, `changes` what it changed. Says what changed if
/// anything did.
fn unchanged(
    live: &Document,
    copy: &Document,
    changes: &[PartChange],
    reads: &Reads,
) -> Result<(), String> {
    let changed = |name: &str| (changes.iter()).find(|c| c.change.part() == name);
    // The part as the job started: the one its transaction replaced, or its copy's.
    let start = |name: &str| -> Option<&Part> {
        match changed(name).map(|c| &c.change) {
            Some(Change::Swapped { other, .. }) => other.as_deref(),
            _ => copy.get(name),
        }
    };
    let touched = changes.iter().map(|c| c.change.part());
    let names_then: BTreeSet<&str> = (copy.names().chain(touched.clone()))
        .filter(|&name| start(name).is_some())
        .collect();
    let names_now: BTreeSet<&str> = live.names().collect();
    if (reads.names || reads.all) && names_then != names_now {
        return Err("the list of parts changed".into());
    }
    // A job that added a part handed out ids from its copy's counter; they are the ids the
    // call gets in place only if no other call handed one out meanwhile.
    if reads.ids.is_some_and(|from| from != live.issued()) {
        return Err("another call added a part".into());
    }
    let mut parts: BTreeSet<&str> = reads.parts.iter().map(String::as_str).collect();
    parts.extend(touched);
    if reads.all {
        parts.extend(names_then.iter().chain(&names_now));
    }
    for name in parts {
        let same = match (
            changed(name).map(|c| &c.change),
            live.get(name),
            start(name),
        ) {
            (Some(Change::Edited { order, .. }), Some(now), Some(edited)) => {
                now.is_base_of(edited, order.as_ref())
            }
            (Some(Change::Edited { .. }), _, _) => false,
            (Some(Change::Props { other, .. }), Some(now), Some(then)) => {
                now.same_body(then) && now.props() == *other
            }
            (_, Some(now), Some(then)) => now.same_as(then),
            (_, now, then) => now.is_none() && then.is_none(),
        };
        if !same {
            return Err(format!("part '{name}' changed"));
        }
    }
    Ok(())
}

/// Applies a checked job's changes to the live document, taking the changed parts out of the
/// job's copy, and returns them as the history keeps them: an in-place edit as the same step on
/// the live part, a swap with the live part as the one it replaced.
fn land(live: &mut Document, copy: &mut Document, changes: Vec<PartChange>) -> Vec<PartChange> {
    let mut landed = Vec::new();
    for PartChange {
        change,
        touched,
        topology_changed,
        changed,
    } in changes
    {
        let name = change.part().to_string();
        let change = match change {
            Change::Edited { order, .. } => {
                let (Some(part), Some(edited)) = (live.get_mut(&name), copy.swap(&name, None))
                else {
                    continue;
                };
                // A step that changed the numbering (a stroke that changed topology) takes the
                // copy's along.
                let mut now = edited.order();
                // `unchanged` checked this store is the copy's base, so it adopts the step.
                let Ok(summary) = part.store_mut().adopt((*edited).into_store()) else {
                    continue;
                };
                if order.is_some() {
                    part.swap_order(&mut now);
                }
                Change::Edited {
                    part: name,
                    step: summary.id,
                    order,
                }
            }
            Change::Swapped { .. } => {
                let other = live.swap(&name, copy.swap(&name, None));
                Change::Swapped { part: name, other }
            }
            // `unchanged` checked the live part is the copy's with the old properties.
            Change::Props { other, .. } => {
                let (Some(part), Some(mut props)) =
                    (live.get_mut(&name), copy.get(&name).map(Part::props))
                else {
                    continue;
                };
                part.swap_props(&mut props);
                Change::Props { part: name, other }
            }
        };
        landed.push(PartChange {
            change,
            touched,
            topology_changed,
            changed,
        });
    }
    landed
}
