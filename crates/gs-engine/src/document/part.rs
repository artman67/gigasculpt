//! One part: its mesh store or its detail levels (decision 0014), its numbering, its last
//! export and what it remembers ([`Props`]). A part copies by sharing (decision 0010):
//! [`Part::share`] shares every node column of every store it holds, its numbering and its
//! properties; [`Part::same_as`] and [`Part::is_base_of`] tell jobs and checkpoints whether a
//! share is still the part it was taken from.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use gs_core::transform::Similarity;
use gs_core::{Aabb, ColumnSet, Hit, Levels, MeshError, MeshStore, NodeId, PosRead, Vec3};
use gs_geo::voxel::Kept;
use gs_protocol::{CommandError, ErrorCode, Length, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::order::{self, Order};
use super::{Numbering, VertexOrder, kept, sound};
use crate::ctx::PartLength;
use crate::effect::Touched;
use crate::errors::{self, error};

/// The scale and offset that write a part back at the size and place its file had:
/// `position * scale + offset`, in the scene's axes. `file.import` finds it; `file.export`
/// uses it when the call gives none.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExportTransform {
    pub scale: f32,
    pub offset: [f32; 3],
}

/// What a part remembers besides its mesh. Every share carries it, so it survives jobs,
/// checkpoints and `history.restore`. Scene files store it as JSON through serde, so a field
/// added here saves and opens with no change to the file format; a field a file does not have
/// reads as its default (decision 0016).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about a part, each saved as one field of the scene file"
)]
pub(crate) struct Props {
    /// The part's stable id (decision 0015): handed out by the document when the part first
    /// enters it, never reused. `None` until then.
    pub id: Option<u64>,
    /// The cell size Voxel Remesh keeps for the part, once it has remeshed it, in the part's
    /// own units.
    #[serde(with = "kept", skip_serializing_if = "Option::is_none")]
    pub voxel: Option<Kept>,
    /// Voxel mode is on (`mesh.voxel_mode`): `mesh.voxel_update` remeshes it at its cell size.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub voxel_mode: bool,
    /// How to write it back to the file it came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportTransform>,
    /// Where it sorts in the part list: by rank, then by id (decision 0028). A part entering
    /// the document gets the largest rank there is, so it goes last.
    #[serde(skip_serializing_if = "is_default")]
    pub rank: u64,
    /// Its eye is off: it is not drawn, picked or framed, unless Isolate chose it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// How it combines with the parts above it in its Live Boolean group (decision 0057,
    /// section 5).
    #[serde(skip_serializing_if = "is_default")]
    pub operation: Operation,
    /// It starts a Live Boolean group: it and the parts below it, up to the next that starts
    /// one, combine apart from the parts above.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub start: bool,
    /// Its paint is hidden (Show Paint off, `part.set_paint_visible`, decision 0039 section
    /// 10): it draws in its material alone and its color column stays as it was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub paint_hidden: bool,
    /// Where its points are in the scene (decision 0029): the store keeps them in the part's
    /// own space.
    #[serde(skip_serializing_if = "Similarity::is_identity")]
    pub transform: Similarity,
}

/// How a part combines with the parts above it in its Live Boolean group (decision 0057,
/// section 5), in Blender's words (`docs/NAMES.md` gives the other program's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Added to what the parts above it make.
    #[default]
    Union,
    /// Cut out of what the parts above it make.
    Difference,
    /// Keeps only what it shares with what the parts above it make.
    Intersect,
}

/// What Isolate made of a part (decision 0028): it shows only the parts it chose and sets the
/// others aside, leaving every eye alone, so ending it shows exactly what showed before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Isolate {
    /// Isolate is off for the part: its eye says whether it shows.
    #[default]
    Off,
    /// Isolate chose it: it shows, whatever its eye says.
    Chosen,
    /// Isolate set it aside: it does not show.
    Aside,
}

/// What decides whether a part shows: the isolation in effect and the active part's id, which
/// always shows (decisions 0028 and 0033). See `Document::showing`.
#[derive(Clone, Copy)]
pub(crate) struct Showing<'a> {
    pub isolation: &'a Isolation,
    pub active: Option<u64>,
}

/// What Isolate chose, by stable id (decision 0029, section 9): the parts it shows and the
/// parts it set aside. The document holds it outside every part, so undo, redo, checkpoints and
/// jobs leave it alone, and a scene file keeps it in its session section. A part in neither set
/// (one made since) follows its eye.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Isolation {
    pub chosen: BTreeSet<u64>,
    pub aside: BTreeSet<u64>,
}

impl Isolation {
    /// What it made of `part`.
    pub(crate) fn mark(&self, part: &Part) -> Isolate {
        match part.props.id {
            Some(id) if self.chosen.contains(&id) => Isolate::Chosen,
            Some(id) if self.aside.contains(&id) => Isolate::Aside,
            _ => Isolate::Off,
        }
    }

    /// Gives the part with id `to` the mark of the part with id `from`.
    pub(crate) fn copy(&mut self, from: u64, to: u64) {
        for set in [&mut self.chosen, &mut self.aside] {
            if set.contains(&from) {
                set.insert(to);
            }
        }
    }
}

/// `value` is its type's default: for serde's `skip_serializing_if`, which passes a reference.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl Props {
    /// These props for the part's points moved into the scene, at the identity (a bake, a
    /// mirror copy, a merge down): what they keep in the part's own units is scaled with the
    /// points, so the kept cell size spans the same length in the scene (review 0008, m2).
    pub(crate) fn at_identity(self) -> Self {
        let scale = f64::from(self.transform.scale);
        Self {
            voxel: (self.voxel).map(|kept| Kept {
                voxel: kept.voxel * scale,
                ..kept
            }),
            transform: Similarity::IDENTITY,
            ..self
        }
    }

    /// These props for a copy of the part (`part.duplicate`, the mirror copy, a Ctrl drag's
    /// live copy): a union that starts no group, whatever the part was (decision 0057, section
    /// 5).
    pub(crate) fn as_copy(self) -> Self {
        Self {
            operation: Operation::Union,
            start: false,
            ..self
        }
    }

    /// The Live Boolean marks as bytes for the scene hash: the operation (0 union, 1
    /// difference, 2 intersect) and the start (1 on), or `None` while both have their defaults,
    /// so a scene that never used them hashes as it did before them.
    pub(crate) fn boolean_marks(&self) -> Option<[u8; 2]> {
        let operation = match self.operation {
            Operation::Union => 0,
            Operation::Difference => 1,
            Operation::Intersect => 2,
        };
        let marks = [operation, u8::from(self.start)];
        (marks != [0, 0]).then_some(marks)
    }

    /// The list order and visibility marks as bytes for the scene hash, or `None` when every
    /// one has its default, so a scene that never used them hashes as it did before them. The
    /// last byte held the Isolate mark before Isolate left the parts (decision 0029, section 9)
    /// and is 0, so a scene keeps its hash.
    pub(crate) fn list_marks(&self) -> Option<[u8; 10]> {
        if (self.rank, self.hidden) == (0, false) {
            return None;
        }
        let mut marks = [0; 10];
        marks[..8].copy_from_slice(&self.rank.to_le_bytes());
        marks[8] = u8::from(self.hidden);
        Some(marks)
    }
}

/// One part: its mesh store (or its detail levels), how its vertices are numbered outside the
/// store, its last export as plain mesh data while anything still holds that, and what it
/// remembers.
pub(crate) struct Part {
    body: Body,
    view: Mutex<Weak<PlainMesh>>,
    pub(super) props: Props,
    /// Unique to this part among every part the process makes, so the window can tell a part
    /// from another put under the same name. A share gets a new one.
    id: u64,
    /// The level stack `level.freeze` set aside, while the part is its base (decision 0030).
    frozen: Option<Box<Levels>>,
}

/// A part's mesh: one store with its numbering, or detail levels (decision 0014), numbered by
/// the active level's numbering.
#[expect(
    clippy::large_enum_variant,
    reason = "parts live boxed in the document; the plain store is the common case and not worth \
              another indirection"
)]
enum Body {
    Plain {
        store: MeshStore,
        order: order::Order,
    },
    Levels(Box<Levels>),
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, AtomicOrdering::Relaxed)
}

impl Part {
    /// A part made from plain mesh data with valid indices, checked first (see `sound`).
    pub(crate) fn new(
        mesh: PlainMesh,
        order: VertexOrder,
        name: &str,
    ) -> Result<Self, CommandError> {
        sound::check_mesh(&mesh, name)?;
        let (store, order) = order::import(mesh, order).map_err(refused)?;
        Ok(Self::of(Body::Plain { store, order }, Props::default()))
    }

    /// A new part (a new id) with no export yet.
    fn of(body: Body, props: Props) -> Self {
        let view = Mutex::new(Weak::new());
        let id = next_id();
        Self {
            body,
            view,
            props,
            id,
            frozen: None,
        }
    }

    /// A part made of detail levels, remembering `props`. A stack of one level becomes a plain
    /// part numbered by that level.
    pub(crate) fn leveled(levels: Levels, props: Props) -> Self {
        let body = if levels.count() == 1 {
            let (store, numbering) = levels.into_single();
            Body::Plain {
                store,
                order: order::Order::Given(numbering),
            }
        } else {
            Body::Levels(Box::new(levels))
        };
        Self::of(body, props)
    }

    /// A part made from a store, its numbering and its properties, as a scene file holds
    /// them (decision 0016).
    pub(crate) fn loaded(store: MeshStore, order: Order, props: Props) -> Part {
        Self::of(Body::Plain { store, order }, props)
    }

    /// A copy that shares this part's store columns (every level's), numbering, last export and
    /// properties, with no history and a new id.
    pub(crate) fn share(&self) -> Part {
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        let body = match &self.body {
            Body::Plain { store, order } => Body::Plain {
                store: store.share(),
                order: order.clone(),
            },
            Body::Levels(levels) => Body::Levels(Box::new(levels.share())),
        };
        Part {
            body,
            view: Mutex::new(view.clone()),
            props: self.props,
            id: next_id(),
            frozen: self.frozen.as_ref().map(|l| Box::new(l.share())),
        }
    }

    /// True if `other` holds this part's very columns, levels, numbering and properties: a
    /// share neither side has changed since.
    pub(crate) fn same_as(&self, other: &Part) -> bool {
        self.same_body(other) && self.props == other.props
    }

    /// True if `edited`, a share of this part whose active store was edited in place, started
    /// from this part: the active store is its step's base, and everything else is the very
    /// same. `order_then` is the numbering the edit replaced, when it changed it.
    pub(crate) fn is_base_of(&self, edited: &Part, order_then: Option<&Order>) -> bool {
        let rest = match (&self.body, &edited.body) {
            (Body::Plain { order, .. }, Body::Plain { order: o, .. }) => {
                order.same(order_then.unwrap_or(o))
            }
            (Body::Levels(a), Body::Levels(b)) => a.same_besides_active(b),
            _ => false,
        };
        rest && self.store().is_base_of(edited.store())
            && self.props == edited.props
            && self.same_frozen(edited)
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn props(&self) -> Props {
        self.props
    }

    /// Whether the part shows (see `Document::showing`): the active part always does (decision
    /// 0033), a part Isolate chose does, a part it set aside does not, any other follows its eye.
    pub(crate) fn shown(&self, showing: Showing) -> bool {
        if self.props.id.is_some() && self.props.id == showing.active {
            return true;
        }
        match showing.isolation.mark(self) {
            Isolate::Chosen => true,
            Isolate::Aside => false,
            Isolate::Off => !self.props.hidden,
        }
    }

    /// Swaps the part's properties with `props`, in place: the part keeps its store, its
    /// pending drawing work and its upload identity (decision 0029, section 8).
    pub(crate) fn swap_props(&mut self, props: &mut Props) {
        std::mem::swap(&mut self.props, props);
    }

    /// True if `other` holds this part's very columns, levels and numbering, whatever the
    /// properties.
    pub(crate) fn same_body(&self, other: &Part) -> bool {
        let body = match (&self.body, &other.body) {
            (Body::Plain { store, order }, Body::Plain { store: s, order: o }) => {
                store.same_columns(s) && order.same(o)
            }
            (Body::Levels(a), Body::Levels(b)) => a.same(b),
            _ => false,
        };
        body && self.same_frozen(other)
    }

    fn same_frozen(&self, other: &Part) -> bool {
        match (&self.frozen, &other.frozen) {
            (Some(a), Some(b)) => a.same(b),
            (a, b) => a.is_none() && b.is_none(),
        }
    }

    /// A share (see [`Part::share`]) remembering `props` instead: a list edit (order,
    /// visibility, a copy's new id) changes the part's properties and nothing else.
    pub(crate) fn share_with(&self, props: Props) -> Part {
        Part {
            props,
            ..self.share()
        }
    }

    /// Moves every point of every level by `f`, pending edits staying pending, and the frozen
    /// levels' points too, so a frozen base and its record stay in one space and `level.unfreeze`
    /// still finds the base as freeze left it (review 0008, M1). The record's pending edits are
    /// carried up before it moves, as freeze carried them for the base: carried after a turn
    /// they round differently, and the moved base would no longer match the record bit for bit.
    pub(super) fn map_positions(&mut self, f: impl Fn(gs_core::Vec3) -> gs_core::Vec3 + Sync) {
        *self.view.get_mut().unwrap_or_else(PoisonError::into_inner) = Weak::new();
        match &mut self.body {
            Body::Levels(levels) => levels.map_positions(&f),
            Body::Plain { store, .. } => {
                let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
                store.write(&all, ColumnSet::POSITION).each(|node| {
                    for p in node.positions() {
                        *p = f(*p);
                    }
                });
            }
        }
        if let Some(record) = &mut self.frozen {
            // A record's levels are its own, never topology-edited: setting a level cannot fail.
            let active = record.active();
            record.set(record.top_level()).ok();
            record.map_positions(&f);
            record.set(active).ok();
        }
    }

    /// The part's detail levels, if it has more than one.
    pub(crate) fn levels(&self) -> Option<&Levels> {
        match &self.body {
            Body::Levels(levels) => Some(levels),
            Body::Plain { .. } => None,
        }
    }

    /// The part as a level stack: its own, or one level made from its store and numbering.
    pub(crate) fn to_levels(&self, settings: &gs_core::Settings) -> Result<Levels, MeshError> {
        match &self.body {
            Body::Levels(levels) => Ok(levels.share()),
            Body::Plain { store, order } => {
                Levels::new(store.share(), order.numbering(store), settings)
            }
        }
    }

    /// The nodes changed since the last call, for drawing. Keeps the last export: what the
    /// window takes away is not part of the mesh.
    pub(crate) fn take_dirty(&mut self) -> Vec<(NodeId, gs_core::ColumnSet)> {
        self.store_raw().take_dirty()
    }

    /// The active store: the plain store, or the active detail level's.
    pub(crate) fn store(&self) -> &MeshStore {
        match &self.body {
            Body::Plain { store, .. } => store,
            Body::Levels(levels) => levels.store(),
        }
    }

    fn store_raw(&mut self) -> &mut MeshStore {
        match &mut self.body {
            Body::Plain { store, .. } => store,
            Body::Levels(levels) => levels.store_mut(),
        }
    }

    /// The active store alone, for handing a job's edit back (`MeshStore::adopt`).
    pub(crate) fn into_store(self) -> MeshStore {
        match self.body {
            Body::Plain { store, .. } => store,
            Body::Levels(levels) => levels.into_single().0,
        }
    }

    /// Swaps the part's numbering with `order` and forgets the last export. A part with detail
    /// levels keeps its own: no edit changes a level's topology in place.
    pub(crate) fn swap_order(&mut self, order: &mut Order) {
        if let Body::Plain { order: own, .. } = &mut self.body {
            std::mem::swap(own, order);
            *self.view.get_mut().unwrap_or_else(PoisonError::into_inner) = Weak::new();
        }
    }

    /// The active store, writable. Forgets the last export.
    pub(crate) fn store_mut(&mut self) -> &mut MeshStore {
        *self.view.get_mut().unwrap_or_else(PoisonError::into_inner) = Weak::new();
        self.store_raw()
    }

    /// Every allocation the part holds besides its active store's columns and steps: the
    /// inactive levels, for the history's memory count.
    pub(crate) fn other_allocations(&self, mut f: impl FnMut(gs_core::Allocation)) {
        if let Body::Levels(levels) = &self.body {
            levels.other_allocations(&mut f);
        }
        if let Some(record) = &self.frozen {
            record.store().column_allocations(&mut f);
            record.other_allocations(f);
        }
    }

    /// The level stack `level.freeze` set aside, if the part is frozen (decision 0030).
    pub(crate) fn frozen(&self) -> Option<&Levels> {
        self.frozen.as_deref()
    }

    /// The part holding `record` as its frozen levels, or none.
    pub(crate) fn with_frozen(mut self, record: Option<Levels>) -> Part {
        self.frozen = record.map(Box::new);
        self
    }

    /// The numbering of the active store.
    pub(crate) fn order(&self) -> Order {
        match &self.body {
            Body::Plain { order, .. } => order.clone(),
            Body::Levels(levels) => order::Order::Given(Arc::clone(levels.numbering())),
        }
    }

    /// The part as plain mesh data. The last export is shared while the part is unchanged and
    /// something (the window, a checkpoint) still holds it; otherwise this exports again.
    pub(crate) fn view(&self) -> Arc<PlainMesh> {
        let mut last = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(view) = last.upgrade() {
            return view;
        }
        let view = Arc::new(order::export(self.store(), &self.order()));
        *last = Arc::downgrade(&view);
        view
    }

    pub(crate) fn numbering(&self) -> Numbering {
        Numbering::of(self.store(), &self.order())
    }

    /// Where the part's points are in the scene (decision 0029).
    pub(crate) fn transform(&self) -> Similarity {
        self.props.transform
    }

    /// The part as plain mesh data in the scene, as `part.get` shows it: the export with every
    /// point mapped by the transform, or the export itself at the identity.
    pub(crate) fn world(&self) -> Arc<PlainMesh> {
        let (view, t) = (self.view(), self.props.transform);
        if t.is_identity() {
            return view;
        }
        let mut mesh = PlainMesh::clone(&view);
        gs_par::apply(&mut mesh.positions, |_, p| {
            *p = t.point(Vec3::from_array(*p)).to_array();
        });
        Arc::new(mesh)
    }

    /// The bounding-box diagonal of the part's points in the scene, which percentage lengths
    /// are taken of (see [`scene_diagonal`]).
    pub(crate) fn world_diagonal(&self) -> f32 {
        scene_diagonal(self.store(), self.props.transform)
    }

    /// A length argument for the part, a percentage of its scene diagonal (see
    /// [`crate::Ctx::length`]).
    pub(crate) fn length(&self, length: Length) -> PartLength {
        PartLength::new(length, self.transform(), || self.world_diagonal())
    }

    /// The first surface point along the scene ray from `origin` along unit `dir`: the ray
    /// mapped into the part, the hit's scene distance and the hit itself in part space.
    pub(crate) fn hit(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Hit)> {
        let t = self.transform();
        let (o, d) = t.part_ray(origin, dir);
        let hit = self.store().ray_cast(o, d).hit?;
        Some((t.length(hit.t), hit))
    }

    /// Every vertex as touched, at its scene point, boxed in the part's numbering order (as an
    /// export lists them): for a part added, removed or replaced whole, counted without
    /// exporting it, in parallel.
    pub(crate) fn touched_whole(&self) -> Touched {
        let t = self.props.transform;
        self.touched_by(|p| Some(t.point(p).to_array()))
    }

    /// The vertices whose scene point differs from what transform `before` made of it, at their
    /// scene points now: what a change of the transform alone touched.
    pub(crate) fn touched_moved(&self, before: &Similarity) -> Touched {
        let t = self.props.transform;
        if t == *before {
            return Touched::default();
        }
        self.touched_by(|p| {
            let (was, now) = (before.point(p), t.point(p));
            (was.bits() != now.bits()).then(|| now.to_array())
        })
    }

    /// The live vertices `at` keeps, in numbering order, boxed in parallel.
    fn touched_by(&self, at: impl Fn(Vec3) -> Option<[f32; 3]> + Sync) -> Touched {
        let store = self.store();
        let p = store.pos();
        match self.order() {
            order::Order::Given(n) => {
                let ids = n.ids();
                Touched::in_runs(ids.len(), |i| at(p.get(ids[i])))
            }
            order::Order::Spatial => Touched::joined(gs_par::map(store.node_count(), |k| {
                let live = store.node(k as NodeId).pos.iter().filter(|q| !q.is_dead());
                Touched::of(live.filter_map(|&q| at(q)))
            })),
        }
    }

    /// For a part with detail levels: the level count, the active level (from 0) and the top
    /// level as plain mesh data in its numbering.
    pub(crate) fn top_view(&self) -> Option<(usize, usize, PlainMesh)> {
        let l = self.levels()?;
        let order = order::Order::Given(Arc::clone(l.level_numbering(l.top_level())));
        Some((l.count(), l.active(), order::export(l.top(), &order)))
    }

    /// For a frozen part: the frozen levels' count, active level (from 0), active level and top
    /// level as plain mesh data, each in its numbering.
    pub(crate) fn frozen_view(&self) -> Option<(usize, usize, PlainMesh, PlainMesh)> {
        let l = self.frozen()?;
        let active = order::Order::Given(Arc::clone(l.numbering()));
        let top = order::Order::Given(Arc::clone(l.level_numbering(l.top_level())));
        let held = order::export(l.store(), &active);
        Some((l.count(), l.active(), held, order::export(l.top(), &top)))
    }
}

/// A mesh the store cannot hold, as a command error.
fn refused(e: MeshError) -> CommandError {
    match e {
        MeshError::Input(message) => errors::args_invalid("/mesh", message),
        other => error(ErrorCode::Limit, other.to_string()),
    }
}

/// The bounding-box diagonal of `store`'s points placed in the scene by `t`. Without a turn,
/// the store's node bounds mapped: rounding keeps order, so that is the box of the points
/// mapped, with no point visited. With one, every point mapped and boxed, in parallel.
pub(crate) fn scene_diagonal(store: &MeshStore, t: Similarity) -> f32 {
    let b = if t.turns() {
        let boxes = gs_par::map(store.node_count(), |k| {
            let live = store.node(k as NodeId).pos.iter().filter(|q| !q.is_dead());
            live.fold(Aabb::EMPTY, |mut b, &q| {
                b.grow(t.point(q));
                b
            })
        });
        boxes.into_iter().fold(Aabb::EMPTY, Aabb::union)
    } else {
        let nodes = (0..store.node_count() as NodeId).map(|n| store.bounds(n));
        t.bounds(nodes.fold(Aabb::EMPTY, Aabb::union))
    };
    if b == Aabb::EMPTY {
        return 0.0;
    }
    crate::mesh::diagonal([b.lo.to_array(), b.hi.to_array()])
}
