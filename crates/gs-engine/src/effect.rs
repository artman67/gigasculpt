//! What an edit changed, for the `effect` every edit reports.
//!
//! A vertex counts as touched when its position or one of its vertex attribute values changed,
//! or when it is a corner of a face whose face attribute values changed. A part whose faces
//! changed, or that was added or removed, counts every vertex. An attribute column that was
//! added, removed or changed shape counts every vertex of its part. Touched vertices are listed
//! part by part in name order, each part's in its own vertex numbering, and the bounds are
//! folded in that order.
//!
//! A part edited in place is compared node by node with its open step's start, in the nodes
//! the edit wrote (see `document::changed`). A part that was added, removed or replaced is
//! compared as plain mesh data, before and after, by [`swapped`]; a part with detail levels
//! whose active level was replaced by one cut and numbered alike is compared node by node by
//! [`swapped_alike`], with the same result and without exporting either.
//!
//! The same comparisons name what changed ([`Names`]), so no command declares it: the part's
//! `positions` if a point moved in the part's own space, `faces` if they changed, each
//! attribute column whose values changed or that was added or removed, by the name `part.get`
//! gives it, `levels` if its detail levels did (see [`levels`]), and the properties that
//! changed ([`props`]). A part added or removed, or whose faces changed, names everything it
//! holds ([`whole`]).

use std::collections::{BTreeMap, BTreeSet};

use gs_core::{
    AttrDef, AttrId, AttrValue, AttrVec, Domain, Levels, MeshStore, NodeId, NodeView, Numbering,
    PosRead,
};
use gs_protocol::{AttrDomain, AttrValues, Attribute, Effect, PlainMesh, Vec3};

use crate::document::{Part, PartChange, Props};
use crate::mesh::{bounds_of, corners, per_axis};

/// What changed in one part, in the names agents read (see `Effect::changed`).
pub(crate) type Names = BTreeSet<String>;

/// The name of a part's points, in its own space.
pub(crate) const POSITIONS: &str = "positions";
/// The name of a part's faces: which vertices each one joins.
const FACES: &str = "faces";
/// The name of a part's detail levels, frozen or not.
const LEVELS: &str = "levels";

/// Everything `part` holds, for a part added, removed or whose faces changed: its positions,
/// faces, every column it exports, and its levels if it has any.
pub(crate) fn whole(part: &Part) -> Names {
    let leveled = part.levels().is_some() || part.frozen().is_some();
    let columns = exported(part.store()).into_keys();
    let mut names: Names = columns.map(str::to_string).collect();
    names.extend([POSITIONS, FACES].map(str::to_string));
    if leveled {
        names.insert(LEVELS.into());
    }
    names
}

/// The properties that differ between `was` and `now`, as `part.list` names them: its
/// `transform`, its `rank` in the list, its eye (`visible`) and its Live Boolean `operation` and
/// `start`; its `voxel_mode`, as `mesh.voxel_mode` sets it; and `paint_visible`, as
/// `part.set_paint_visible` sets it. Its id, kept cell size and export transform are bookkeeping
/// no agent sets, so they are not named.
pub(crate) fn props(was: &Props, now: &Props) -> Names {
    let differ = [
        ("transform", was.transform != now.transform),
        ("rank", was.rank != now.rank),
        ("visible", was.hidden != now.hidden),
        ("paint_visible", was.paint_hidden != now.paint_hidden),
        ("voxel_mode", was.voxel_mode != now.voxel_mode),
        ("operation", was.operation != now.operation),
        ("start", was.start != now.start),
    ];
    (differ.into_iter())
        .filter(|(_, differs)| *differs)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// `levels` if the detail levels of `old` and `new` differ besides the active level's columns,
/// which the other names cover: a level added, removed or made active, or the frozen levels.
/// Compares shares, so it reads no mesh.
pub(crate) fn levels(old: &Part, new: &Part) -> Names {
    let same =
        |a: Option<&Levels>, b: Option<&Levels>, f: fn(&Levels, &Levels) -> bool| match (a, b) {
            (Some(a), Some(b)) => f(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
    let kept = same(old.levels(), new.levels(), Levels::same_besides_active)
        && same(old.frozen(), new.frozen(), Levels::same);
    (!kept).then(|| LEVELS.to_string()).into_iter().collect()
}

/// What a change touched: how many vertices, and the box around them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Touched {
    pub count: u64,
    pub bounds: Option<[[f32; 3]; 2]>,
}

/// Points boxed per parallel run by [`Touched::in_runs`].
const RUN: usize = 1 << 16;

impl Touched {
    /// The vertices at `points`, boxed in the order given (as `bounds_of` folds them), without
    /// collecting them.
    pub fn of(points: impl IntoIterator<Item = [f32; 3]>) -> Self {
        let mut count = 0;
        let bounds = bounds_of(points.into_iter().inspect(|_| count += 1));
        Self { count, bounds }
    }

    /// `parts` one after another: their counts added and their boxes joined in order.
    pub fn joined(parts: impl IntoIterator<Item = Touched>) -> Self {
        parts.into_iter().fold(Self::default(), |a, b| Self {
            count: a.count + b.count,
            bounds: join(a.bounds, b.bounds),
        })
    }

    /// The vertices `point(i)` gives for `i` in `0..len` in order (`None` for one not
    /// touched), boxed in parallel: fixed runs of that order are boxed apart and joined in
    /// order. Min and max with a fixed rule for ties group freely, so the box has the bits of
    /// one fold in order, on any thread count.
    pub fn in_runs(len: usize, point: impl Fn(usize) -> Option<[f32; 3]> + Sync) -> Self {
        let runs = gs_par::map(len.div_ceil(RUN), |k| {
            Self::of((k * RUN..len.min((k + 1) * RUN)).filter_map(&point))
        });
        Self::joined(runs)
    }
}

/// Two boxes joined, either one missing.
fn join(a: Option<[[f32; 3]; 2]>, b: Option<[[f32; 3]; 2]>) -> Option<[[f32; 3]; 2]> {
    match (a, b) {
        (Some([lo, hi]), Some([l, h])) => {
            Some([per_axis(lo, l, f32::min), per_axis(hi, h, f32::max)])
        }
        (x, y) => x.or(y),
    }
}

/// The effect of a call's changes. No changes give a `noop_reason`: `reason` if the command
/// gave one, a general sentence otherwise. A change that touched no vertex and no face (one
/// that changed only a part's detail levels around its active level, which `part.get` does not
/// show) is not listed, but is a change.
pub(crate) fn effect_of(changes: &[PartChange], reason: Option<String>) -> Effect {
    let bbox = changes.iter().fold(None, |b, c| join(b, c.touched.bounds));
    let nothing = || "nothing changed: the command left every part exactly as it was".to_string();
    Effect {
        parts: (changes.iter())
            .filter(|c| c.touched.count > 0 || c.topology_changed)
            .map(|c| c.change.part().to_string())
            .collect(),
        verts_touched: changes.iter().map(|c| c.touched.count).sum(),
        bbox: bbox.map(|[min, max]| [Vec3(min), Vec3(max)]),
        topology_changed: changes.iter().any(|c| c.topology_changed),
        changed: (changes.iter())
            .filter(|c| !c.changed.is_empty())
            .map(|c| (c.change.part().into(), c.changed.iter().cloned().collect()))
            .collect(),
        noop_reason: changes.is_empty().then(|| reason.unwrap_or_else(nothing)),
    }
}

/// What replacing part `old` with `new` touched, from their exports in the scene, `was` and
/// `now`: the touched vertices, whether the topology changed, and what changed by name, read in
/// the parts' own space (the same comparison when both are at the identity, where the scene is
/// their space; otherwise their own exports are compared, since a bake moves no scene point).
pub(crate) fn swapped(
    old: &Part,
    new: &Part,
    was: &PlainMesh,
    now: &PlainMesh,
) -> (Touched, bool, Names) {
    if !same_topology(was, now) {
        let mut names = whole(old);
        names.extend(whole(new));
        return (Touched::of(now.positions.iter().copied()), true, names);
    }
    let (marked, mut names) = touched_vertices(was, now);
    if !(old.transform().is_identity() && new.transform().is_identity()) {
        names = touched_vertices(&old.view(), &new.view()).1;
    }
    let points = now.positions.iter().zip(marked);
    let touched = Touched::of(points.filter(|(_, hit)| *hit).map(|(p, _)| *p));
    (touched, false, names)
}

/// What [`swapped`] finds between the exports of stores `old` and `new`, read node by node
/// without exporting either: the touched vertices, in the stores' own space (so the caller
/// uses it only for parts at the identity transform), boxed in `numbering`'s order, no
/// topology change, and what changed by name. Only for stores cut the same way with no dead
/// slot (the same node size and each node's slots and faces) and both numbered by
/// `numbering`, so their exports have the same faces. A column only one exports was added or
/// removed, so every vertex is touched; the columns both export are compared value by value,
/// skipping those both hold in the same allocation. `None` when the stores are cut
/// differently, a column changed its kind, or a mirror map's values changed (they are
/// renumbered on export): the caller exports both.
pub(crate) fn swapped_alike(
    old: &MeshStore,
    new: &MeshStore,
    numbering: &Numbering,
) -> Option<(Touched, Names)> {
    let nodes = new.node_count();
    if old.config() != new.config()
        || old.node_count() != nodes
        || numbering.vertex_count() != new.vertex_count()
        || !dense(old)
        || !dense(new)
    {
        return None;
    }
    let (was, now) = (exported(old), exported(new));
    if (was.iter()).any(|(name, (_, d))| now.get(name).is_some_and(|(_, e)| e != d)) {
        return None;
    }
    let columns: Vec<Column> = (was.iter())
        .filter_map(|(name, &(a, d))| now.get(name).map(|&(b, _)| (a, b, d)))
        .collect();
    let one_side = |a: &BTreeMap<&str, _>, b: &BTreeMap<&str, _>| {
        (a.keys().filter(|k| !b.contains_key(*k)))
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let mut names: Names = one_side(&was, &now).into_iter().collect();
    names.extend(one_side(&now, &was));
    let reshaped = !names.is_empty();
    let hits = gs_par::map(nodes, |n| {
        let n = n as NodeId;
        node_changes(old.node(n), new.node(n), &columns, numbering)
    });
    let mut marked = vec![false; numbering.vertex_count()];
    for hits in hits {
        let (vertices, named) = hits?;
        names.extend(named.into_iter().map(str::to_string));
        for v in vertices {
            marked[v as usize] = true;
        }
    }
    let p = new.pos();
    let point = |c: usize| p.get(numbering.id(c as u32)).to_array();
    if reshaped {
        let every = Touched::in_runs(numbering.vertex_count(), |c| Some(point(c)));
        return Some((every, names));
    }
    let points = (marked.iter().enumerate()).filter(|(_, hit)| **hit);
    Some((Touched::of(points.map(|(c, _)| point(c))), names))
}

/// A column both stores export: its id in the old store, in the new, and its definition.
type Column<'a> = (AttrId, AttrId, &'a AttrDef);

/// True if every vertex and face slot of `store` is live.
fn dense(store: &MeshStore) -> bool {
    let nodes = (0..store.node_count()).map(|n| store.node(n as NodeId));
    let (verts, faces) = nodes.fold((0, 0), |(v, f), n| (v + n.pos.len(), f + n.faces.len()));
    verts == store.vertex_count() && faces == store.face_count()
}

/// The columns an export of `store` lists, by name: those some node holds.
fn exported(store: &MeshStore) -> BTreeMap<&str, (AttrId, &AttrDef)> {
    (store.attrs().iter())
        .filter_map(|d| {
            let a = store.attr_id(&d.name)?;
            let held = (0..store.node_count()).any(|n| store.node(n as NodeId).attr(a).is_some());
            held.then_some((d.name.as_str(), (a, d)))
        })
        .collect()
}

/// The vertices whose position or values in `columns` differ between two versions of a dense
/// node, by their index in `numbering` (a face's values mark its corners), and the names of
/// what differs (`positions`, columns), or `None` if the node's slots or faces differ, or a
/// mirror map's values do.
fn node_changes<'a>(
    old: NodeView,
    new: NodeView,
    columns: &[Column<'a>],
    numbering: &Numbering,
) -> Option<(Vec<u32>, BTreeSet<&'a str>)> {
    let same_faces = std::ptr::eq(old.faces, new.faces) || old.faces == new.faces;
    if old.pos.len() != new.pos.len() || !same_faces {
        return None;
    }
    let (mut hits, mut names) = (Vec::new(), BTreeSet::new());
    if !std::ptr::eq(old.pos, new.pos) {
        for (l, (a, b)) in old.pos.iter().zip(new.pos).enumerate() {
            if a.to_array().map(f32::to_bits) != b.to_array().map(f32::to_bits) {
                hits.push(numbering.index(new.id, l as u32));
            }
        }
    }
    if !hits.is_empty() {
        names.insert(POSITIONS);
    }
    for &(a, b, def) in columns {
        let (was, now) = (old.attr(a), new.attr(b));
        match (was, now) {
            (None, None) => continue,
            (Some(x), Some(y)) if std::ptr::eq(x, y) => continue,
            _ => {}
        }
        // A node that never wrote a column holds its kind's empty value (white for a color).
        let empty = AttrValue::zero(def.kind).bits();
        let bits = |c: Option<&AttrVec>, i| c.map_or(empty, |c| c.bits(i));
        let slots = match def.domain {
            Domain::Vertex => new.pos.len(),
            Domain::Face => new.faces.len(),
        };
        for i in (0..slots).filter(|&i| bits(was, i) != bits(now, i)) {
            if gs_core::mirror::is_map(&def.name) {
                return None;
            }
            names.insert(def.name.as_str());
            match def.domain {
                Domain::Vertex => hits.push(numbering.index(new.id, i as u32)),
                Domain::Face => {
                    let face = &new.faces[i];
                    hits.extend(corners(face).iter().map(|&v| numbering.canonical(v)));
                }
            }
        }
    }
    Some((hits, names))
}

fn same_topology(before: &PlainMesh, after: &PlainMesh) -> bool {
    before.positions.len() == after.positions.len() && before.faces == after.faces
}

/// Which vertices of a part with unchanged faces were touched, and what changed by name. A
/// column added, removed or changed in shape touches every vertex.
fn touched_vertices(before: &PlainMesh, after: &PlainMesh) -> (Vec<bool>, Names) {
    let mut marked: Vec<bool> = after
        .positions
        .iter()
        .zip(&before.positions)
        .map(|(new, old)| new.map(f32::to_bits) != old.map(f32::to_bits))
        .collect();
    let mut changed = Names::new();
    if marked.contains(&true) {
        changed.insert(POSITIONS.into());
    }
    let names: BTreeSet<&String> = before
        .attributes
        .keys()
        .chain(after.attributes.keys())
        .collect();
    let mut every = false;
    for name in names {
        let pair = (before.attributes.get(name), after.attributes.get(name));
        let Some(differs) = value_changes(pair) else {
            every = true;
            changed.insert(name.clone());
            continue;
        };
        if !differs.is_empty() {
            changed.insert(name.clone());
        }
        for index in differs {
            let vertices: &[u32] = if pair.1.is_some_and(|a| a.domain == AttrDomain::Vertex) {
                &[index as u32]
            } else {
                after.faces.get(index).map(corners).unwrap_or_default()
            };
            for &v in vertices {
                mark(&mut marked, v);
            }
        }
    }
    if every {
        marked.fill(true);
    }
    (marked, changed)
}

fn mark(marked: &mut [bool], vertex: u32) {
    if let Some(slot) = marked.get_mut(vertex as usize) {
        *slot = true;
    }
}

/// The indices whose values differ between two versions of a column, or `None` if the column
/// was added, removed or changed domain, type or length.
fn value_changes(pair: (Option<&Attribute>, Option<&Attribute>)) -> Option<Vec<usize>> {
    let (Some(old), Some(new)) = pair else {
        return None;
    };
    let bits: fn(&AttrValues) -> (u8, Vec<u32>) = |values| match values {
        AttrValues::F32(v) => (0, v.iter().map(|x| x.to_bits()).collect()),
        AttrValues::U32(v) => (1, v.clone()),
    };
    let ((old_type, old_bits), (new_type, new_bits)) = (bits(&old.values), bits(&new.values));
    if old.domain != new.domain || old_type != new_type || old_bits.len() != new_bits.len() {
        return None;
    }
    let pairs = old_bits.iter().zip(&new_bits).enumerate();
    Some(pairs.filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect())
}
