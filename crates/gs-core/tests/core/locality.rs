//! Locality: an edit leaves everything outside the nodes it names bit-identical, and the dirty
//! bits name exactly what was written.

use gs_core::{
    AttrVec, ColumnSet, Kernel, MeshStore, NodeConfig, NodeView, Scratch, SculptSurface, Vec3,
};

use crate::common::{NodeSnap, Relax, Snap, check, ok, snap, sphere, store, surface_point};

fn same_node(a: &NodeSnap, b: &NodeSnap) -> bool {
    let bits = |v: &[Vec3]| v.iter().map(|p| p.bits()).collect::<Vec<_>>();
    bits(&a.pos) == bits(&b.pos)
        && bits(&a.nrm) == bits(&b.nrm)
        && a.faces == b.faces
        && a.rings == b.rings
        && a.attrs == b.attrs
}

/// Nodes whose data differ between two snapshots.
fn changed(a: &Snap, b: &Snap) -> Vec<u32> {
    (0..a.nodes.len() as u32)
        .filter(|&n| !same_node(&a.nodes[n as usize], &b.nodes[n as usize]))
        .collect()
}

fn mesh() -> MeshStore {
    store(&sphere(12, 0.5, 21, true), ok(NodeConfig::new(6, 40)))
}

#[test]
fn a_kernel_step_touches_only_its_nodes_and_vertices_inside_the_brush() {
    let mut s = mesh();
    s.take_dirty();
    let before = snap(&s);
    let (c, r) = (surface_point(&s, Vec3::new(0.5, 0.5, -1.0)), 0.3);
    let rep = s.run_kernel(&Relax { c, r, s: 0.8 }, &mut Scratch::default());
    let after = snap(&s);
    check(&after).unwrap();
    let ch = changed(&before, &after);
    assert!(ch.len() > 3, "the dab changed several nodes");
    for n in &ch {
        assert!(
            rep.brushed.contains(n) || rep.shell.contains(n),
            "node {n} outside the step"
        );
    }
    for (a, b) in before.nodes.iter().zip(&after.nodes) {
        for (p, q) in a.pos.iter().zip(&b.pos) {
            if (*p - c).len2() >= r * r {
                assert_eq!(p.bits(), q.bits(), "a vertex outside the brush moved");
            }
        }
    }
    let dirty = s.take_dirty();
    let mut want: Vec<(u32, ColumnSet)> = (rep.brushed.iter())
        .map(|&n| (n, ColumnSet::POSITION | ColumnSet::NORMAL))
        .chain(rep.shell.iter().map(|&n| (n, ColumnSet::NORMAL)))
        .collect();
    want.sort_by_key(|x| x.0);
    assert_eq!(dirty, want, "dirty bits");
    assert!(s.take_dirty().is_empty(), "taking clears them");
}

/// Lift each vertex along its normal with a smooth falloff: a kernel that reads only its own
/// vertex, run in one pass per node when `one` says so.
struct Lift {
    c: Vec3,
    r: f32,
    one: bool,
    panics: bool,
}

impl Kernel for Lift {
    fn spheres(&self) -> Vec<(Vec3, f32)> {
        vec![(self.c, self.r)]
    }
    fn deltas(&self, _: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]) {
        assert!(!self.panics, "a kernel bug");
        for (l, o) in out.iter_mut().enumerate() {
            let d2 = (node.pos[l] - self.c).len2();
            if !node.pos[l].is_dead() && d2 < self.r * self.r {
                *o = node.nrm[l] * (0.05 * (1.0 - d2 / (self.r * self.r)));
            }
        }
    }
    fn own_only(&self) -> bool {
        self.one
    }
}

#[test]
fn an_own_vertex_kernel_gives_the_same_bits_in_one_pass_as_in_two() {
    let (mut a, mut b) = (mesh(), mesh());
    for i in 0..6u64 {
        let c = surface_point(&a, crate::common::dir(i % 3));
        let r = [0.08, 0.3, 0.7][(i % 3) as usize];
        let k = |one| Lift {
            c,
            r,
            one,
            panics: false,
        };
        let (ra, rb) = (
            a.run_kernel(&k(true), &mut Scratch::default()),
            b.run_kernel(&k(false), &mut Scratch::default()),
        );
        assert!(!ra.brushed.is_empty(), "dab {i} moved points");
        assert_eq!(ra, rb, "dab {i}: report");
        let (sa, sb) = (snap(&a), snap(&b));
        check(&sa).unwrap_or_else(|e| panic!("dab {i}: {e}"));
        assert!(
            changed(&sa, &sb).is_empty(),
            "dab {i}: positions or normals"
        );
    }
    assert_eq!(a.take_dirty(), b.take_dirty(), "dirty bits");
}

#[test]
fn a_panicking_one_pass_kernel_leaves_every_column_in_place() {
    let mut s = mesh();
    let before = snap(&s);
    let c = surface_point(&s, Vec3::new(0.0, 1.0, 0.0));
    let k = Lift {
        c,
        r: 0.5,
        one: true,
        panics: true,
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        s.run_kernel(&k, &mut Scratch::default())
    }));
    assert!(run.is_err(), "the kernel panicked");
    let after = snap(&s);
    check(&after).unwrap();
    assert!(changed(&before, &after).is_empty(), "nothing lost or moved");
}

#[test]
fn an_attribute_write_changes_only_the_declared_nodes_and_column() {
    let mut s = mesh();
    let mask = s.attr_id("mask").unwrap();
    s.take_dirty();
    let before = snap(&s);
    let nodes = [2, 3, 9];
    s.write(&nodes, ColumnSet::attr(mask)).each(|n| {
        let id = n.id() as f32;
        if let AttrVec::F32(v) = n.attr(mask) {
            v[0] = 7.0 + id;
        }
    });
    let after = snap(&s);
    assert_eq!(changed(&before, &after), nodes, "changed nodes");
    for n in nodes {
        let first = s.node(n).attr(mask).map(|a| crate::common::attr_bits(a)[0]);
        assert_eq!(
            first,
            Some((7.0 + n as f32).to_bits()),
            "node {n} wrote its own id"
        );
    }
    for &n in &nodes {
        let (a, b) = (&before.nodes[n as usize], &after.nodes[n as usize]);
        assert_eq!(a.pos, b.pos, "positions of {n}");
        assert!(a.nrm == b.nrm, "normals of {n}");
    }
    let dirty = s.take_dirty();
    assert_eq!(
        dirty,
        nodes.map(|n| (n, ColumnSet::attr(mask))),
        "dirty bits"
    );
}

#[test]
fn a_position_write_refreshes_normals_around_the_written_nodes() {
    let mut s = mesh();
    s.take_dirty();
    let before = snap(&s);
    s.write(&[4], ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            p.z += 0.01;
        }
    });
    let after = snap(&s);
    check(&after).unwrap();
    let (written, others): (Vec<_>, Vec<_>) = s.take_dirty().into_iter().partition(|d| d.0 == 4);
    assert_eq!(
        written,
        [(4, ColumnSet::POSITION | ColumnSet::NORMAL)],
        "the written node"
    );
    let normals_only: Vec<u32> = others.iter().map(|d| d.0).collect();
    assert!(
        others.iter().all(|d| d.1 == ColumnSet::NORMAL),
        "neighbours: normals"
    );
    for n in changed(&before, &after) {
        assert!(
            n == 4 || normals_only.contains(&n),
            "node {n} changed without a dirty bit"
        );
    }
    assert!(!normals_only.is_empty(), "node 4 has neighbours");
}

#[test]
fn an_unwritten_attribute_reads_as_zeros_and_holds_no_memory() {
    let mut s = store(&sphere(6, 0.5, 2, false), NodeConfig::new(6, 40).unwrap());
    let mask = s.add_attr(gs_core::AttrDef::mask()).unwrap();
    let bytes = s.mesh_bytes();
    assert!(s.node(0).attr(mask).is_none(), "unallocated");
    s.write(&[0], ColumnSet::attr(mask)).each(|n| {
        let _ = n.attr(mask);
    });
    let slots = s.node(0).pos.len();
    assert_eq!(
        s.node(0).attr(mask),
        Some(&AttrVec::F32(vec![0.0; slots])),
        "allocated as zeros"
    );
    assert_eq!(s.mesh_bytes(), bytes + 4 * slots, "one column's bytes");
    s.write(&[0], ColumnSet::attr(mask))
        .each(|n| n.clear_attr(mask));
    assert!(s.node(0).attr(mask).is_none(), "cleared");
    assert_eq!(s.mesh_bytes(), bytes, "memory back");
}
