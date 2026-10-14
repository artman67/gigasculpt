//! How much memory the history and the checkpoints hold that the scene does not, counting
//! every allocation once (decision 0010).
//!
//! The scene, the steps and the checkpoints share node columns: a step that replaced a part
//! keeps the old part, which shares every node the edits since left alone with the scene, and
//! a checkpoint shares nodes with both. So each allocation goes to one holder only: the scene
//! if it holds it, else the newest step that holds it, else the checkpoints. A step's bytes are
//! what it holds that no newer step and not the scene holds, which is what dropping it and every
//! older step frees: the steps add up to the history's total, and the budget, which drops the
//! oldest steps first, frees exactly what they list.
//!
//! An in-place step lives in a part's mesh store and is found by its id; a step that replaced a
//! part holds the other part, whose columns it holds (its store's steps belong to the in-place
//! entries that made them). Stores are read as committed: while a call runs, its own edits are
//! not counted, so `history.list` inside a batch reads what it reads in a job, whose listing is
//! taken when the job starts.

use std::collections::BTreeMap;

use gs_core::{Allocation, StepId};

use super::store::{Change, Entry};
use crate::document::Part;

/// Who holds an allocation, in the order that decides who counts it: the checkpoints last,
/// then steps from the oldest, then a store step no entry names (a leak, counted in the total
/// so it shows), then the scene first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Holder {
    Checkpoint,
    Step(usize),
    Unowned,
    Scene,
}

/// What the history and the checkpoints hold.
pub(crate) struct Held {
    /// Each step's bytes, oldest first.
    pub steps: Vec<u64>,
    /// Everything the steps hold that the scene does not.
    pub total: u64,
    /// What only checkpoints hold.
    pub checkpoints: u64,
}

/// Counts what `entries` and `checkpoints` hold beyond the scene's parts as committed (an edit
/// in progress is not counted yet, so a call reads the history as it was when it began).
pub(crate) fn held<'h>(
    entries: impl Iterator<Item = &'h Entry> + Clone,
    checkpoints: impl Iterator<Item = &'h Part>,
    scene: impl Iterator<Item = &'h Part>,
) -> Held {
    let mut owner: BTreeMap<StepId, usize> = BTreeMap::new();
    for (i, entry) in entries.clone().enumerate() {
        for change in &entry.changes {
            if let Change::Edited { step, .. } = change {
                owner.insert(*step, i);
            }
        }
    }
    let mut found: Vec<(usize, Holder, usize)> = Vec::new();
    let mut store_of = |part: &Part, columns: Holder| {
        let store = part.store();
        store.start_allocations(|a: Allocation| found.push((a.at, columns, a.bytes)));
        store.step_allocations(|id, a| {
            let holder = owner.get(&id).map_or(Holder::Unowned, |&i| Holder::Step(i));
            found.push((a.at, holder, a.bytes));
        });
        part.other_allocations(|a| found.push((a.at, columns, a.bytes)));
    };
    for part in scene {
        store_of(part, Holder::Scene);
    }
    let count = entries.clone().count();
    let mut orders = Vec::new();
    for (i, entry) in entries.enumerate() {
        for change in &entry.changes {
            match change {
                Change::Swapped {
                    other: Some(part), ..
                } => store_of(part, Holder::Step(i)),
                Change::Edited {
                    order: Some(order), ..
                } => orders.extend(order.allocation().map(|a| (a.at, Holder::Step(i), a.bytes))),
                Change::Swapped { .. } | Change::Edited { .. } | Change::Props { .. } => {}
            }
        }
    }
    for part in checkpoints {
        store_of(part, Holder::Checkpoint);
    }
    found.extend(orders);
    // Highest holder first for each address, so the first of each run is the one that counts.
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    found.dedup_by_key(|f| f.0);
    let mut held = Held {
        steps: vec![0; count],
        total: 0,
        checkpoints: 0,
    };
    for (_, holder, bytes) in found {
        let bytes = bytes as u64;
        match holder {
            Holder::Scene => {}
            Holder::Unowned => held.total += bytes,
            Holder::Step(i) => {
                held.steps[i] += bytes;
                held.total += bytes;
            }
            Holder::Checkpoint => held.checkpoints += bytes,
        }
    }
    held
}
