//! The transaction every call's writes go through ([`DocTxn`]) and its write guard on one part
//! ([`PartMut`]). The transaction records what each call reads ([`Reads`]), so a job can tell
//! whether what it read is still there when it lands (decision 0010), and hands what it
//! touched to the commit (`commit`) when it finishes.

use std::cell::RefCell;
use std::collections::btree_map::Entry as Slot;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::Arc;

use gs_core::transform::Similarity;
use gs_core::{ColumnSet, Levels, MeshStore, NodeId, Writer};
use gs_geo::voxel::Kept;
use gs_protocol::{CommandError, ErrorCode, PlainMesh};

use super::changed::{self, Changed};
use super::commit::{PartChange, commit_edit, commit_props, commit_swap};
use super::{Document, ExportTransform, Isolation, Numbering, Part, Props, VertexOrder, sound};
use crate::errors::{self, error};
use crate::mesh::store_diagonal;

/// How a transaction has touched a part so far.
enum Touch {
    /// Edited in place, inside an open mesh-store step: the nodes written, ascending.
    Edited(Vec<NodeId>),
    /// Added, removed or replaced: the part as it was before.
    Swapped(Option<Box<Part>>),
    /// Its properties changed in place and nothing else: the properties it had before.
    Props(Props),
}

/// What a call read of the document, for telling whether a job's result still applies when
/// it lands (decision 0010). Writes count as reads.
#[derive(Default)]
pub(crate) struct Reads {
    /// Parts read or written, by name, whether or not they existed.
    pub parts: BTreeSet<String>,
    /// The list of part names (which parts exist).
    pub names: bool,
    /// The whole document.
    pub all: bool,
    /// The document's id counter when the call first handed out a part id (decision 0015): a
    /// job that added a part lands only if no other call handed one out meanwhile.
    pub ids: Option<u64>,
}

/// Guarded access to the document while one command (or one batch) runs. Every write is
/// recorded, so undo and rollback need no code in the command, and every read, so a job knows
/// what it depended on.
pub struct DocTxn<'a> {
    doc: &'a mut Document,
    touched: BTreeMap<String, Touch>,
    reads: RefCell<Reads>,
    /// The id counter when the transaction began; a rollback puts it back, so a failed call
    /// (which is not logged) hands out no id a replay would not.
    issued: u64,
    /// How many writes went through the guards, so the executor can tell which command wrote.
    pub(crate) writes: u64,
    /// The document's isolation before the call first changed it; a rollback puts it back.
    isolation: Option<Isolation>,
    /// The active part when the transaction began (decision 0033); a rollback puts it back.
    active: (Option<u64>, Option<(u64, u64)>),
}

/// A part, writable, inside a transaction. Reads go to its store; writes go through
/// [`PartMut::write_positions`] or [`PartMut::sculpt`], which the transaction records for undo
/// and for the effect.
pub struct PartMut<'t> {
    part: &'t mut Part,
    /// The nodes written so far, when the part is edited in place (a part added in this
    /// transaction needs no record: undo removes it whole).
    written: Option<&'t mut Vec<NodeId>>,
}

impl Deref for PartMut<'_> {
    type Target = MeshStore;

    fn deref(&self) -> &MeshStore {
        self.part.store()
    }
}

impl PartMut<'_> {
    /// Every node, ascending.
    pub fn all_nodes(&self) -> Vec<NodeId> {
        (0..self.part.store().node_count() as NodeId).collect()
    }

    /// The part's vertex numbering, as commands and observations see it.
    pub fn numbering(&self) -> Numbering {
        self.part.numbering()
    }

    /// A write guard over the positions of `nodes` (ascending, distinct). Normals, bounds and
    /// the spatial tree follow when it drops.
    pub fn write_positions(&mut self, nodes: &[NodeId]) -> Writer<'_> {
        self.note(nodes);
        self.part.store_mut().write(nodes, ColumnSet::POSITION)
    }

    /// Hands the store to gs-core code that writes through gs-core's own guards (the stroke
    /// pipeline), inside the part's open step. `f` returns the nodes it wrote, which the
    /// transaction checks and reports like any other write.
    pub(crate) fn sculpt<R>(&mut self, f: impl FnOnce(&mut MeshStore) -> (R, Vec<NodeId>)) -> R {
        let (out, nodes) = f(self.part.store_mut());
        self.note(&nodes);
        out
    }

    fn note(&mut self, nodes: &[NodeId]) {
        if let Some(written) = self.written.as_deref_mut() {
            written.extend_from_slice(nodes);
            written.sort_unstable();
            written.dedup();
        }
    }
}

impl<'a> DocTxn<'a> {
    pub(crate) fn new(doc: &'a mut Document) -> Self {
        Self {
            issued: doc.issued,
            active: doc.active_state(),
            doc,
            touched: BTreeMap::new(),
            reads: RefCell::default(),
            writes: 0,
            isolation: None,
        }
    }

    /// A transaction over an in-place edit made before it began, committed like any edit. The
    /// window's pen stroke is made this way, sample by sample between calls: `part`'s store has
    /// its step open and `written` (ascending) are the nodes written. A gizmo drag that moved
    /// the whole part changed only its properties: `props` are the ones it had before.
    pub(crate) fn adopt(
        doc: &'a mut Document,
        part: &str,
        written: Vec<NodeId>,
        props: Option<Props>,
    ) -> Self {
        let touch = props.map_or(Touch::Edited(written), Touch::Props);
        let mut txn = Self::new(doc);
        txn.touched.insert(part.to_string(), touch);
        txn
    }

    fn read(&self, name: &str) {
        self.reads.borrow_mut().parts.insert(name.to_string());
    }

    /// Names of every part, sorted.
    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.reads.borrow_mut().names = true;
        self.doc.names()
    }

    /// The name of the part a reference means: `#12` by stable id, `@0` by place in the list,
    /// otherwise by name (decision 0015). Records what it read: a place reads the whole list.
    pub(crate) fn resolve(&self, part: &str) -> Option<String> {
        if part.starts_with('@') {
            self.reads.borrow_mut().names = true;
        }
        let name = self.doc.resolve(part).map(str::to_string);
        self.read(name.as_deref().unwrap_or(part));
        name
    }

    /// A part's stable id and place in the list.
    pub(crate) fn place(&self, name: &str) -> Option<(u64, usize)> {
        self.reads.borrow_mut().names = true;
        self.doc.place(name)
    }

    /// True if the part `part` refers to exists (see [`DocTxn::resolve`]).
    pub fn contains(&self, part: &str) -> bool {
        self.resolve(part).is_some()
    }

    /// A part as plain mesh data in the scene, as `part.get` shows it (decision 0029).
    /// `E_NOT_FOUND` with suggestions if it does not exist.
    pub fn part(&self, name: &str) -> Result<Arc<PlainMesh>, CommandError> {
        self.find(name).map(Part::world)
    }

    /// A part as plain mesh data in its own space, for a command that rebuilds it and keeps its
    /// transform (decision 0029, section 4).
    pub(crate) fn local(&self, name: &str) -> Result<Arc<PlainMesh>, CommandError> {
        self.find(name).map(Part::view)
    }

    /// Where a part's points are in the scene.
    pub(crate) fn transform(&self, name: &str) -> Result<Similarity, CommandError> {
        self.find(name).map(Part::transform)
    }

    /// A part's bounding-box diagonal in the scene, which percentage lengths are taken of.
    /// Reads the store's node bounds where it can, so it costs nothing like an export.
    pub fn diagonal(&self, name: &str) -> Result<f32, CommandError> {
        self.find(name).map(Part::world_diagonal)
    }

    /// A part's bounding-box diagonal in its own space, for tolerances that scale with the part
    /// as its store holds it (snapping, mirror matching).
    pub(crate) fn part_diagonal(&self, name: &str) -> Result<f32, CommandError> {
        self.find(name).map(|part| store_diagonal(part.store()))
    }

    /// A part's unit vertex normals in the scene, indexed like `part`'s vertices. Reads the
    /// store's normals, so it costs no export.
    pub fn normals(&self, name: &str) -> Result<Vec<[f32; 3]>, CommandError> {
        let part = self.find(name)?;
        let (numbering, store, t) = (part.numbering(), part.store(), part.transform());
        let mut normals = vec![[0.0; 3]; store.vertex_count()];
        for n in 0..store.node_count() as NodeId {
            for (local, normal) in (0..).zip(store.node(n).nrm) {
                normals[numbering.index(n, local) as usize] = t.direction(*normal).to_array();
            }
        }
        Ok(normals)
    }

    fn find(&self, part: &str) -> Result<&Part, CommandError> {
        let name = self.resolve(part);
        (name.and_then(|name| self.doc.get(&name)))
            .ok_or_else(|| errors::not_found("part", part, self.part_names()))
    }

    /// The cell size Voxel Remesh kept for a part, if it has remeshed it.
    pub fn voxel_cell(&self, name: &str) -> Result<Option<Kept>, CommandError> {
        self.find(name).map(|part| part.props.voxel)
    }

    /// How to write a part back to the file it came from, if it came from one.
    pub(crate) fn export_transform(
        &self,
        name: &str,
    ) -> Result<Option<ExportTransform>, CommandError> {
        self.find(name).map(|part| part.props.export)
    }

    /// A part, writable. The first write in a call opens its undo step.
    pub fn write_part(&mut self, part: &str) -> Result<PartMut<'_>, CommandError> {
        let Some(name) = self.resolve(part) else {
            return Err(errors::not_found("part", part, self.part_names()));
        };
        let name = name.as_str();
        self.writes += 1;
        let Self { doc, touched, .. } = self;
        let part = doc
            .get_mut(name)
            .ok_or_else(|| errors::internal("part vanished"))?;
        let touch = touched.entry(name.to_string()).or_insert_with(|| {
            part.store_mut().begin_step();
            Touch::Edited(Vec::new())
        });
        if let Touch::Props(props) = touch {
            // New properties, then an edit: the part as it was is a share of it with the old
            // properties, and the call replaces it.
            let mut before = part.share();
            before.props = *props;
            *touch = Touch::Swapped(Some(Box::new(before)));
        }
        let written = match touch {
            Touch::Edited(nodes) => Some(nodes),
            Touch::Swapped(_) | Touch::Props(_) => None,
        };
        Ok(PartMut { part, written })
    }

    /// Runs `edit`, gs-core code that writes through gs-core's own guards and returns the nodes
    /// it wrote, on a part's store as this call's write, and gives the part back to read.
    pub(crate) fn edit_store(
        &mut self,
        part: &str,
        edit: impl FnOnce(&mut MeshStore) -> Vec<NodeId>,
    ) -> Result<PartMut<'_>, CommandError> {
        let mut store = self.write_part(part)?;
        store.sculpt(|s| ((), edit(s)));
        Ok(store)
    }

    /// Adds a part made from plain mesh data with valid indices, replacing any part with that
    /// name. Refuses meshes the store cannot hold, or that break the rules in `sound`.
    pub fn insert_part(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        order: VertexOrder,
    ) -> Result<(), CommandError> {
        let part = Part::new(mesh, order, name)?;
        self.swap_part(name, Some(Box::new(part)));
        Ok(())
    }

    /// Replaces a part with a voxel-remeshed mesh placed by `transform` (decision 0029),
    /// keeping the cell size for its next remesh and the rest of what the part remembers.
    pub fn insert_remeshed(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        (kept, transform): (Kept, Similarity),
    ) -> Result<(), CommandError> {
        if self.contains(name) {
            self.single_level(name, "a remesh")?;
        }
        self.put_remeshed(name, mesh, (Some(kept), transform))
    }

    /// Replaces part `name` with its own voxel remesh or decimate, as
    /// [`DocTxn::insert_remeshed`] does, keeping its frozen levels (decision 0030, section 2) for
    /// `level.unfreeze` to project. With no cell size (decimate), the part keeps the one it had.
    pub(crate) fn remesh_in_place<K: Into<Option<Kept>>>(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        (kept, transform): (K, Similarity),
    ) -> Result<(), CommandError> {
        self.single_level_or_frozen(name, "voxel remesh")?;
        self.put_remeshed(name, mesh, (kept.into(), transform))
    }

    fn put_remeshed(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        (kept, transform): (Option<Kept>, Similarity),
    ) -> Result<(), CommandError> {
        // The store's own order, UVs included: they turn with the faces the build turns.
        let mut part = Part::new(mesh, VertexOrder::Spatial, name)?;
        let old = self.find(name).ok();
        let props = old.map(Part::props).unwrap_or_default();
        part.props = Props {
            voxel: kept.or(props.voxel),
            transform,
            ..props
        };
        let record = old.and_then(Part::frozen).map(Levels::share);
        self.swap_part(name, Some(Box::new(part.with_frozen(record))));
        Ok(())
    }

    /// Replaces a part with a rebuilt mesh of new topology from `what` (a command that works on
    /// one level), keeping the rest of what the part remembers.
    pub(crate) fn insert_rebuilt(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        what: &str,
    ) -> Result<(), CommandError> {
        self.single_level(name, what)?;
        let mut part = Part::new(mesh, VertexOrder::Spatial, name)?;
        part.props = self.find(name).map(Part::props).unwrap_or_default();
        self.swap_part(name, Some(Box::new(part)));
        Ok(())
    }

    /// The part named `name`, read-only, for commands that build a changed copy of it (detail
    /// levels). `E_NOT_FOUND` with suggestions if it does not exist.
    pub(crate) fn get_part(&self, name: &str) -> Result<&Part, CommandError> {
        self.find(name)
    }

    /// Puts `part` in place of the part named `name`. A level change (decision 0014) is one
    /// undo step that swaps the whole part.
    pub(crate) fn replace_part(&mut self, name: &str, part: Part) {
        self.read(name);
        self.swap_part(name, Some(Box::new(part)));
    }

    /// Refuses with `E_PRECONDITION` when part `name` has detail levels, which `what` (a
    /// topology command) cannot keep, or frozen levels, which it would drop: it names the part
    /// and the remedies (decisions 0014 and 0030), not `level.freeze`, whose frozen part it
    /// refuses too.
    pub fn single_level(&self, name: &str, what: &str) -> Result<(), CommandError> {
        if let Some(record) = self.find(name)?.frozen() {
            return Err(CommandError {
                hint: Some("run level.unfreeze first".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!(
                        "part '{name}' has {} frozen detail levels, which {what} would drop",
                        record.count()
                    ),
                )
            });
        }
        self.refuse_levels(name, what, false)
    }

    /// [`DocTxn::single_level`] for a command that keeps frozen levels (voxel remesh): refuses
    /// only a part with detail levels, naming `level.freeze` too.
    pub(crate) fn single_level_or_frozen(
        &self,
        name: &str,
        what: &str,
    ) -> Result<(), CommandError> {
        self.refuse_levels(name, what, true)
    }

    /// Refuses part `name` if it has detail levels, naming how to leave one and, when `what`
    /// keeps frozen levels (`freeze`), `level.freeze` first.
    fn refuse_levels(&self, name: &str, what: &str, freeze: bool) -> Result<(), CommandError> {
        let Some(levels) = self.find(name)?.levels() else {
            return Ok(());
        };
        let freeze = if freeze {
            "run level.freeze to keep every level while the base takes new topology, or "
        } else {
            ""
        };
        Err(CommandError {
            hint: Some(format!(
                "{freeze}run level.delete_higher at the level to keep, then level.delete_lower, \
                 so one level is left"
            )),
            ..error(
                ErrorCode::Precondition,
                format!(
                    "part '{name}' has {} detail levels, and {what} works on a part with one level",
                    levels.count()
                ),
            )
        })
    }

    /// Adds a part with detail levels (a ZTL file's part, decision 0014), replacing any part
    /// with that name. `stored` are the meshes it was built from, checked first (see `sound`).
    pub(crate) fn insert_levels(
        &mut self,
        name: &str,
        levels: Levels,
        stored: &[&PlainMesh],
    ) -> Result<(), CommandError> {
        for mesh in stored {
            sound::check_mesh(mesh, name)?;
        }
        self.swap_part(
            name,
            Some(Box::new(Part::leveled(levels, Props::default()))),
        );
        Ok(())
    }

    /// Moves every point of a part this call added, every detail level included (an import's
    /// fit). Only for a part the call made: an edit of an existing part goes through
    /// [`DocTxn::write_part`].
    pub(crate) fn map_new_part(
        &mut self,
        name: &str,
        f: impl Fn(gs_core::Vec3) -> gs_core::Vec3 + Sync,
    ) -> Result<(), CommandError> {
        let made_here = matches!(self.touched.get(name), Some(Touch::Swapped(_)));
        let part = self.doc.get_mut(name).filter(|_| made_here);
        let part = part.ok_or_else(|| errors::internal("only a part the call made moves whole"))?;
        part.map_positions(f);
        Ok(())
    }

    /// Writes part `name`'s transform into every level's points and sets it to the identity
    /// (decision 0029, section 5), as a swap of the whole part: each point becomes, bit for bit,
    /// the scene point `part.get` showed. False, with nothing changed, at the identity.
    pub(crate) fn bake(&mut self, name: &str) -> Result<bool, CommandError> {
        self.read(name);
        let part = self.find(name)?;
        let placed = part.transform();
        if placed.is_identity() {
            return Ok(false);
        }
        let mut baked = part.share_with(part.props().at_identity());
        baked.map_positions(|p| if p.is_dead() { p } else { placed.point(p) });
        self.swap_part(name, Some(Box::new(baked)));
        Ok(true)
    }

    /// Sets how to write a part back to its file. Only for a part this call added or replaced:
    /// what a part remembers changes with the part, never on its own.
    pub(crate) fn set_export(
        &mut self,
        name: &str,
        export: ExportTransform,
    ) -> Result<(), CommandError> {
        let made_here = matches!(self.touched.get(name), Some(Touch::Swapped(_)));
        match self.doc.get_mut(name) {
            Some(part) if made_here => {
                part.props.export = Some(export);
                Ok(())
            }
            _ => Err(errors::internal(
                "an export transform can only be set on a part the call made",
            )),
        }
    }

    /// Removes a part.
    pub(crate) fn remove_part(&mut self, name: &str) {
        self.read(name);
        self.swap_part(name, None);
    }

    /// Brings a part back to `saved` (a checkpoint's copy), unless it already is.
    pub(crate) fn restore_part(&mut self, name: &str, saved: &Part) {
        self.read(name);
        if (self.doc.get(name)).is_some_and(|part| part.same_as(saved)) {
            return;
        }
        self.swap_part(name, Some(Box::new(saved.share())));
    }

    fn swap_part(&mut self, name: &str, part: Option<Box<Part>>) {
        self.writes += 1;
        let issued = self.doc.issued;
        let mut old = self.doc.swap(name, part);
        if self.doc.issued != issued {
            self.reads.get_mut().ids.get_or_insert(issued);
        }
        match self.touched.entry(name.to_string()) {
            Slot::Vacant(slot) => {
                slot.insert(Touch::Swapped(old));
            }
            Slot::Occupied(mut slot) => match slot.get() {
                // Edited in place earlier in this call: the part as it was before is the store
                // with that step cancelled.
                Touch::Edited(_) => {
                    if let Some(part) = &mut old {
                        part.store_mut().cancel_step();
                    }
                    slot.insert(Touch::Swapped(old));
                }
                // New properties earlier in this call: the part as it was had the old ones.
                Touch::Props(props) => {
                    if let Some(part) = &mut old {
                        part.props = *props;
                    }
                    slot.insert(Touch::Swapped(old));
                }
                // Swapped earlier: keep the first part; the ones made in between are dropped.
                Touch::Swapped(_) => {}
            },
        }
    }

    /// Gives part `name` the properties `props` in place (decision 0029, section 8): its store,
    /// pending drawing work and upload identity stay. A part this call already replaced takes
    /// them as part of that replacement.
    pub(crate) fn set_props(&mut self, name: &str, props: Props) -> Result<(), CommandError> {
        let Some(part) = self.doc.get_mut(name) else {
            return Err(errors::not_found("part", name, self.part_names()));
        };
        self.reads.get_mut().parts.insert(name.to_string());
        if part.props == props {
            return Ok(());
        }
        self.writes += 1;
        match self.touched.get(name) {
            None => {
                self.touched
                    .insert(name.to_string(), Touch::Props(part.props));
                part.props = props;
            }
            Some(Touch::Props(_) | Touch::Swapped(_)) => part.props = props,
            Some(Touch::Edited(_)) => {
                let copy = part.share_with(props);
                self.swap_part(name, Some(Box::new(copy)));
            }
        }
        Ok(())
    }

    /// Every part as it was when the transaction began, for counting what the history holds:
    /// the part it replaced or removed where it did, else the document's, whose store's open
    /// step holds its start (a part given new properties shows its new ones).
    pub(crate) fn committed(&self) -> Vec<&Part> {
        let swapped = |name: &str| matches!(self.touched.get(name), Some(Touch::Swapped(_)));
        let mut parts: Vec<&Part> = (self.doc.parts())
            .filter(|(name, _)| !swapped(name))
            .map(|(_, part)| part)
            .collect();
        parts.extend(self.touched.values().filter_map(|t| match t {
            Touch::Swapped(before) => before.as_deref(),
            Touch::Edited(_) | Touch::Props(_) => None,
        }));
        parts
    }

    /// The whole document, read-only.
    pub(crate) fn document(&self) -> &Document {
        self.reads.borrow_mut().all = true;
        self.doc
    }

    /// Changes the document's isolation by `change` (decision 0029, section 9). It is session
    /// state, so this is not a write and no step keeps it, but a rollback puts it back.
    pub(crate) fn isolate(&mut self, change: impl FnOnce(&mut Isolation)) {
        self.isolation
            .get_or_insert_with(|| self.doc.isolation.clone());
        change(&mut self.doc.isolation);
    }

    /// The active part's name (decision 0033); `None` only in an empty scene. Reads the list.
    pub(crate) fn active(&self) -> Option<String> {
        self.reads.borrow_mut().names = true;
        self.doc.active().map(str::to_string)
    }

    /// Makes part `name` the active part (decision 0033). It is session state, so this is not a
    /// write and no step keeps it, but a rollback puts it back.
    pub(crate) fn select(&mut self, name: &str) {
        self.doc.select(name);
    }

    /// The whole document, writable without recording undo. Only for undo and redo, which move
    /// along steps the history already holds.
    pub(crate) fn document_mut(&mut self) -> &mut Document {
        self.reads.get_mut().all = true;
        self.doc
    }

    /// Ends the transaction, keeping its writes, and returns the parts that really changed and
    /// what the call read. Refuses, rolling everything back, if a change breaks a rule in
    /// `sound`. A part written back unchanged keeps no step.
    pub(crate) fn finish(self) -> Result<(Vec<PartChange>, Reads), CommandError> {
        let mut found = match self.check() {
            Ok(found) => found,
            Err(err) => {
                self.rollback();
                return Err(err);
            }
        };
        let mut changed = Vec::new();
        for (name, touch) in self.touched {
            changed.extend(match touch {
                Touch::Edited(_) => commit_edit(self.doc, &name, found.remove(&name)),
                Touch::Swapped(before) => commit_swap(self.doc, &name, before),
                Touch::Props(before) => commit_props(self.doc, &name, before),
            });
        }
        self.doc.settle();
        Ok((changed, self.reads.into_inner()))
    }

    /// Checks every changed part; returns what each in-place edit changed.
    fn check(&self) -> Result<BTreeMap<String, Changed>, CommandError> {
        let mut found = BTreeMap::new();
        for (name, touch) in &self.touched {
            let Some(part) = self.doc.get(name) else {
                continue;
            };
            match touch {
                Touch::Edited(nodes) => {
                    let changed = changed::of(part.store(), nodes, &part.transform());
                    sound::check_moved(part.store(), &changed, name)?;
                    found.insert(name.clone(), changed);
                }
                Touch::Swapped(_) => sound::check_store(part.store(), name)?,
                Touch::Props(_) => {}
            }
            sound::check_placed(part.store(), &part.transform(), name)?;
        }
        Ok(found)
    }

    /// Ends the transaction, undoing every write.
    pub(crate) fn rollback(self) {
        self.doc.issued = self.issued;
        if let Some(isolation) = self.isolation {
            self.doc.isolation = isolation;
        }
        for (name, touch) in self.touched {
            match touch {
                Touch::Edited(_) => {
                    if let Some(part) = self.doc.get_mut(&name) {
                        part.store_mut().cancel_step();
                    }
                }
                Touch::Swapped(before) => {
                    self.doc.swap(&name, before);
                }
                Touch::Props(mut before) => {
                    if let Some(part) = self.doc.get_mut(&name) {
                        part.swap_props(&mut before);
                    }
                }
            }
        }
        // After the parts: putting back a part the call removed, or removing one it added,
        // moves the active part.
        self.doc.restore_active(self.active);
    }
}
