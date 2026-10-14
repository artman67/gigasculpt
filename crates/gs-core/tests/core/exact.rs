//! A store's exact form (decision 0016): rebuilding a store from the columns it holds gives the
//! same store, rings and normals included, after strokes, edge splits and node splits; the next
//! stroke gives the same bits on both; and a form a store could not hold is refused.

use gs_core::{
    AttrDef, AttrId, AttrKind, AttrVec, Domain, ExactNode, MeshStore, NO, NodeConfig, Vec3,
};

use crate::common::{fingerprint, ok, refine, snap, sphere, store, stroke};

/// The exact form of `s`, read through its public views.
fn exact(s: &MeshStore) -> Vec<ExactNode> {
    (0..s.node_count() as u32)
        .map(|n| {
            let view = s.node(n);
            let (vfree, ffree) = s.free_slots(n);
            ExactNode {
                pos: view.pos.to_vec(),
                faces: view.faces.to_vec(),
                vfree: vfree.to_vec(),
                ffree: ffree.to_vec(),
                attrs: (0..s.attrs().len())
                    .map(|a| view.attr(attr_id(s, a)).cloned())
                    .collect(),
            }
        })
        .collect()
}

fn attr_id(s: &MeshStore, a: usize) -> AttrId {
    ok(s.attr_id(&s.attrs()[a].name).ok_or("no such attribute"))
}

/// `s` rebuilt from its exact form, as a scene file reopens a store.
pub(crate) fn rebuilt(s: &MeshStore) -> MeshStore {
    ok(MeshStore::from_exact(
        s.config(),
        s.attrs().to_vec(),
        exact(s),
    ))
}

fn normal_bits(s: &MeshStore) -> Vec<Vec<[u32; 3]>> {
    (0..s.node_count() as u32)
        .map(|n| s.node(n).nrm.iter().map(|p| p.bits()).collect())
        .collect()
}

/// A sphere with every attribute kind, stroked, refined (dead slots, free lists, node splits),
/// and a column that only a few nodes hold.
fn worked() -> MeshStore {
    let mut s = store(&sphere(12, 0.5, 7, true), ok(NodeConfig::new(6, 40)));
    stroke(&mut s, 3, 6, 0.4);
    let splits = refine(&mut s, Vec3::new(0.0, 0.0, 1.0), 0.5, 0.05, 2);
    assert!(splits > 0, "the refinement cut nodes in two");
    let weight = ok(s.add_attr(AttrDef::new("weight", Domain::Vertex, AttrKind::Scalar)));
    let mut w = s.write(&[0, 2], gs_core::ColumnSet::attr(weight));
    w.each(|n| {
        if let AttrVec::F32(v) = n.attr(weight) {
            v.fill(0.25);
        }
    });
    drop(w);
    let (vfree, _) = (0..s.node_count() as u32)
        .map(|n| s.free_slots(n))
        .fold((0, 0), |a, (v, f)| (a.0 + v.len(), a.1 + f.len()));
    assert!(vfree > 0, "the refinement left free vertex slots");
    s
}

#[test]
fn a_store_rebuilt_from_its_exact_form_is_the_same_store() {
    let mut original = worked();
    let mut copy = rebuilt(&original);
    assert!(
        copy.same_content(&original),
        "same columns, rings and free lists"
    );
    assert_eq!(normal_bits(&copy), normal_bits(&original), "same normals");
    let holders = |s: &MeshStore| {
        let weight = attr_id(s, s.attrs().len() - 1);
        (0..s.node_count() as u32)
            .filter(|&n| s.node(n).attr(weight).is_some())
            .count()
    };
    assert_eq!(holders(&copy), 2, "unallocated columns stay unallocated");
    stroke(&mut original, 9, 5, 0.5);
    stroke(&mut copy, 9, 5, 0.5);
    assert_eq!(
        fingerprint(&snap(&copy)),
        fingerprint(&snap(&original)),
        "the next stroke gives the same bits"
    );
    assert!(copy.same_content(&original), "and leaves the same store");
}

/// Breaks a form, given a node and a live face slot in it.
type Damage = fn(&mut Vec<ExactNode>, (usize, usize));

#[test]
fn an_exact_form_a_store_cannot_hold_is_refused() {
    let s = worked();
    let (cfg, defs) = (s.config(), s.attrs().to_vec());
    let live_face = |nodes: &[ExactNode]| {
        let k = (0..nodes.len()).find(|&k| nodes[k].faces.iter().any(|f| f[0] != NO));
        let k = ok(k.ok_or("no live face"));
        (
            k,
            ok(nodes[k].faces.iter().position(|f| f[0] != NO).ok_or("none")),
        )
    };
    let broken: [(&str, Damage); 5] = [
        ("a face naming a node that does not exist", |n, (k, l)| {
            n[k].faces[l][1] = u32::MAX - 1;
        }),
        ("a face repeating a vertex", |n, (k, l)| {
            n[k].faces[l][1] = n[k].faces[l][0];
        }),
        ("a free list naming a live slot", |n, (k, _)| {
            let live = ok(n[k].pos.iter().position(|p| !p.is_dead()).ok_or("none"));
            n[k].vfree.push(live as u32);
        }),
        ("a scalar value that is not a number", |n, _| {
            let k = ok(n
                .iter()
                .position(|x| x.attrs.last().is_some_and(Option::is_some))
                .ok_or("none"));
            if let Some(Some(AttrVec::F32(v))) = n[k].attrs.last_mut() {
                v[0] = f32::NAN;
            }
        }),
        ("an attribute column of the wrong length", |n, (k, _)| {
            if let Some(Some(AttrVec::U32(v))) = n[k].attrs.first_mut() {
                v.pop();
            }
        }),
    ];
    for (what, damage) in broken {
        let mut nodes = exact(&s);
        let at = live_face(&nodes);
        damage(&mut nodes, at);
        let refused = MeshStore::from_exact(cfg, defs.clone(), nodes);
        assert!(refused.is_err(), "refused: {what}");
    }
}
