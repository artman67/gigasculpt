//! The document (named parts, each a gs-core `MeshStore`) and the write guards every edit goes
//! through.
//!
//! Commands never touch undo. The first write to a part in a call opens a mesh-store history
//! step; closing the call closes it, so undo and redo swap node columns back and forth
//! (copy-on-write, decision 0001). Adding, removing or replacing a part moves whole parts in
//! and out of the document instead. Observation commands read parts as `PlainMesh` exported
//! from the store, in the part's own vertex numbering (see `order`).
//!
//! A part copies by sharing (decision 0010): [`Part::share`] shares every node column of its
//! store, its numbering and its properties, so jobs and checkpoints cost reference counts, not
//! exports. The transaction records what each call reads ([`Reads`]), so a job can tell
//! whether what it read is still there when it lands.
//!
//! Where things live: `part` holds a part and what it remembers, `txn` the transaction and its
//! write guard, `commit` what the transaction touched turned into history changes and the
//! effect, `list` the part list and how references resolve, `active` the active part, `order`
//! the numbering a part shows outside its store, `sound` the rules a change must keep,
//! `changed` what an in-place edit changed.

mod active;
mod changed;
mod commit;
mod kept;
mod list;
mod order;
mod part;
mod sound;
mod txn;

use std::collections::BTreeMap;
use std::sync::Arc;

use gs_protocol::{CommandError, PartRef, PlainMesh};

use changed::Changed;
pub(crate) use commit::PartChange;
pub(crate) use list::unfit;
pub use order::{Numbering, VertexOrder};
pub(crate) use order::{Order, resort_spatial};
pub(crate) use part::{
    ExportTransform, Isolate, Isolation, Operation, Part, Props, Showing, scene_diagonal,
};
pub(crate) use txn::Reads;
pub use txn::{DocTxn, PartMut};

use crate::errors;

/// The scene: named parts, listed by rank, then by stable id (see `list`).
#[derive(Default)]
pub struct Document {
    parts: BTreeMap<String, Box<Part>>,
    /// How many part ids the document has handed out; the next is one more.
    issued: u64,
    /// What Isolate chose: session state, in no step (decision 0029, section 9).
    isolation: Isolation,
    /// The active part's id: session state, in no step (decision 0033). See `active`.
    active: Option<u64>,
    /// Where the active part sat in the list (its rank and id) when a call took it away, until
    /// the call ends: the part after it takes its place.
    gone: Option<(u64, u64)>,
}

impl Document {
    /// A document of these parts, in list order, whose id counter has handed out `issued` ids
    /// (a scene file's). A part without an id gets the next one, as it would entering by
    /// [`Document::swap`]; the counter never falls below an id a part holds.
    pub(crate) fn of(parts: impl IntoIterator<Item = (String, Part)>, issued: u64) -> Document {
        let parts: Vec<(String, Part)> = parts.into_iter().collect();
        let held = parts.iter().filter_map(|(_, p)| p.props.id).max();
        let mut doc = Document {
            parts: BTreeMap::new(),
            issued: issued.max(held.unwrap_or(0)),
            ..Document::default()
        };
        for (name, part) in parts {
            doc.swap(&name, Some(Box::new(part)));
        }
        doc
    }

    /// A copy that shares every part (see [`Part::share`]).
    pub(crate) fn share(&self) -> Document {
        let parts = (self.parts.iter())
            .map(|(name, part)| (name.clone(), Box::new(part.share())))
            .collect();
        Document {
            parts,
            issued: self.issued,
            isolation: self.isolation.clone(),
            active: self.active,
            gone: self.gone,
        }
    }

    /// Names of every part, sorted.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    pub(crate) fn len(&self) -> usize {
        self.parts.len()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Part> {
        self.parts.get(name).map(|p| &**p)
    }

    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut Part> {
        self.parts.get_mut(name).map(|p| &mut **p)
    }

    /// Every part, by name.
    pub(crate) fn parts(&self) -> impl Iterator<Item = (&String, &Part)> {
        self.parts.iter().map(|(name, part)| (name, &**part))
    }

    /// Every part, by name, writable.
    pub(crate) fn parts_mut(&mut self) -> impl Iterator<Item = (&String, &mut Part)> {
        self.parts
            .iter_mut()
            .map(|(name, part)| (name, &mut **part))
    }

    /// Every part as plain mesh data in the scene, by name, as `part.get` shows it.
    pub(crate) fn views(&self) -> BTreeMap<String, Arc<PlainMesh>> {
        (self.parts.iter())
            .map(|(name, part)| (name.clone(), part.world()))
            .collect()
    }

    /// Puts `part` under `name`, or removes the name for `None`, and returns what was there. A
    /// part entering the document for the first time gets the next stable id and the largest
    /// rank there is, so it goes last in the list (decision 0028), and becomes the active part
    /// (decision 0033).
    pub(crate) fn swap(&mut self, name: &str, part: Option<Box<Part>>) -> Option<Box<Part>> {
        let Some(mut part) = part else {
            let removed = self.parts.remove(name);
            if let Some(part) = &removed {
                self.left(part);
            }
            return removed;
        };
        if part.props.id.is_none() {
            self.issued += 1;
            part.props.id = Some(self.issued);
            part.props.rank = self.parts.values().map(|p| p.props.rank).max().unwrap_or(0);
            self.set_active(Some(self.issued));
        }
        self.parts.insert(name.to_string(), part)
    }

    /// The id the next part entering the document gets. A part shown for the length of a
    /// gesture (a Ctrl drag's copy, decision 0027, section 14) holds it without taking it, so
    /// the part the gesture's call adds gets the same one.
    pub(crate) fn next_id(&self) -> u64 {
        self.issued + 1
    }
}

/// The name of a part argument that the engine pinned before the command ran. Missing means
/// the command's `PART` flag does not match its arguments: a bug, reported as `E_INTERNAL`.
pub(crate) fn pinned(part: Option<PartRef>) -> Result<String, CommandError> {
    part.map(|PartRef(name)| name)
        .ok_or_else(|| errors::internal("the engine did not pin the part argument"))
}
