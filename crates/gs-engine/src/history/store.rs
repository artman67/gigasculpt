//! Undo history: a list of steps with a cursor, a byte budget, and named checkpoints.
//!
//! A step lists what one call (a command or a batch) did to each part it changed. A part edited
//! in place keeps its own copy-on-write undo step in its mesh store, named by the step's id, so
//! undo and redo there swap node columns; a part that was added, removed or replaced is moved
//! whole into the step, and undo and redo swap it with the document's; a part whose properties
//! alone changed keeps its old properties in the step. Memory is counted by allocation, once
//! each (see `held`).
//!
//! Step ids count steps (1, 2, 3...). Only calls that change the scene make steps, so a replay
//! of the session's log, which holds exactly those calls in order, gives every step the same id.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use gs_core::StepId;
use gs_protocol::{CommandError, ErrorCode, Role};
use serde_json::Value;

use super::held::{Held, held};
use crate::Origin;
use crate::document::{Document, Order, Part, Props};
use crate::errors::{self, error};

/// What one step did to one part.
pub(crate) enum Change {
    /// The part was edited in place. Its mesh store holds the step, `step`. `order` is the
    /// part's numbering on the other side of the step when the step changed it: a stroke that
    /// changes topology switches a given numbering to the store's own (decision 0021, item
    /// 11.4).
    Edited {
        part: String,
        step: StepId,
        order: Option<Order>,
    },
    /// The part was added, removed or replaced: `other` is what is on the other side of the
    /// step (before it while the step is done, after it while it is undone).
    Swapped {
        part: String,
        other: Option<Box<Part>>,
    },
    /// The part's properties changed and nothing else (a list edit, a move of its transform,
    /// decision 0029): `other` are the properties on the other side of the step. The part stays
    /// in place, so the window keeps its copy of it.
    Props { part: String, other: Props },
}

impl Change {
    /// The part's name.
    pub fn part(&self) -> &str {
        match self {
            Self::Edited { part, .. } | Self::Swapped { part, .. } | Self::Props { part, .. } => {
                part
            }
        }
    }
}

/// One undo step: what one command (or batch) changed.
pub(crate) struct Entry {
    /// The step's id.
    pub id: u64,
    pub command: String,
    /// The command's resolved arguments, shared with every listing of the history.
    pub args: Arc<Value>,
    /// Who made it, as `name#connection`.
    pub client: String,
    /// The name of the client that made it, and its role.
    pub author: (String, Role),
    pub changes: Vec<Change>,
    /// The active part's id before the step and after it (decision 0033): an undo makes the
    /// first active again and a redo the second, so undoing a duplicate goes back to the part
    /// it copied. Selecting a part is never a step of its own.
    pub active: (Option<u64>, Option<u64>),
}

impl Entry {
    /// A step made by `origin`. Its id is set when it is pushed.
    pub fn new(origin: &Origin, command: &str, args: Value, changes: Vec<Change>) -> Self {
        Self {
            id: 0,
            command: command.to_string(),
            args: Arc::new(args),
            client: origin.label(),
            author: (origin.client.clone(), origin.role),
            changes,
            active: (None, None),
        }
    }

    /// Moves the document across this step: forward (redo) or back (undo). A part edited in
    /// place is stepped by its own store, which holds exactly this step on top of its undo
    /// (or redo) stack, because the stores and this list are only ever moved together.
    fn apply(&mut self, doc: &mut Document, forward: bool) {
        for change in &mut self.changes {
            match change {
                Change::Edited { part, order, .. } => {
                    if let Some(part) = doc.get_mut(part) {
                        let store = part.store_mut();
                        if forward {
                            store.redo()
                        } else {
                            store.undo()
                        };
                        if let Some(order) = order {
                            part.swap_order(order);
                        }
                    }
                }
                Change::Swapped { part, other } => *other = doc.swap(part, other.take()),
                Change::Props { part, other } => {
                    if let Some(part) = doc.get_mut(part) {
                        part.swap_props(other);
                    }
                }
            }
        }
        let (before, after) = self.active;
        doc.reselect(if forward { after } else { before });
    }
}

/// A named copy of the whole document, kept until replaced. Never evicted by the budget. Its
/// parts are shares (decision 0010), so it costs nothing until the scene moves on, and
/// restoring it brings back each part exactly: store, numbering and what it remembers.
pub(crate) struct Checkpoint {
    /// Sequence number of the command that made it.
    pub seq: u64,
    pub parts: BTreeMap<String, Part>,
}

/// One step as `history.list` shows it.
#[derive(Clone)]
pub(crate) struct Row {
    pub id: u64,
    pub command: String,
    pub args: Arc<Value>,
    pub client: String,
    /// False if the step is undone and can be redone.
    pub done: bool,
    /// What dropping it and every older step frees.
    pub bytes: u64,
}

/// The history as `history.list` and `scene.describe` show it.
#[derive(Clone)]
pub(crate) struct Listing {
    /// Every step, oldest first.
    pub steps: Vec<Row>,
    pub undoable: usize,
    pub redoable: usize,
    /// What the steps hold that the scene does not.
    pub bytes: u64,
    pub budget_bytes: u64,
    /// What only checkpoints hold.
    pub checkpoint_bytes: u64,
    /// Checkpoint names and the sequence numbers that saved them.
    pub checkpoints: Vec<(String, u64)>,
}

/// Which way the history moved last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Moved {
    Undo,
    Redo,
}

/// The history's two ends, as `session.state` names them: the step an undo would undo and the
/// one a redo would redo, each by its command and arguments, and which way it moved last.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ends {
    pub undo: Option<(String, Arc<Value>)>,
    /// The id of the step an undo would undo, as `history.list` gives it.
    pub undo_id: Option<u64>,
    pub redo: Option<(String, Arc<Value>)>,
    /// The last change was an undo or a redo: no step was made since.
    pub moved: Option<Moved>,
    /// How many steps there are to undo and to redo.
    pub counts: (usize, usize),
}

/// The undo history of one session.
pub(crate) struct History {
    entries: VecDeque<Entry>,
    /// Steps `entries[..applied]` are done; the rest can be redone.
    applied: usize,
    /// Set by an undo or a redo, cleared by a new step.
    moved: Option<Moved>,
    /// The id the next step gets.
    next_id: u64,
    /// Saved checkpoints, by name. Shared with the jobs that started while they existed.
    checkpoints: BTreeMap<String, Arc<Checkpoint>>,
    /// The byte budget.
    pub budget_bytes: u64,
}

impl History {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            entries: VecDeque::new(),
            applied: 0,
            moved: None,
            next_id: 1,
            checkpoints: BTreeMap::new(),
            budget_bytes,
        }
    }

    /// Adds a step and returns its id: drops anything that could be redone, then drops the
    /// oldest steps while the history is over budget. The newest step is always kept.
    ///
    /// Dropping the steps that could be redone also clears the stores' redo stacks, where
    /// their in-place edits were: the parts in the document, and the parts this step replaced.
    pub fn push(&mut self, mut entry: Entry, doc: &mut Document) -> u64 {
        if self.entries.len() > self.applied {
            self.entries.truncate(self.applied);
            for (_, part) in doc.parts_mut() {
                part.store_mut().clear_redo();
            }
            for change in &mut entry.changes {
                if let Change::Swapped {
                    other: Some(part), ..
                } = change
                {
                    part.store_mut().clear_redo();
                }
            }
        }
        entry.id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(entry);
        self.applied = self.entries.len();
        self.moved = None;
        if self.at_most(doc) <= self.budget_bytes {
            return self.next_id - 1;
        }
        let held = self.held(doc.parts().map(|(_, p)| p));
        let (mut total, mut steps) = (held.total, held.steps.into_iter());
        while total > self.budget_bytes && self.entries.len() > 1 {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.applied -= 1;
            total -= steps.next().unwrap_or(0);
            self.forget(oldest, doc);
        }
        self.next_id - 1
    }

    /// Frees what the oldest step, just dropped, kept in the stores: each in-place edit's step,
    /// in the document's part or in a part a later step holds.
    fn forget(&mut self, oldest: Entry, doc: &mut Document) {
        for change in oldest.changes {
            let Change::Edited { step, .. } = change else {
                continue;
            };
            let held = (self.entries.iter_mut().flat_map(|e| e.changes.iter_mut())).filter_map(
                |c| match c {
                    Change::Swapped { other, .. } => other.as_deref_mut(),
                    Change::Edited { .. } | Change::Props { .. } => None,
                },
            );
            let mut stores = doc.parts_mut().map(|(_, p)| p).chain(held);
            stores.any(|part| part.store_mut().drop_step(step));
        }
    }

    /// Undoes the newest done step for `origin` (see `may_move`).
    pub fn undo(
        &mut self,
        doc: &mut Document,
        expect: Option<u64>,
        origin: &Origin,
    ) -> Result<&Entry, CommandError> {
        let index = self
            .applied
            .checked_sub(1)
            .ok_or_else(|| nothing_to("undo"))?;
        may_move(&self.entries[index], expect, origin, "newest step")?;
        self.entries[index].apply(doc, false);
        self.applied = index;
        self.moved = Some(Moved::Undo);
        Ok(&self.entries[index])
    }

    /// Redoes the oldest undone step for `origin` (see `may_move`).
    pub fn redo(
        &mut self,
        doc: &mut Document,
        expect: Option<u64>,
        origin: &Origin,
    ) -> Result<&Entry, CommandError> {
        let index = self.applied;
        let entry = self.entries.get(index).ok_or_else(|| nothing_to("redo"))?;
        may_move(entry, expect, origin, "step to redo")?;
        self.entries[index].apply(doc, true);
        self.applied = index + 1;
        self.moved = Some(Moved::Redo);
        Ok(&self.entries[index])
    }

    /// The history's two ends now, for `session.state`.
    pub fn ends(&self) -> Ends {
        let step = |e: &Entry| (e.command.clone(), Arc::clone(&e.args));
        let undo = self
            .applied
            .checked_sub(1)
            .and_then(|i| self.entries.get(i));
        Ends {
            undo: undo.map(step),
            undo_id: undo.map(|e| e.id),
            redo: self.entries.get(self.applied).map(step),
            moved: self.moved,
            counts: (self.applied, self.entries.len() - self.applied),
        }
    }

    /// A bound on what the steps hold that is cheap to find, with no sort: every store's undo
    /// and redo steps and every replaced part's columns, a shared allocation counted each time
    /// it is held. Only a history this bound puts over budget needs the exact count.
    fn at_most(&self, doc: &Document) -> u64 {
        let replaced = (self.entries.iter().flat_map(|e| &e.changes)).filter_map(|c| match c {
            Change::Swapped {
                other: Some(part), ..
            } => Some(part),
            Change::Edited { .. } | Change::Swapped { .. } | Change::Props { .. } => None,
        });
        let orders: usize = (self.entries.iter().flat_map(|e| &e.changes))
            .filter_map(|c| match c {
                Change::Edited { order, .. } => order.as_ref().map(Order::bytes),
                Change::Swapped { .. } | Change::Props { .. } => None,
            })
            .sum();
        let steps: usize = doc
            .parts()
            .map(|(_, p)| p.store().history_bytes())
            .sum::<usize>()
            + orders;
        let parts: usize = replaced
            .map(|p| {
                let mut levels = 0;
                p.other_allocations(|a| levels += a.bytes);
                p.store().mesh_bytes() + p.store().history_bytes() + levels
            })
            .sum();
        (steps + parts) as u64
    }

    /// What the steps and checkpoints hold beside `scene`, the scene's parts as committed.
    fn held<'p>(&'p self, scene: impl Iterator<Item = &'p Part>) -> Held {
        let saved = (self.checkpoints.values()).flat_map(|c| c.parts.values());
        held(self.entries.iter(), saved, scene)
    }

    /// The history as `history.list` shows it, beside `scene`, the scene's parts as committed.
    pub fn listing<'p>(&'p self, scene: impl Iterator<Item = &'p Part>) -> Listing {
        let held = self.held(scene);
        let steps = (self.entries.iter().zip(held.steps).enumerate())
            .map(|(i, (entry, bytes))| Row {
                id: entry.id,
                command: entry.command.clone(),
                args: Arc::clone(&entry.args),
                client: entry.client.clone(),
                done: i < self.applied,
                bytes,
            })
            .collect();
        Listing {
            steps,
            undoable: self.applied,
            redoable: self.entries.len() - self.applied,
            bytes: held.total,
            budget_bytes: self.budget_bytes,
            checkpoint_bytes: held.checkpoints,
            checkpoints: (self.checkpoints.iter())
                .map(|(name, c)| (name.clone(), c.seq))
                .collect(),
        }
    }

    /// What a job reads of the history: the listing and the checkpoints as they are now.
    pub fn snapshot(&self, doc: &Document) -> Snapshot {
        Snapshot {
            listing: self.listing(doc.parts().map(|(_, p)| p)),
            checkpoints: self.checkpoints.clone(),
            ends: self.ends(),
        }
    }

    /// Saves the document under `name`, replacing an older checkpoint of that name.
    pub fn checkpoint(&mut self, name: &str, seq: u64, doc: &Document) {
        let parts = (doc.parts())
            .map(|(name, part)| (name.clone(), part.share()))
            .collect();
        let saved = Arc::new(Checkpoint { seq, parts });
        self.checkpoints.insert(name.to_string(), saved);
    }

    /// A checkpoint by name.
    pub fn checkpoint_named(&self, name: &str) -> Result<&Checkpoint, CommandError> {
        named(&self.checkpoints, name)
    }
}

/// The history as a job sees it: as it was when the job started, read-only.
pub(crate) struct Snapshot {
    pub listing: Listing,
    checkpoints: BTreeMap<String, Arc<Checkpoint>>,
    /// The history's two ends, for `session.state`.
    pub ends: Ends,
}

impl Snapshot {
    pub fn checkpoint_named(&self, name: &str) -> Result<&Checkpoint, CommandError> {
        named(&self.checkpoints, name)
    }
}

fn named<'c>(
    checkpoints: &'c BTreeMap<String, Arc<Checkpoint>>,
    name: &str,
) -> Result<&'c Checkpoint, CommandError> {
    checkpoints.get(name).map(|c| &**c).ok_or_else(|| {
        errors::not_found("checkpoint", name, checkpoints.keys().map(String::as_str))
    })
}

fn nothing_to(action: &str) -> CommandError {
    errors::unavailable(
        crate::Unavailable::new(format!("there is nothing to {action}"))
            .fix("run history.list to see the steps"),
    )
}

/// Whether `origin` may undo or redo `entry` (the `which` step). With `expect`, only if it is
/// that step. Without, a human may move any step, and anyone else only steps made under their
/// own client name and role, so an agent never silently undoes a human's stroke.
fn may_move(
    entry: &Entry,
    expect: Option<u64>,
    origin: &Origin,
    which: &str,
) -> Result<(), CommandError> {
    let made_by = format!("{} ({}, by {})", entry.id, entry.command, entry.client);
    match expect {
        Some(id) if id != entry.id => Err(CommandError {
            hint: Some("run history.list to see the steps, then decide again".into()),
            ..error(
                ErrorCode::Stale,
                format!("the {which} is {made_by}, not {id}"),
            )
        }),
        Some(_) => Ok(()),
        None if origin.role == Role::Human => Ok(()),
        None if entry.author == (origin.client.clone(), origin.role) => Ok(()),
        None if entry.author.1 == Role::Human => Err(CommandError {
            hint: Some(
                "run history.list to see the steps; a human's step is theirs to undo".into(),
            ),
            ..error(
                ErrorCode::Stale,
                format!("the {which} is {made_by}, a human's step, not one of yours"),
            )
        }),
        None => Err(CommandError {
            hint: Some(format!(
                "run history.list to see the steps; to move step {} anyway, pass expect={}",
                entry.id, entry.id
            )),
            ..error(
                ErrorCode::Stale,
                format!("the {which} is {made_by}, another client's step, not one of yours"),
            )
        }),
    }
}
