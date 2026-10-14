//! Node splitting and edge splits (the Live Detail foundation): the mesh stays the same shape,
//! ownership and seam lists stay right, full nodes are cut in two, and storage stays compact.

use gs_core::{AttrDef, MeshStore, NO, NodeConfig, Vec3};

use crate::common::{
    Snap, attr_bits, canonical, check, check_closed, fan, ok, refine, snap, some, sphere, store,
    surface_point,
};

fn small() -> NodeConfig {
    ok(NodeConfig::new(6, 40))
}

#[test]
fn splitting_a_node_halves_it_along_its_longest_axis_and_keeps_the_mesh() {
    let mut s = store(&sphere(10, 0.5, 31, true), NodeConfig::new(7, 100).unwrap());
    let before_mesh = canonical(&s.to_plain());
    let before = snap(&s);
    let x = 3;
    let live = before.nodes[x as usize].pos.len();
    let moved = s.edit_topology().split_node(x);
    let after = snap(&s);
    check(&after).unwrap();
    check_closed(&s).unwrap();
    assert!(canonical(&s.to_plain()) == before_mesh, "same mesh");
    assert_eq!(after.nodes.len(), before.nodes.len() + 1, "one node added");
    assert_eq!(moved.len(), live / 2, "half the vertices moved");
    let (y, cfg) = (after.nodes.len() - 1, after.cfg);
    let count = |k: usize| after.nodes[k].pos.iter().filter(|p| !p.x.is_nan()).count();
    assert_eq!(
        (count(x as usize), count(y)),
        (live - live / 2, live / 2),
        "halves"
    );
    assert!(
        moved.windows(2).all(|w| w[0].0 < w[1].0),
        "sorted by old id"
    );
    for &(old, new) in &moved {
        assert_eq!(
            (cfg.node_of(old), cfg.node_of(new) as usize),
            (x, y),
            "x to y"
        );
        assert!(!after.live(old), "old slot freed");
        assert_eq!(
            after.pos(new).bits(),
            before.pos(old).bits(),
            "same position"
        );
    }
    // The moved half is the upper half along the longest axis of x's vertices.
    let pts = &before.nodes[x as usize].pos;
    let extent = |i: usize| {
        let v: Vec<f32> = pts.iter().map(|p| p.to_array()[i]).collect();
        v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
    };
    let axis = (0..3)
        .max_by(|&i, &j| extent(i).total_cmp(&extent(j)))
        .unwrap();
    let coord = |v: u32| before.pos(v).to_array()[axis];
    let low = (before.nodes[x as usize].base..)
        .take(live)
        .filter(|v| moved.iter().all(|m| m.0 != *v));
    let kept_max = low.map(coord).fold(f32::MIN, f32::max);
    assert!(
        moved.iter().all(|m| coord(m.0) >= kept_max),
        "upper half along axis {axis}"
    );
    for (f, corners) in after.live_faces() {
        let in_x = corners.iter().any(|&v| cfg.node_of(v) == x);
        if after.face_node(f) == x as usize {
            assert!(in_x, "a face left in x keeps a corner there");
        }
        if after.face_node(f) == y {
            assert!(!in_x, "a face moved to y has none in x");
        }
    }
}

/// Every byte the store says its columns hold, counted from a snapshot: 28 bytes per vertex
/// slot (position, normal, ring header), 4 per ring entry, 16 per face slot, 4 per free slot,
/// 4 per allocated attribute value.
fn counted_bytes(sn: &Snap) -> usize {
    let mut b = 0;
    for n in &sn.nodes {
        let dead_v = n.pos.iter().filter(|p| p.x.is_nan()).count();
        let dead_f = n.faces.iter().filter(|f| f[0] == NO).count();
        let rings: usize = n.rings.iter().map(Vec::len).sum();
        let attrs: usize = n.attrs.iter().flatten().map(|a| attr_bits(a).len()).sum();
        b += 28 * n.pos.len() + 4 * rings + 16 * n.faces.len() + 4 * (dead_v + dead_f + attrs);
    }
    b
}

#[test]
fn refining_a_region_splits_full_nodes_and_keeps_storage_compact() {
    let cfg = small();
    let mut s = store(&sphere(10, 1.0, 32, true), cfg);
    let weight = s.add_attr(AttrDef::new(
        "weight",
        gs_core::Domain::Vertex,
        gs_core::AttrKind::Scalar,
    ));
    let weight = weight.unwrap();
    let (nodes, faces) = (s.node_count(), s.face_count());
    let c = surface_point(&s, Vec3::new(-0.3, 0.4, 1.0));
    let splits = refine(&mut s, c, 0.4, 0.02, 4);
    let sn = snap(&s);
    check(&sn).unwrap();
    check_closed(&s).unwrap();
    assert!(splits >= 3, "{splits} node splits");
    assert_eq!(s.node_count(), nodes + splits, "one new node per split");
    assert!(s.face_count() > faces + 400, "the region was refined");
    assert_eq!(s.vertex_count(), sn.live_vertices(), "live vertices");
    assert_eq!(s.face_count(), sn.live_faces().len(), "live faces");
    // Ring pools are compacted only once their garbage passes their live entries (decision
    // 0021, section 6), so at most that much is left over.
    let rings: usize = (sn.nodes.iter().flat_map(|n| &n.rings)).map(Vec::len).sum();
    let (bytes, counted) = (s.mesh_bytes(), counted_bytes(&sn));
    assert!(
        bytes >= counted && bytes <= counted + 4 * rings,
        "garbage in the columns: {bytes} bytes held, {counted} live"
    );
    assert!(
        sn.nodes
            .iter()
            .all(|n| n.pos.len() <= cfg.capacity() as usize),
        "no node past its slots"
    );
    assert!(
        (0..s.node_count() as u32).all(|n| s.node(n).attr(weight).is_none()),
        "zeros allocate nothing"
    );
    let out = s.to_plain();
    assert_eq!(
        (out.positions.len(), out.faces.len()),
        (s.vertex_count(), s.face_count()),
        "export skips dead slots"
    );
    let again = store(&out, cfg);
    check_closed(&again).unwrap();
}

#[test]
fn an_edge_split_blends_attributes_and_copies_face_groups() {
    let mut s = store(&sphere(6, 1.0, 33, true), small());
    let before = snap(&s);
    let ids = ["mask", "color", "group"].map(|n| some(s.attr_id(n)).index());
    let bits = |sn: &Snap, a: usize, v: u32| {
        let n = &sn.nodes[sn.node_of(v)];
        attr_bits(some(n.attrs[a].as_ref()))[(v - n.base) as usize]
    };
    let group_of = |sn: &Snap, f: u32| {
        let n = &sn.nodes[sn.face_node(f)];
        attr_bits(some(n.attrs[ids[2]].as_ref()))[(f - n.first_face) as usize]
    };
    let faces = before.live_faces();
    // A triangle whose neighbour across its first edge is in another group.
    let (f, other, corners) = (faces.iter())
        .find_map(|(f, cs)| {
            let (a, b) = (cs[0], cs[1]);
            let (g, _) = faces
                .iter()
                .find(|(g, gs)| g != f && gs.contains(&a) && gs.contains(&b))?;
            (group_of(&before, *f) != group_of(&before, *g)).then(|| (*f, *g, cs.clone()))
        })
        .unwrap();
    let (a, b, c) = (corners[0], corners[1], corners[2]);
    let m = s.edit_topology().split_edge(a, b).unwrap();
    let after = snap(&s);
    check(&after).unwrap();
    check_closed(&s).unwrap();
    let (x, y) = (
        f32::from_bits(bits(&before, ids[0], a)),
        f32::from_bits(bits(&before, ids[0], b)),
    );
    assert_eq!(
        bits(&after, ids[0], m),
        gs_core::mask::settle([x, y].iter().sum::<f32>() * 0.5).to_bits(),
        "mask blended, then settled on its grid"
    );
    let (ca, cb) = (bits(&before, ids[1], a), bits(&before, ids[1], b));
    let mean = (0..4).fold(0, |o, k| {
        o + (((ca >> (8 * k) & 255) + (cb >> (8 * k) & 255)).div_ceil(2) << (8 * k))
    });
    assert_eq!(bits(&after, ids[1], m), mean, "color blended per channel");
    let mid = (before.pos(a) + before.pos(b)) * 0.5;
    assert_eq!(after.pos(m).bits(), mid.bits(), "midpoint");
    let ring = &after.nodes[after.node_of(m)].rings[after.cfg.local_of(m) as usize];
    assert_eq!(ring.len(), 4, "four faces around the new vertex");
    for &g in ring {
        let side = if after.face(g).contains(&c) { f } else { other };
        assert_eq!(
            group_of(&after, g),
            group_of(&before, side),
            "face {g} keeps its group"
        );
    }
}

#[test]
fn an_edge_split_that_fills_a_node_cuts_it_and_still_splits() {
    let mut s = store(&sphere(8, 1.0, 35, false), small());
    let sn = snap(&s);
    let a = sn.nodes[2].base;
    let mut b = some(
        sn.face(sn.nodes[2].rings[0][0])
            .into_iter()
            .find(|&v| v != a),
    );
    let mut ed = s.edit_topology();
    // Every split puts its new vertex in a's node; split a's edge to the last new vertex.
    for i in 0..30 {
        let before = ed.node_splits();
        let m = ed.split_edge(a, b);
        assert!(m.is_some(), "split {i}");
        if ed.node_splits() > before {
            return;
        }
        b = some(m);
    }
    panic!("30 splits into one node never filled it");
}

#[test]
fn a_node_splits_only_when_it_lacks_room_for_two_faces() {
    // Four vertex slots and sixteen face slots per node; the center's node owns the fan.
    let cfg = ok(NodeConfig::new(2, 1));
    // The center is vertex 0 of node 0; the second face's rim vertices are interior to the fan.
    let edge = |s: &MeshStore| (0, s.node(0).faces[1][1]);
    let mut s = ok(MeshStore::from_plain(&fan(14), cfg));
    let (a, b) = edge(&s);
    let mut ed = s.edit_topology();
    assert!(
        ed.split_edge(a, b).is_some(),
        "two free face slots are enough"
    );
    assert_eq!(ed.node_splits(), 0, "no split needed");
    drop(ed);
    let mut s = ok(MeshStore::from_plain(&fan(15), cfg));
    let (a, b) = edge(&s);
    let mut ed = s.edit_topology();
    assert_eq!(
        ed.split_edge(a, b),
        None,
        "a lone vertex cannot make room by splitting"
    );
    assert_eq!(ed.node_splits(), 1, "but its node was cut");
}

#[test]
fn only_live_vertices_and_edges_between_two_triangles_split() {
    let mut s = store(&sphere(4, 0.0, 34, false), small());
    let sn = snap(&s);
    let (_, q) = sn.live_faces()[0].clone();
    let cfg = sn.cfg;
    let past_end = cfg.vert(0, sn.nodes[0].pos.len() as u32);
    let no_node = cfg.vert(sn.nodes.len() as u32, 0);
    let mut ed = s.edit_topology();
    assert_eq!(ed.split_edge(q[0], q[1]), None, "a quad edge");
    assert_eq!(ed.split_edge(q[0], q[2]), None, "a quad diagonal");
    assert_eq!(
        ed.split_edge(past_end, q[0]),
        None,
        "a slot past the node's end"
    );
    assert_eq!(
        ed.split_edge(no_node, q[0]),
        None,
        "a node that does not exist"
    );
    assert_eq!(ed.split_edge(q[0], past_end), None, "the other end missing");
    assert!(ed.is_live(q[0]), "a live vertex");
    assert!(
        !ed.is_live(past_end) && !ed.is_live(no_node),
        "missing slots"
    );
    let moved = ed.split_node(0);
    assert!(
        !ed.is_live(moved[0].0) && ed.is_live(moved[0].1),
        "a freed slot is dead"
    );
}

#[test]
fn undo_and_redo_of_added_nodes_survive_a_new_attribute() {
    let mut s = store(&sphere(8, 1.0, 36, true), small());
    let c = surface_point(&s, Vec3::new(0.1, -0.2, -1.0));
    let mesh0 = s.mesh_bytes();
    s.begin_step();
    assert!(refine(&mut s, c, 0.4, 0.02, 3) > 0, "nodes added");
    s.end_step();
    let mesh1 = s.mesh_bytes();
    let after = crate::common::fingerprint(&snap(&s));
    s.undo();
    assert!(
        s.history_bytes() >= mesh1 - mesh0,
        "history holds the added nodes"
    );
    let extra = ok(s.add_attr(AttrDef::new(
        "late",
        gs_core::Domain::Face,
        gs_core::AttrKind::Label,
    )));
    s.redo();
    let sn = snap(&s);
    check(&sn).unwrap();
    assert!(
        (0..s.node_count() as u32).all(|n| s.node(n).attr(extra).is_none()),
        "the new column is empty"
    );
    let mut without = sn.clone();
    for n in &mut without.nodes {
        n.attrs.pop();
    }
    assert_eq!(
        crate::common::fingerprint(&without),
        after,
        "redo restored the refined mesh"
    );
}
