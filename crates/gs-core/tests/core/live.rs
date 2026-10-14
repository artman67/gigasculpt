//! Live Detail's pass (decision 0021, section 2), run directly on the store: random passes keep
//! a valid closed sphere with every derived list and normal exact (property, against the
//! oracle's full recompute); the budget bounds every pass; a pass run to rest leaves no long
//! edge; masks, hidden faces and face groups are respected, and group borders stay where they
//! are; new points blend their parents'
//! masks; the same bits on 1 and 24 threads; history holds only nodes whose data changed;
//! flips even out valences inside, on borders and around poles past 16 faces; a whole part or
//! a picked region refines to rest.

use std::collections::{BTreeMap, BTreeSet};

use gs_core::live::{LiveDetail, LiveReport, Target, refine, refine_all};
use gs_core::select::Selector;
use gs_core::{MeshStore, NO, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::common::{
    Snap, canonical, check, check_closed, dir, fingerprint, mix, ok, small_nodes, snap, some,
    sphere, store, surface_point, turned_and_flat, unit,
};

/// Settings with a fixed target length.
fn constant(len: f32) -> LiveDetail {
    LiveDetail {
        target: Target::Constant(len),
        ..LiveDetail::default()
    }
}

/// A pass sphere at a random spot on the surface, radius 0.2 to 0.5.
fn spot(s: &MeshStore, i: u64) -> (Vec3, f32) {
    let r = 0.2 + 0.15 * (unit(mix(i)) + 1.0);
    (surface_point(s, dir(i)), r)
}

fn pass(s: &mut MeshStore, at: (Vec3, f32), ld: &LiveDetail) -> LiveReport {
    let mut out = LiveReport::default();
    refine(s, &[at], ld, [false; 3], &mut out);
    out
}

/// A pass at [`spot`] `i`.
fn pass_at(s: &mut MeshStore, i: u64, ld: &LiveDetail) -> LiveReport {
    let at = spot(s, i);
    pass(s, at, ld)
}

fn corners(sn: &Snap, c: &[u32]) -> Vec<Vec3> {
    c.iter().map(|&v| sn.pos(v)).collect()
}

/// Every face of a star-shaped sphere around the origin faces away from it.
fn outward(sn: &Snap) -> Result<(), String> {
    for (f, c) in sn.live_faces() {
        let p = corners(sn, &c);
        let n = if p.len() == 3 {
            (p[1] - p[0]).cross(p[2] - p[0])
        } else {
            (p[2] - p[0]).cross(p[3] - p[1])
        };
        let mid = p.iter().fold(Vec3::ZERO, |a, &q| a + q);
        if n.dot(mid) <= 0.0 {
            return Err(format!("face {f} {c:?} faces inward"));
        }
    }
    Ok(())
}

pub(crate) fn valid(s: &MeshStore) -> Result<(), String> {
    let sn = snap(s);
    check(&sn)?;
    check_closed(s)?;
    outward(&sn)
}

#[test]
fn random_passes_keep_a_valid_closed_sphere() {
    for (tris, seed) in [(1.0, 3), (0.5, 4)] {
        let mut s = store(&sphere(8, tris, seed, false), small_nodes());
        let mut total = LiveReport::default();
        for i in 0..60 {
            let len = [0.06, 0.03, 0.35, 0.12][i as usize % 4];
            let ld = LiveDetail {
                budget: 50 + (mix(seed * 7 + i) % 400) as u32,
                merge: 0.2 + 0.125 * (unit(mix(seed + i)) + 1.0),
                ..constant(len)
            };
            let out = pass_at(&mut s, seed * 100 + i, &ld);
            if let Err(e) = valid(&s) {
                panic!("pass {i} of mesh {seed}: {e}");
            }
            total.splits += out.splits;
            total.collapses += out.collapses;
            total.flips += out.flips;
            total.quad_splits += out.quad_splits;
            total.node_cuts += out.node_cuts;
        }
        assert!(total.splits > 100 && total.collapses > 100, "{total:?}");
        assert!(total.flips > 10 && total.node_cuts > 0, "{total:?}");
        assert_eq!(total.quad_splits > 0, tris < 1.0, "{total:?}");
    }
}

#[test]
fn the_budget_bounds_every_pass() {
    let mut s = store(&sphere(8, 1.0, 5, false), small_nodes());
    let ld = LiveDetail {
        budget: 16,
        ..constant(0.03)
    };
    let mut hits = 0;
    for i in 0..10 {
        let out = pass_at(&mut s, i, &ld);
        assert!(out.most_ops <= 16 && out.ops() <= 16, "{out:?}");
        hits += out.budget_hits;
    }
    assert!(
        hits > 0,
        "a fine target on a coarse mesh runs out of budget"
    );
}

/// Sides of equal length end the pass: the split step once sent an edge and its equally long
/// side back to the queue in turn forever (review 0006, C1). A pass that stops making progress
/// fails a debug build rather than hanging.
#[test]
fn a_pass_over_equal_sides_ends() {
    let tri = |order: [usize; 3]| {
        let p = [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.05, 0.3, 0.0]];
        PlainMesh {
            positions: order.map(|i| p[i]).to_vec(),
            faces: vec![[0, 1, 2, NO]],
            ..PlainMesh::default()
        }
    };
    let mut stretched = crate::brush_kit::grid(8);
    for p in &mut stretched.positions {
        p[0] *= 2.0;
    }
    for (m, at, len) in [
        (tri([0, 1, 2]), (Vec3::new(0.05, 0.1, 0.0), 0.5), 0.03),
        (tri([2, 0, 1]), (Vec3::new(0.05, 0.1, 0.0), 0.5), 0.03),
        (stretched, (Vec3::new(0.1, 0.05, 0.0), 0.4), 0.05),
    ] {
        let mut s = store(&m, small_nodes());
        let out = pass(&mut s, at, &constant(len));
        assert!(out.ops() > 0 && out.budget_hits == 0, "{out:?}");
        ok(check(&snap(&s)));
    }
}

/// Every edge passing inside the sphere, with its length.
fn footprint_edges(sn: &Snap, c: Vec3, r: f32) -> Vec<(u32, u32, f32)> {
    let mut out = Vec::new();
    for (_, cs) in sn.live_faces() {
        for i in 0..cs.len() {
            let (a, b) = (cs[i], cs[(i + 1) % cs.len()]);
            let (pa, pb) = (sn.pos(a), sn.pos(b));
            let ab = pb - pa;
            let t = ((c - pa).dot(ab) / ab.len2()).clamp(0.0, 1.0);
            if (c - (pa + ab * t)).len2() <= r * r {
                out.push((a, b, ab.len2().sqrt()));
            }
        }
    }
    out
}

/// Passes at one spot come to rest with no long edge left in it, also when each runs out of a
/// small budget and leaves the rest to the next.
#[test]
fn a_pass_run_to_rest_leaves_no_long_edge() {
    for budget in [LiveDetail::default().budget, 24] {
        let mut s = store(&sphere(6, 1.0, 6, false), small_nodes());
        let at = (surface_point(&s, Vec3::new(0.2, 1.0, 0.1)), 0.4);
        let ld = LiveDetail {
            budget,
            ..constant(0.08)
        };
        let (mut rested, mut hits) = (false, 0);
        for _ in 0..200 {
            let out = pass(&mut s, at, &ld);
            hits += out.budget_hits;
            if out.ops() == 0 && out.waits == 0 {
                rested = true;
                break;
            }
        }
        assert!(rested, "budget {budget}: the passes come to rest");
        assert_eq!(hits > 0, budget == 24, "budget {budget}: {hits} hits");
        let sn = snap(&s);
        for (a, b, len) in footprint_edges(&sn, at.0, at.1) {
            assert!(
                len <= 0.08 * 1.0001,
                "budget {budget}: edge {a}-{b} is {len} long"
            );
        }
        ok(valid(&s));
    }
}

/// Live positions as bits, those farther than `d` from `c`.
fn far_points(s: &MeshStore, c: Vec3, d: f32) -> BTreeSet<[u32; 3]> {
    let sn = snap(s);
    (sn.nodes.iter().flat_map(|n| &n.pos))
        .filter(|p| !p.is_dead() && (**p - c).len2() > d * d)
        .map(|p| p.bits())
        .collect()
}

#[test]
fn points_far_from_the_pass_keep_their_bits() {
    let mut s = store(&sphere(8, 1.0, 7, false), small_nodes());
    for i in 0..12 {
        let at = spot(&s, 40 + i);
        let len = if i % 2 == 0 { 0.05 } else { 0.3 };
        // A pass moves points only inside its sphere grown by the target length.
        let reach = at.1 + len;
        let before = far_points(&s, at.0, reach);
        pass(&mut s, at, &constant(len));
        let after = far_points(&s, at.0, reach);
        assert!(
            before.is_subset(&after),
            "pass {i} moved or removed a far point"
        );
    }
}

/// `m` with a vertex column `name` from `f(position)`, or a face column from the face center.
fn with_column(m: &mut PlainMesh, name: &str, domain: AttrDomain, f: impl Fn(Vec3) -> AttrValue) {
    let at = |v: u32| Vec3::from_array(m.positions[v as usize]);
    let values: Vec<AttrValue> = match domain {
        AttrDomain::Vertex => (0..m.positions.len() as u32).map(|v| f(at(v))).collect(),
        AttrDomain::Face => (m.faces.iter())
            .map(|fc| {
                let c: Vec<u32> = fc.iter().copied().filter(|&v| v != NO).collect();
                f(c.iter().fold(Vec3::ZERO, |s, &v| s + at(v)) * (1.0 / c.len() as f32))
            })
            .collect(),
    };
    let values = match values[0] {
        AttrValue::F(_) => AttrValues::F32(values.iter().copied().map(AttrValue::f).collect()),
        AttrValue::U(_) => AttrValues::U32(values.iter().copied().map(AttrValue::u).collect()),
    };
    m.attributes
        .insert(name.into(), Attribute { domain, values });
}

#[derive(Clone, Copy)]
enum AttrValue {
    F(f32),
    U(u32),
}
impl AttrValue {
    fn f(self) -> f32 {
        match self {
            AttrValue::F(x) => x,
            AttrValue::U(x) => x as f32,
        }
    }
    fn u(self) -> u32 {
        match self {
            AttrValue::U(x) => x,
            AttrValue::F(x) => x as u32,
        }
    }
}

/// Corner positions (as bits, sorted) of every face whose column `name` reads `want`.
fn faces_where(s: &MeshStore, name: &str, want: u32) -> BTreeSet<Vec<[u32; 3]>> {
    let col = s.attr_reader(some(s.attr_id(name)));
    let sn = snap(s);
    (sn.live_faces().into_iter())
        .filter(|(f, _)| col.label(*f) == want)
        .map(|(_, c)| {
            let mut k: Vec<[u32; 3]> = c.iter().map(|&v| sn.pos(v).bits()).collect();
            k.sort_unstable();
            k
        })
        .collect()
}

/// Positions (as bits) of every live point whose mask is at least 1.
fn held_points(s: &MeshStore) -> BTreeSet<[u32; 3]> {
    let mask = s.attr_reader(some(s.attr_id("mask")));
    let sn = snap(s);
    (sn.nodes.iter())
        .flat_map(|n| (0..n.pos.len()).map(move |l| (n.base + l as u32, n.pos[l])))
        .filter(|&(v, p)| !p.is_dead() && mask.scalar(v) >= 1.0)
        .map(|(_, p)| p.bits())
        .collect()
}

#[test]
fn masks_hidden_faces_and_groups_are_respected() {
    let mut m = sphere(8, 1.0, 8, false);
    with_column(&mut m, "mask", AttrDomain::Vertex, |p| {
        AttrValue::F(if p.x > 0.3 { 1.0 } else { (p.x + 1.0) * 0.4 })
    });
    with_column(&mut m, "hide", AttrDomain::Face, |p| {
        AttrValue::U(u32::from(p.y > 0.5))
    });
    with_column(&mut m, "group", AttrDomain::Face, |p| {
        AttrValue::U(if p.z > 0.0 { 7 } else { 3 })
    });
    let mut s = store(&m, small_nodes());
    let (held, hidden) = (held_points(&s), faces_where(&s, "hide", 1));
    let mut total = LiveReport::default();
    for i in 0..40 {
        let ld = constant(if i % 3 == 2 { 0.4 } else { 0.05 });
        let out = pass_at(&mut s, 70 + i, &ld);
        total.splits += out.splits;
        total.collapses += out.collapses;
        ok(valid(&s));
    }
    assert!(total.splits > 50 && total.collapses > 20, "{total:?}");
    assert!(
        held.is_subset(&held_points(&s)),
        "a fully masked point moved"
    );
    assert_eq!(faces_where(&s, "hide", 1), hidden, "a hidden face changed");
    let sn = snap(&s);
    let group = s.attr_reader(some(s.attr_id("group")));
    for (f, c) in sn.live_faces() {
        let z: Vec<f32> = c.iter().map(|&v| sn.pos(v).z).collect();
        let side = if z.iter().all(|&z| z > 0.05) {
            7
        } else if z.iter().all(|&z| z < -0.05) {
            3
        } else {
            continue;
        };
        assert_eq!(group.label(f), side, "face {f} at z {z:?} left its group");
    }
}

/// The positions (as bits) of the points on a group border of `s`, where faces of two groups
/// meet, and of every live point.
fn group_border(s: &MeshStore) -> (BTreeSet<[u32; 3]>, BTreeSet<[u32; 3]>) {
    let group = s.attr_reader(some(s.attr_id("group")));
    let sn = snap(s);
    let mut groups: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (f, c) in sn.live_faces() {
        for v in c {
            groups.entry(v).or_default().insert(group.label(f));
        }
    }
    let border = (groups.into_iter())
        .filter(|(_, g)| g.len() > 1)
        .map(|(v, _)| sn.pos(v).bits())
        .collect();
    let live = (sn.nodes.iter())
        .flat_map(|n| n.pos.iter().filter(|p| !p.is_dead()).map(|p| p.bits()))
        .collect();
    (border, live)
}

/// Flips never cross a group border (decision 0026): no pass over a sphere with three groups
/// puts a point that was inside a group, where it was, on a border. Splits copy their face's
/// group and collapses keep the faces they move, so only a new or moved point can join a
/// border; a flip across one would bring both far corners onto it.
#[test]
fn refining_never_moves_a_group_border() {
    let mut m = sphere(8, 1.0, 5, false);
    with_column(&mut m, "group", AttrDomain::Face, |p| {
        AttrValue::U(if p.z > 0.3 {
            1
        } else if p.x > 0.0 {
            2
        } else {
            3
        })
    });
    let mut s = store(&m, small_nodes());
    let mut flips = 0;
    for i in 0..30 {
        let (border, live) = group_border(&s);
        let ld = constant([0.05, 0.03, 0.4][i as usize % 3]);
        flips += pass_at(&mut s, 90 + i, &ld).flips;
        let joined = (group_border(&s).0.iter())
            .filter(|p| live.contains(*p) && !border.contains(*p))
            .count();
        assert_eq!(
            joined, 0,
            "pass {i}: points from inside a group joined a border"
        );
    }
    assert!(flips > 100, "{flips} flips, too few to test the rule");
}

#[test]
fn new_points_take_their_parents_mean_mask() {
    let mut m = sphere(6, 1.0, 9, false);
    let field = |p: Vec3| 0.5 + 0.4 * p.x;
    with_column(&mut m, "mask", AttrDomain::Vertex, |p| {
        AttrValue::F(field(p))
    });
    let mut s = store(&m, small_nodes());
    let ld = LiveDetail {
        combined: false,
        smooth_split: false,
        relax: 0.0,
        ..constant(0.05)
    };
    let mut splits = 0;
    for i in 0..10 {
        splits += pass_at(&mut s, 90 + i, &ld).splits;
    }
    assert!(splits > 50);
    let mask = s.attr_reader(some(s.attr_id("mask")));
    let sn = snap(&s);
    for n in &sn.nodes {
        for (l, p) in n.pos.iter().enumerate().filter(|x| !x.1.is_dead()) {
            let v = n.base + l as u32;
            let got = mask.scalar(v);
            assert!(
                (got - field(*p)).abs() < 1e-5,
                "point {v}: {got} vs {}",
                field(*p)
            );
        }
    }
}

fn script() -> Vec<u64> {
    let mut s = store(&sphere(8, 0.7, 10, true), small_nodes());
    let mut prints = Vec::new();
    for i in 0..16 {
        let ld = constant(if i % 4 == 3 { 0.3 } else { 0.05 });
        pass_at(&mut s, 110 + i, &ld);
        prints.push(fingerprint(&snap(&s)));
    }
    prints
}

#[test]
fn passes_are_bit_identical_on_1_and_24_threads() {
    let runs = [1, 24].map(|t| gs_par::with_threads(t, script));
    assert_eq!(runs[0], runs[1]);
}

/// Nodes whose positions, normals, faces, rings or attributes differ in any bit, or are new.
pub(crate) fn changed_nodes(a: &Snap, b: &Snap) -> usize {
    (0..b.nodes.len())
        .filter(|&k| {
            let Some(x) = a.nodes.get(k) else { return true };
            let y = &b.nodes[k];
            let bits = |v: &[Vec3]| v.iter().map(|p| p.bits()).collect::<Vec<_>>();
            bits(&x.pos) != bits(&y.pos)
                || bits(&x.nrm) != bits(&y.nrm)
                || x.faces != y.faces
                || x.rings != y.rings
                || x.attrs != y.attrs
        })
        .count()
}

#[test]
fn history_holds_only_nodes_whose_data_changed() {
    let mut s = store(&sphere(10, 1.0, 11, true), small_nodes());
    for i in 0..8 {
        let before = snap(&s);
        s.begin_step();
        pass_at(&mut s, 130 + i, &constant(0.05));
        let held = s.end_step().nodes;
        let changed = changed_nodes(&before, &snap(&s));
        assert!(changed > 0);
        assert_eq!(held, changed, "pass {i}: history holds only changed nodes");
    }
}

#[test]
fn compaction_leaves_no_dead_slot_and_the_same_mesh() {
    let mut s = store(&sphere(8, 0.7, 12, true), small_nodes());
    for i in 0..12 {
        let ld = constant(if i % 3 == 2 { 0.3 } else { 0.05 });
        pass_at(&mut s, 150 + i, &ld);
    }
    let before = canonical(&s.to_plain());
    let dead = snap(&s)
        .nodes
        .iter()
        .any(|n| n.pos.iter().any(|p| p.is_dead()) || n.faces.iter().any(|f| f[0] == NO));
    assert!(dead, "the passes left dead slots to compact");
    s.begin_step();
    let compacted = s.compact_slots();
    s.end_step();
    assert_ne!(
        compacted,
        [] as [u32; 0],
        "compaction rewrote the nodes with dead slots"
    );
    let sn = snap(&s);
    for (k, n) in sn.nodes.iter().enumerate() {
        assert!(
            n.pos.iter().all(|p| !p.is_dead()),
            "node {k} keeps a dead point"
        );
        assert!(
            n.faces.iter().all(|f| f[0] != NO),
            "node {k} keeps a dead face"
        );
    }
    ok(valid(&s));
    assert_eq!(
        canonical(&s.to_plain()),
        before,
        "the mesh itself is unchanged"
    );
    assert!(s.compact_slots().is_empty(), "nothing is left to compact");
}

/// C1's degenerate grid: `grid(16)` with one column of points moved onto the next, so the 16
/// quads between them have no area (review 0006, S3); cut into triangles when `tris`.
fn degenerate_grid(tris: bool) -> PlainMesh {
    let mut m = crate::brush_kit::grid(16);
    for j in 0..17 {
        m.positions[8 * 17 + j] = m.positions[9 * 17 + j];
    }
    if tris {
        m.faces = (m.faces.iter())
            .flat_map(|f| [[f[0], f[1], f[2], NO], [f[0], f[2], f[3], NO]])
            .collect();
    }
    m
}

/// Review 0006, S3: refining a mesh with faces without area never makes more of them, nor
/// turns a face, with or without the collapse step; and the collapse step removes those with
/// two corners at one point (splitting one used to make two more, 16 growing past 450).
#[test]
fn faces_without_area_are_never_multiplied() {
    for tris in [false, true] {
        for combined in [true, false] {
            let mut s = store(&degenerate_grid(tris), small_nodes());
            let (_, start) = turned_and_flat(&snap(&s));
            assert_eq!(start, if tris { 32 } else { 16 });
            let mut flat = start;
            for i in 0..40u64 {
                let c = Vec3::new(0.8 * unit(mix(3 * i + 1)), 0.8 * unit(mix(3 * i + 2)), 0.0);
                let r = 0.2 + 0.15 * (unit(mix(5 * i)) + 1.0);
                let ld = LiveDetail {
                    combined,
                    ..constant([0.06, 0.03, 0.12][i as usize % 3])
                };
                pass(&mut s, (c, r), &ld);
                ok(check(&snap(&s)));
                let turned;
                (turned, flat) = turned_and_flat(&snap(&s));
                let at = format!("tris {tris}, combined {combined}, pass {i}");
                assert!(
                    flat <= start,
                    "{at}: {flat} faces without area, from {start}"
                );
                assert_eq!(turned, 0, "{at}: a face turned over");
            }
            if tris && combined {
                assert!(flat < start / 2, "the collapse step left {flat} of {start}");
            }
        }
    }
}

// ---------------------------------------------------------------- the whole part (L4)

/// [`refine_all`] over all of `s`, or what `sel` picks, with `ld`, at most 64 passes, as one
/// history step: its report and whether it came to rest.
fn refine_whole(s: &mut MeshStore, sel: Option<&Selector>, ld: &LiveDetail) -> (LiveReport, bool) {
    let mut out = LiveReport::default();
    s.begin_step();
    let (_, rest) = refine_all(s, sel, ld, [false; 3], 64, &mut out);
    s.end_step();
    (out, rest)
}

/// The longest edge of `s`'s live faces.
fn longest_edge(s: &MeshStore) -> f32 {
    let sn = snap(s);
    let mut most = 0.0f32;
    for (_, c) in sn.live_faces() {
        for i in 0..c.len() {
            most = most.max((sn.pos(c[i]) - sn.pos(c[(i + 1) % c.len()])).len2().sqrt());
        }
    }
    most
}

/// `mesh.refine`'s pass over a whole part comes to rest with no edge longer than the target and
/// no dead slot, and a second run changes nothing (Blender's Detail Flood Fill).
#[test]
fn refining_a_whole_part_comes_to_rest_and_again_changes_nothing() {
    let mut s = store(&sphere(8, 1.0, 3, false), small_nodes());
    let (out, rest) = refine_whole(&mut s, None, &constant(0.1));
    assert!(rest, "the passes came to rest: {out:?}");
    assert!(out.splits > 500 && out.passes > 1, "{out:?}");
    ok(valid(&s));
    let longest = longest_edge(&s);
    assert!(longest <= 0.1 * 1.0001, "an edge {longest} long is left");
    let dead = (snap(&s).nodes.iter())
        .any(|n| n.pos.iter().any(|p| p.is_dead()) || n.faces.iter().any(|f| f[0] == NO));
    assert!(!dead, "no dead slot outlives the command");
    let before = fingerprint(&snap(&s));
    let (again, rest) = refine_whole(&mut s, None, &constant(0.1));
    assert!(rest && again.passes == 1 && !again.changed(), "{again:?}");
    assert_eq!(
        fingerprint(&snap(&s)),
        before,
        "the second run changed the part"
    );
}

/// A cap of one pass stops a fine target short and says so; the rest comes on the next run.
#[test]
fn the_pass_cap_stops_short_and_says_so() {
    let mut s = store(&sphere(8, 1.0, 5, false), small_nodes());
    let mut out = LiveReport::default();
    let (_, rest) = refine_all(&mut s, None, &constant(0.03), [false; 3], 1, &mut out);
    assert!(!rest && out.passes == 1 && out.ops() > 0, "{out:?}");
    ok(valid(&s));
    let (out, rest) = refine_whole(&mut s, None, &constant(0.03));
    assert!(rest && out.ops() > 0, "{out:?}");
}

/// A selector limits the passes to the faces it picks: every point of another group keeps its
/// place bit for bit, and only splits reach past the region, as a stroke's do (longest-edge
/// splits propagate to keep the surface conforming), so faces there gain points but lose none.
#[test]
fn a_selected_region_refines_alone() {
    let mut m = sphere(8, 1.0, 6, false);
    with_column(&mut m, "group", AttrDomain::Face, |p| {
        AttrValue::U(if p.z > 0.2 { 1 } else { 2 })
    });
    let mut s = store(&m, small_nodes());
    let (border, _) = group_border(&s);
    let inside = faces_where(&s, "group", 1).len();
    let other: BTreeSet<[u32; 3]> = (faces_where(&s, "group", 2).into_iter())
        .flatten()
        .filter(|p| !border.contains(p))
        .collect();
    let (out, rest) = refine_whole(&mut s, Some(&Selector::Groups(vec![1])), &constant(0.08));
    assert!(rest && out.splits > 100, "{out:?}");
    ok(valid(&s));
    assert!(
        faces_where(&s, "group", 1).len() > 2 * inside,
        "the region refined"
    );
    let (_, live) = group_border(&s);
    let moved = other.difference(&live).count();
    assert_eq!(moved, 0, "{moved} points off the region moved or went");
}

// ---------------------------------------------------------------- the flip step's valences (L4)

/// Each live point's valence (the distinct points sharing an edge with it) and whether it lies
/// on an open border (an edge of it with one face), counted from the faces alone.
fn valences(s: &MeshStore) -> BTreeMap<u32, (usize, bool)> {
    let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (_, c) in snap(s).live_faces() {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut out: BTreeMap<u32, (usize, bool)> = BTreeMap::new();
    for (&(a, b), &faces) in &edges {
        for v in [a, b] {
            let at = out.entry(v).or_default();
            at.0 += 1;
            at.1 |= faces == 1;
        }
    }
    out
}

/// Over inside and border points: how many, and their squared valence deviations from 6 and
/// 4 summed.
fn deviation(s: &MeshStore) -> [(usize, usize); 2] {
    let mut out = [(0, 0); 2];
    for (val, border) in valences(s).into_values() {
        let (k, target) = if border { (1, 4) } else { (0, 6) };
        out[k].0 += 1;
        out[k].1 += val.abs_diff(target).pow(2);
    }
    out
}

/// A flat disc of `n` triangles around one point, radius 1, on z = 0.
fn fan_disc(n: u32) -> PlainMesh {
    let mut m = PlainMesh {
        positions: vec![[0.0; 3]],
        ..PlainMesh::default()
    };
    for k in 0..n {
        let t = std::f64::consts::TAU * f64::from(k) / f64::from(n);
        m.positions
            .push([libm::cos(t) as f32, libm::sin(t) as f32, 0.0]);
        m.faces.push([0, 1 + k, 1 + (k + 1) % n, NO]);
    }
    m
}

/// The flip step counts each point's valence from one sort of its ring (`TopoEditor::fan`): on
/// the stack up to 16 faces, on the heap past that, with 4 the aim on an open border. No test
/// noticed a wrong count (the stroke-step spike, SP2's round 2; the flip step also checks the
/// count against the plain neighbour list in debug builds now). Refined whole, discs around a
/// point of valence 20 and 24 (past the stack) with open rims, and an open grid: every pole
/// is gone, no point is left above valence 9, and the passes come to rest and stay there,
/// which they do only while every flip lowers the deviation (one that keeps it can undo
/// another). A heap count missing a face kept the poles at 20 and 24; counting every side
/// twice, or aiming border points at 6, kept a disc or the grid from resting.
#[test]
fn flips_even_out_valences_on_every_kind_of_point() {
    for (name, m, len) in [
        ("disc of 20", fan_disc(20), 0.3),
        ("disc of 24", fan_disc(24), 0.15),
        ("grid", crate::brush_kit::grid(6), 0.1),
    ] {
        let mut s = store(&m, small_nodes());
        let pole = valences(&s).values().map(|v| v.0).max();
        let (out, rest) = refine_whole(&mut s, None, &constant(len));
        assert!(rest && out.flips > 50, "{name}: {out:?}");
        ok(check(&snap(&s)));
        let most = some(valences(&s).values().map(|v| v.0).max());
        assert!(
            most <= 9,
            "{name}: a point of valence {most} is left, from {pole:?}"
        );
        let [inside, border] = deviation(&s);
        assert!(
            border.0 > 0 && inside.0 > border.0,
            "{name}: {inside:?} {border:?}"
        );
        let before = fingerprint(&snap(&s));
        let (again, rest) = refine_whole(&mut s, None, &constant(len));
        assert!(rest && !again.changed(), "{name}: {again:?}");
        assert_eq!(fingerprint(&snap(&s)), before, "{name}");
    }
}
