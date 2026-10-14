//! Oracle self-tests (ARCHITECTURE 10.1, kind 4): the checker, hasher and canonical form each
//! reject known-bad data, so a passing test means something.

use gs_core::{Aabb, NodeConfig, Vec3};

use crate::common::{
    Snap, brute_query, canonical, check, fingerprint, ok, snap, some, sphere, store,
};

fn good() -> Snap {
    snap(&store(
        &sphere(8, 0.5, 51, true),
        ok(NodeConfig::new(6, 40)),
    ))
}

/// A node with seam vertices, and one of them.
fn seam_node(s: &Snap) -> (usize, usize) {
    let k = some((0..s.nodes.len()).find(|&k| !s.nodes[k].seam.is_empty()));
    (k, s.nodes[k].seam[0] as usize)
}

#[test]
fn the_checker_rejects_each_kind_of_broken_store() {
    let s = good();
    check(&s).unwrap();
    let (k, l) = seam_node(&s);
    let mut bad: Vec<(&str, Snap)> = Vec::new();
    let mut b = s.clone();
    b.nodes[k].nrm[l].x = f32::from_bits(b.nodes[k].nrm[l].x.to_bits() ^ 1);
    bad.push(("a normal one bit off", b));
    let mut b = s.clone();
    let i = (0..b.nodes[0].pos.len())
        .find(|i| !b.nodes[0].seam.contains(&(*i as u32)))
        .unwrap();
    b.nodes[0].nrm[i] = Vec3::ZERO;
    bad.push(("a stale interior normal", b));
    let mut b = s.clone();
    b.nodes[k].seam.remove(0);
    bad.push(("a missing seam vertex", b));
    let mut b = s.clone();
    b.nodes[k].shared.pop();
    bad.push(("a missing shared vertex", b));
    let mut b = s.clone();
    b.nodes[k].bounds.hi.x -= 1e-3;
    bad.push(("bounds too small", b));
    let mut b = s.clone();
    b.nodes[k].halo = b.nodes[k].bounds;
    bad.push(("halo without its seam neighbours", b));
    let mut b = s.clone();
    b.nodes[k].rings[l].reverse();
    bad.push(("a ring out of order", b));
    let mut b = s.clone();
    b.nodes[k].rings[l].pop();
    bad.push(("a ring missing a face", b));
    let mut b = s.clone();
    let p = b.nodes[k].pos[l];
    b.nodes[k].pos[l] = Vec3::new(p.x + 1e-4, p.y, p.z);
    bad.push(("a moved vertex with stale derived data", b));
    let mut b = s.clone();
    b.nodes[k].pos[l] = Vec3::DEAD;
    bad.push(("a face on a dead vertex", b));
    let mut b = s.clone();
    b.nodes[k].nrm.pop();
    bad.push(("columns of different lengths", b));
    let mut b = s.clone();
    b.nodes[k].base += 1;
    bad.push(("a node owning the wrong ids", b));
    let mut b = s.clone();
    match b.nodes[k].attrs.iter_mut().flatten().next() {
        Some(gs_core::AttrVec::F32(v)) => drop(v.pop()),
        Some(gs_core::AttrVec::U32(v)) => drop(v.pop()),
        Some(gs_core::AttrVec::Bits(b)) => b.resize(b.len() - 1),
        None => panic!("the test mesh has attributes"),
    }
    bad.push(("a short attribute column", b));
    for (name, b) in bad {
        assert!(check(&b).is_err(), "{name}");
    }
}

type Tweak = fn(&mut Snap, usize, usize);

#[test]
fn the_fingerprint_sees_one_bit_anywhere() {
    let s = good();
    let h = fingerprint(&s);
    let (k, l) = seam_node(&s);
    let tweaks: Vec<(&str, Tweak)> = vec![
        ("position", |s, k, l| {
            s.nodes[k].pos[l].z = f32::from_bits(s.nodes[k].pos[l].z.to_bits() ^ 1);
        }),
        ("normal", |s, k, l| {
            s.nodes[k].nrm[l].y = -s.nodes[k].nrm[l].y;
        }),
        ("face", |s, k, _| s.nodes[k].faces[0].swap(1, 2)),
        ("ring", |s, k, l| s.nodes[k].rings[l].push(0)),
        ("seam", |s, k, _| s.nodes[k].seam.push(999)),
        ("shared", |s, k, _| s.nodes[k].shared.push(999)),
        ("bounds", |s, k, _| s.nodes[k].bounds = Aabb::EMPTY),
        ("halo", |s, k, _| s.nodes[k].halo.lo.x -= 1.0),
        ("attribute", |s, k, _| s.nodes[k].attrs[0] = None),
        ("node count", |s, _, _| {
            s.nodes.pop();
        }),
    ];
    for (name, tweak) in tweaks {
        let mut b = s.clone();
        tweak(&mut b, k, l);
        assert_ne!(fingerprint(&b), h, "{name}");
    }
}

#[test]
fn the_canonical_form_ignores_order_but_not_shape_or_attributes() {
    let m = sphere(5, 0.5, 52, true);
    let c = canonical(&m);
    let mut b = m.clone();
    b.faces.reverse();
    for a in b
        .attributes
        .values_mut()
        .filter(|a| a.domain == gs_protocol::AttrDomain::Face)
    {
        if let gs_protocol::AttrValues::U32(v) = &mut a.values {
            v.reverse();
        }
    }
    assert!(canonical(&b) == c, "face order does not matter");
    let mut b = m.clone();
    b.faces[3].swap(0, 1);
    assert!(canonical(&b) != c, "a flipped face");
    let mut b = m.clone();
    b.positions[4][0] += 1e-3;
    assert!(canonical(&b) != c, "a moved vertex");
    let mut b = m.clone();
    if let Some(gs_protocol::AttrValues::U32(v)) =
        b.attributes.get_mut("group").map(|a| &mut a.values)
    {
        v[0] += 1;
    }
    assert!(canonical(&b) != c, "a face attribute");
    let mut b = m.clone();
    if let Some(gs_protocol::AttrValues::F32(v)) =
        b.attributes.get_mut("mask").map(|a| &mut a.values)
    {
        v[0] += 1.0;
    }
    assert!(canonical(&b) != c, "a vertex attribute");
}

#[test]
fn the_brute_force_query_splits_brushed_from_shell() {
    let s = good();
    let k = 5;
    let n = &s.nodes[k];
    let c = (n.bounds.lo + n.bounds.hi) * 0.5;
    let (brushed, _) = brute_query(&s, c, 1e-6);
    assert!(
        brushed.contains(&(k as u32)),
        "a node whose bounds hold the point"
    );
    let far = Vec3::new(10.0, 10.0, 10.0);
    assert_eq!(
        brute_query(&s, far, 1.0),
        (vec![], vec![]),
        "nothing far away"
    );
    let all = brute_query(&s, Vec3::ZERO, 10.0).0;
    assert_eq!(all.len(), s.nodes.len(), "everything inside a big sphere");
}
