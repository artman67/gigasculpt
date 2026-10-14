//! Live Boolean's live mode (decision 0057, section 8, as amended for step LB6). With
//! `view.display`'s `live_boolean` on, the engine looks at the groups after every call and
//! every landed job ([`Engine::look_live`]) and cuts each group whose parts changed again, as
//! `boolean.apply` would, on a thread of its own inside `gs_par::background`. The result is
//! derived state, held here beside the document: never saved, never undone, never logged.
//! `session.state` reports it as `boolean.live`, and `boolean.apply` takes a settled result
//! instead of cutting.
//!
//! Here the key and the look; the runs are `run.rs`, the operand cache `cache.rs`, what
//! `session.state` reports `report.rs`, what the window draws `draw.rs` (section 9, as amended
//! for LB7) and the walls and issue marks it draws `pieces.rs`.

mod cache;
mod draw;
mod pieces;
mod report;
mod run;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gs_geo::voxel::Kept;

use super::groups;
use super::ladder::{Repair, Solver};
use crate::ctx::unfinished;
use crate::document::{Document, Operation, Part};
use crate::engine::Engine;
pub use draw::{LiveDraw, LiveMesh, LivePeel, PeelOperand, Stand, Tint};
use pieces::Mark;
pub(crate) use report::LiveState;
pub(crate) use run::Live;
use run::Ran;

/// A group's part as its run took it: `(name, part, operation)`.
pub(crate) type Key<'d> = (&'d str, &'d Part, Operation);

/// What the live mode holds of the session: each group's run or result, and its counts. Part
/// of the session's state, so a job's copy (`boolean.apply`'s) reads the same.
#[derive(Clone, Default)]
pub(crate) struct Board {
    groups: Vec<Group>,
    runs: u64,
    cancelled: u64,
    landed: u64,
    /// Counted on the runs' threads.
    imports: Arc<AtomicU64>,
}

/// A group of two parts or more that show, and its run or what came of it.
#[derive(Clone)]
struct Group {
    /// Its parts as its run took them: shares, so their bodies and properties as they were.
    members: Arc<[Member]>,
    run: u64,
    cancel: Arc<AtomicBool>,
    /// `None` while its run works; the sentence `boolean.apply` would refuse it with.
    outcome: Option<Result<Arc<Settled>, String>>,
    /// The checks its run marks: Show Issues, Show Coplanar; and its marks, once it lands.
    checks: [bool; 2],
    marks: Arc<[Mark]>,
    /// The result of the group before it, drawn while its run works.
    last: Option<Arc<Settled>>,
}

struct Member {
    name: String,
    part: Part,
    operation: Operation,
}

/// A group's result, as `boolean.apply` would make it.
pub(crate) struct Settled {
    /// The part Apply would insert, but for its place in the document.
    pub part: Part,
    /// The cell size the part keeps when the voxel fold cut it.
    pub kept: Option<Kept>,
    pub solver: Solver,
    pub repairs: Vec<Repair>,
    pub warnings: Vec<String>,
    pub dropped: Vec<String>,
    /// The parts it was made from, and its Subtracts' and Intersects' walls.
    from: Arc<[Member]>,
    walls: Vec<(Operation, Part)>,
    hash: String,
    faces: usize,
    points: usize,
}

impl Engine {
    /// Looks at the groups after a call or a landed job: with the setting off, cancels every
    /// run and drops every result and import; with it on and no gesture open, keeps each group
    /// whose parts are as its run took them and starts one run for each other.
    pub(crate) fn look_live(&mut self) {
        let board = &mut self.state.live;
        if !self.state.display.live_boolean {
            for group in std::mem::take(&mut board.groups) {
                board.drop_group(&group);
            }
            self.live.cache.prune(&[]);
            return;
        }
        if unfinished(&self.state.sculpt, &self.state.gizmos) {
            return;
        }
        let doc = &self.state.doc;
        let shown = groups::shown(doc);
        let wanted: Vec<Vec<Key>> = (shown.iter())
            .filter(|group| group.len() >= 2)
            .map(|group| keyed(doc, group))
            .collect();
        self.live.cache.prune(&wanted);
        let d = &self.state.display;
        let checks = [d.show_issues, d.show_coplanar];
        let old = std::mem::take(&mut board.groups);
        for key in &wanted {
            let group = match old.iter().find(|g| g.holds(key) && g.covers(checks)) {
                Some(group) => group.clone(),
                None => Group {
                    last: Group::before(&old, key).cloned(),
                    ..self.live.start(board, key, checks)
                },
            };
            board.groups.push(group);
        }
        for group in &old {
            if !board.groups.iter().any(|g| g.run == group.run) {
                board.drop_group(group);
            }
        }
    }

    /// Lands every run that has handed its work back, without waiting (the server's
    /// `Task::Live`), and has the window draw what landed.
    pub(crate) fn land_live(&mut self) {
        let mut landed = false;
        while let Ok(ran) = self.live.from.try_recv() {
            landed |= self.land(ran);
        }
        if landed {
            self.redraw();
        }
    }

    /// Lands every run, waiting for those still working: how an in-process caller sees them
    /// land (`Engine::tick`); the window draws what landed.
    pub(crate) fn settle_live(&mut self) {
        let mut landed = false;
        while self.live.owed > 0
            && let Ok(ran) = self.live.from.recv()
        {
            landed |= self.land(ran);
        }
        if landed {
            self.redraw();
        }
    }

    /// Stores a run's result and marks in its group, unless a later change dropped the group.
    /// True if it did.
    fn land(&mut self, ran: Ran) -> bool {
        self.live.owed = self.live.owed.saturating_sub(1);
        let board = &mut self.state.live;
        let group = (board.groups.iter_mut()).find(|g| g.run == ran.run && g.outcome.is_none());
        let (Some(group), Some((outcome, marks))) = (group, ran.outcome) else {
            return false;
        };
        group.outcome = Some(outcome.map(Arc::new));
        group.marks = marks.into();
        group.last = None;
        board.landed += 1;
        true
    }
}

/// `group`'s parts in `doc`.
fn keyed<'d>(doc: &'d Document, group: &'d [(gs_protocol::PartRef, Operation)]) -> Vec<Key<'d>> {
    (group.iter())
        .filter_map(|(part, operation)| {
            let found = doc.get(&part.0)?;
            Some((part.0.as_str(), found, *operation))
        })
        .collect()
}

impl Board {
    /// The settled result of the group of `key`, if its run took these very parts: names,
    /// bodies, transforms and operations.
    pub(crate) fn settled(&self, key: &[Key]) -> Option<&Settled> {
        let group = self.groups.iter().find(|g| g.holds(key))?;
        group
            .outcome
            .as_ref()?
            .as_ref()
            .ok()
            .map(|settled| &**settled)
    }

    /// Lets a group go: a run still working is cancelled, so its result never lands.
    fn drop_group(&mut self, group: &Group) {
        if group.outcome.is_none() {
            group.cancel.store(true, Ordering::Relaxed);
            self.cancelled += 1;
        }
    }
}

impl Group {
    /// True if its run took exactly `key`'s parts.
    fn holds(&self, key: &[Key]) -> bool {
        self.members.len() == key.len()
            && (self.members.iter().zip(key)).all(|(m, &(name, part, operation))| {
                m.name == name
                    && m.operation == operation
                    && m.part.same_body(part)
                    && m.part.transform() == part.transform()
            })
    }

    /// True if its run marks every check `checks` asks for.
    fn covers(&self, checks: [bool; 2]) -> bool {
        (checks.iter().zip(self.checks)).all(|(&wanted, ran)| ran || !wanted)
    }

    /// Its base's name.
    fn base(&self) -> &str {
        &self.members[0].name
    }

    /// The result to draw of the group of `groups` with `key`'s base.
    fn before<'g>(groups: &'g [Group], key: &[Key]) -> Option<&'g Arc<Settled>> {
        let base = key.first()?.0;
        groups.iter().find(|g| g.base() == base)?.drawn()
    }

    /// The result to draw: its own once it lands, the one before it while it works.
    fn drawn(&self) -> Option<&Arc<Settled>> {
        match &self.outcome {
            Some(outcome) => outcome.as_ref().ok(),
            None => self.last.as_ref(),
        }
    }
}
