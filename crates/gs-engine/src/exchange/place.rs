//! The in-place write (decision 0032, section 2): a part that came back with its own points and
//! faces takes the new values node by node, so its levels, its active level and every column
//! Blender never saw stay, and a value that did not change is not written.

use gs_core::{AttrDef, AttrKind, AttrValue, AttrVec, ColumnSet, Domain, MeshStore, NO};
use gs_core::{NewValues, NodeId};
use gs_core::{Numbering, Vec3};
use gs_protocol::{AttrValues, PlainMesh};

/// Where each point and face of a part's plain view sits in its store: per node, the view index
/// of every vertex slot and face slot, `NO` for a dead one.
pub struct Slots {
    points: Vec<Vec<u32>>,
    faces: Vec<Vec<u32>>,
}

impl Slots {
    /// The slots of `store` under `numbering`, or under the store's own order (live slots node
    /// by node, as `MeshStore::to_plain` counts them) when it has none.
    pub fn of(store: &MeshStore, numbering: Option<&Numbering>) -> Slots {
        let nodes = 0..store.node_count() as NodeId;
        let Some(num) = numbering else {
            let (mut p, mut f) = (0, 0);
            let (mut points, mut faces) = (Vec::new(), Vec::new());
            for n in nodes {
                let node = store.node(n);
                points.push(count(node.pos.iter().map(|q| !q.is_dead()), &mut p));
                faces.push(count(node.faces.iter().map(|q| q[0] != NO), &mut f));
            }
            return Slots { points, faces };
        };
        let (canon, starts) = (num.face_canon(), num.face_starts());
        let slots = |n: NodeId| (0..store.node(n).pos.len() as u32).map(move |l| num.index(n, l));
        let face_slots = |n: NodeId| {
            let first = starts[n as usize] as usize;
            canon[first..first + store.node(n).faces.len()].to_vec()
        };
        Slots {
            points: nodes.clone().map(|n| slots(n).collect()).collect(),
            faces: nodes.map(face_slots).collect(),
        }
    }
}

/// Numbers the live slots of one node on from `next`.
fn count(live: impl Iterator<Item = bool>, next: &mut u32) -> Vec<u32> {
    live.map(|l| {
        if l {
            *next += 1;
            *next - 1
        } else {
            NO
        }
    })
    .collect()
}

/// Writes `mesh`, the part's own points and faces in its view's order, into `store`: the
/// positions that differ, and with `columns` the mask, face groups, hidden faces and colors, a
/// column `mesh` lacks reading as its empty value (zeros, and white for colors). Returns the
/// nodes written, ascending.
pub fn write(store: &mut MeshStore, slots: &Slots, mesh: &PlainMesh, columns: bool) -> Vec<NodeId> {
    let mut written = positions(store, slots, &mesh.positions);
    let defs = if columns {
        vec![
            AttrDef::mask(),
            AttrDef::face_group(),
            AttrDef::hide(),
            AttrDef::color(),
        ]
    } else {
        Vec::new()
    };
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    for def in defs {
        let at = match def.domain {
            Domain::Vertex => &slots.points,
            Domain::Face => &slots.faces,
        };
        let want = (mesh.attributes.get(&def.name)).map(|a| words(&a.values));
        let fresh = fresh(store, &def, at, want.as_deref());
        written.extend(store.put_column(def, &nodes, fresh));
    }
    written.sort_unstable();
    written.dedup();
    written
}

/// Moves the points whose position differs from `to` (compared bit for bit).
fn positions(store: &mut MeshStore, slots: &Slots, to: &[[f32; 3]]) -> Vec<NodeId> {
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    let moved = |n: NodeId| {
        let node = store.node(n);
        (node.pos.iter().zip(&slots.points[n as usize]))
            .any(|(p, &c)| c != NO && bits(p.to_array()) != bits(to[c as usize]))
    };
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId)
        .filter(|&n| moved(n))
        .collect();
    if !nodes.is_empty() {
        store.write(&nodes, ColumnSet::POSITION).each(|m| {
            let at = &slots.points[m.id() as usize];
            for (p, &c) in m.positions().iter_mut().zip(at) {
                if c != NO {
                    *p = Vec3::from_array(to[c as usize]);
                }
            }
        });
    }
    nodes
}

/// Each node's new values of column `def`: `want` at each live slot (the column's empty value
/// when `want` is `None`), the old value at a dead one. `Same` where nothing changes, `Zeros`
/// where every value is the empty one.
fn fresh(
    store: &MeshStore,
    def: &AttrDef,
    slots: &[Vec<u32>],
    want: Option<&[u32]>,
) -> Vec<NewValues> {
    let id = store.attr_id(&def.name);
    let empty = AttrValue::zero(def.kind).bits();
    gs_par::map(store.node_count(), |k| {
        let node = store.node(k as NodeId);
        let old = held(id.and_then(|a| node.attr(a)), slots[k].len(), empty);
        let new: Vec<u32> = (slots[k].iter().zip(&old))
            .map(|(&c, &o)| {
                if c == NO {
                    o
                } else {
                    want.map_or(empty, |w| w[c as usize])
                }
            })
            .collect();
        match (new == old, new.iter().any(|&w| w != empty)) {
            (true, _) => NewValues::Same,
            (false, true) => NewValues::Values(column(def.kind, new)),
            (false, false) => NewValues::Zeros,
        }
    })
}

/// A plain column's values as 32-bit words: a float's bits, an id, a flag's 0 or 1.
fn words(values: &AttrValues) -> Vec<u32> {
    match values {
        AttrValues::F32(v) => v.iter().map(|x| x.to_bits()).collect(),
        AttrValues::U32(v) => v.clone(),
    }
}

/// A node's values of a column as words, `empty` (the column's empty value) where it holds none.
fn held(values: Option<&AttrVec>, len: usize, empty: u32) -> Vec<u32> {
    match values {
        Some(AttrVec::F32(v)) => v.iter().map(|x| x.to_bits()).collect(),
        Some(AttrVec::U32(v)) => v.clone(),
        Some(AttrVec::Bits(b)) => (0..len).map(|i| u32::from(b.get(i))).collect(),
        None => vec![empty; len],
    }
}

/// Words back as a column of kind `kind`.
fn column(kind: AttrKind, words: Vec<u32>) -> AttrVec {
    match kind {
        AttrKind::Scalar => AttrVec::F32(words.into_iter().map(f32::from_bits).collect()),
        AttrKind::Flag => AttrVec::Bits(words.iter().map(|&w| w != 0).collect()),
        AttrKind::Label | AttrKind::Rgba8 => AttrVec::U32(words),
    }
}
