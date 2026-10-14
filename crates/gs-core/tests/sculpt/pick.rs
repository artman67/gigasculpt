//! Ray casts and nearest-vertex searches against brute force in `f64` over the plain mesh, the
//! same answers to the bit as testing every face or vertex in `f32` (in small nodes and in
//! large ones of many runs, through a refining stroke and its undo), and the work counters
//! that keep them to the nodes and runs near the ray or point.

use gs_core::live::{LiveDetail, Target};
use gs_core::pick::triangle_hit;
use gs_core::{
    BrushPreset, ColumnSet, Hit, MeshStore, NO, NodeConfig, NodeId, PosRead, Stroke, StrokeParams,
    StrokeSample, TopoRead, Vec3, View,
};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh, face_triangles};
use gs_testkit::SplitMix64;
use gs_testkit::fixtures::{cube, cylinder, icosphere, patch, torus};
use gs_testkit::measure::{P, add, cross, dot, norm, scale, sub};

use crate::{ok, point, store, wide};

/// The nearest hit at or past `o` of the unit ray `d` on triangle `abc`, from either side.
fn triangle(o: P, d: P, [a, b, c]: [P; 3]) -> Option<f64> {
    let (e1, e2) = (sub(b, a), sub(c, a));
    let n = cross(e1, e2);
    let along = dot(n, d);
    if along.abs() < 1e-12 * norm(n) {
        return None;
    }
    let t = dot(n, sub(a, o)) / along;
    let x = add(o, scale(d, t));
    // Inside when the point is on the inner side of every edge.
    let inner = |p: P, q: P| dot(cross(sub(q, p), sub(x, p)), n) >= -1e-9 * dot(n, n);
    (t >= 0.0 && inner(a, b) && inner(b, c) && inner(c, a)).then_some(t)
}

/// The triangles a face is drawn as, with vertex `v` at `pos(v)`.
fn triangles(f: [u32; 4], pos: impl Fn(u32) -> [f32; 3]) -> Vec<[u32; 3]> {
    (face_triangles(f, |k| pos(f[k])).iter())
        .map(|t| t.map(|k| f[k]))
        .collect()
}

/// The nearest hit over every face that is not hidden.
fn brute_cast(m: &PlainMesh, hidden: &[bool], o: P, d: P) -> Option<f64> {
    let at = |v: u32| m.positions[v as usize].map(f64::from);
    (m.faces.iter().zip(hidden))
        .filter(|(_, h)| !**h)
        .flat_map(|(f, _)| triangles(*f, |v| m.positions[v as usize]))
        .filter_map(|t| triangle(o, d, t.map(at)))
        .min_by(f64::total_cmp)
}

/// The fixtures with one face in seven hidden.
fn scenes() -> Vec<PlainMesh> {
    [icosphere(5), torus(), cube(12), patch(24), cylinder()]
        .into_iter()
        .map(|mut m| {
            let values = (0..m.faces.len()).map(|f| u32::from(f % 7 == 3)).collect();
            let hide = Attribute {
                domain: AttrDomain::Face,
                values: AttrValues::U32(values),
            };
            m.attributes.insert("hide".into(), hide);
            m
        })
        .collect()
}

/// The store and its mesh as stored: a quad splits along the diagonal from its first corner,
/// and the store turns each face to start at its lowest vertex, so the reference reads faces
/// the way the store keeps (and the renderer draws) them.
fn stored(m: &PlainMesh) -> (MeshStore, PlainMesh, Vec<bool>) {
    let s = store(m);
    let m = s.to_plain();
    let hidden = match m.attributes.get("hide").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&h| h != 0).collect(),
        _ => vec![false; m.faces.len()],
    };
    (s, m, hidden)
}

/// A ray from outside the unit-sized fixtures (or from inside, one time in four) toward a
/// point near them.
fn ray(rng: &mut SplitMix64) -> (Vec3, Vec3) {
    let toward = point(rng, 1.2);
    let from = if rng.below(4) == 0 {
        point(rng, 0.3)
    } else {
        point(rng, 1.0).normalized() * 4.0
    };
    (from, toward - from)
}

#[test]
fn ray_casts_find_the_nearest_visible_face() {
    let mut rng = SplitMix64::new(21);
    let (mut hits, mut misses) = (0, 0);
    for m in scenes() {
        let (s, m, hidden) = stored(&m);
        for _ in 0..150 {
            let (o, dir) = ray(&mut rng);
            let d = dir.normalized();
            let want = brute_cast(&m, &hidden, wide(o), wide(d));
            let cast = s.ray_cast(o, dir);
            match (want, cast.hit) {
                (None, None) => misses += 1,
                (Some(t), Some(hit)) => {
                    hits += 1;
                    assert!(cast.faces_tested > 0, "a hit counts the faces it tested");
                    assert!(
                        (f64::from(hit.t) - t).abs() < 1e-4,
                        "ray {o:?} {d:?} on {} faces: hit at {} (face {:?}) not {t}",
                        m.faces.len(),
                        hit.t,
                        s.topo().face(hit.face)
                    );
                    let p = add(wide(o), scale(wide(d), t));
                    assert!(
                        norm(sub(wide(hit.pos), p)) < 1e-4,
                        "the hit point is on the ray"
                    );
                    on_face(&s, hit.face, wide(hit.pos), hit.normal);
                }
                (want, got) => panic!("ray {o:?} {d:?}: brute force {want:?}, cast {got:?}"),
            }
        }
    }
    assert!(hits > 300 && misses > 50, "{hits} hits and {misses} misses");
}

/// The face the store reports holds the hit point, and the normal is the unit normal of one of
/// its triangles.
fn on_face(s: &MeshStore, face: u32, x: P, normal: Vec3) {
    let f = s.topo().face(face);
    let pos = s.pos();
    let at = |v: u32| wide(pos.get(v));
    let holds = (triangles(f, |v| pos.get(v).to_array()).into_iter()).any(|t| {
        let [a, b, c] = t.map(at);
        let n = cross(sub(b, a), sub(c, a));
        let unit = norm(sub(scale(n, 1.0 / norm(n)), wide(normal))) < 1e-4;
        let plane = (dot(n, sub(x, a)) / norm(n)).abs() < 1e-4;
        unit && plane
    });
    assert!(
        holds,
        "face {face} {f:?} does not hold the hit or its normal"
    );
}

#[test]
fn ray_casts_stay_near_the_ray() {
    let mut rng = SplitMix64::new(22);
    let m = icosphere(5);
    let s = store(&m);
    let (mut tested, mut casts) = (0, 0);
    for _ in 0..100 {
        let (o, dir) = ray(&mut rng);
        tested += s.ray_cast(o, dir).faces_tested;
        casts += 1;
    }
    // Nearest node first, stopping past the hit: about half what searching every node along
    // the ray would test.
    let mean = tested / casts;
    assert!(
        mean * 30 < m.faces.len(),
        "a cast tests {mean} of {} faces on average",
        m.faces.len()
    );
    // Rays along an axis that pass beside the mesh's box, or start past it, test nothing.
    let beside = [
        (Vec3::new(-5.0, 5.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        (Vec3::new(0.0, -5.0, 1.5), Vec3::new(0.0, 1.0, 0.0)),
        (Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0)),
    ];
    for (o, d) in beside {
        let away = s.ray_cast(o, d);
        assert_eq!(
            (away.hit, away.faces_tested),
            (None, 0),
            "{o:?} along {d:?}"
        );
    }
}

/// Every face of the store, with its id.
fn faces(s: &MeshStore) -> Vec<(u32, [u32; 4])> {
    (0..s.node_count() as u32)
        .flat_map(|n| {
            let node = s.node(n);
            (node.first_face..).zip(node.faces.iter().copied())
        })
        .filter(|f| f.1[0] != NO)
        .collect()
}

/// A flat grid of `n` by `n` unit quads at z = 0 with whole-number corners, so rays straight
/// down meet it with exact arithmetic.
fn grid(n: u32) -> PlainMesh {
    let row = n + 1;
    let positions = (0..row * row)
        .map(|k| [(k % row) as f32, (k / row) as f32, 0.0])
        .collect();
    let faces = (0..n * n)
        .map(|k| {
            let (i, j) = (k % n, k / n);
            let v = |i: u32, j: u32| j * row + i;
            [v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1)]
        })
        .collect();
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

#[test]
fn ties_go_to_the_lowest_id() {
    let s = store(&grid(24));
    let pos = s.pos();
    let at = |v: u32| wide(pos.get(v));
    // Straight down through grid points and edge middles: several faces at exactly one
    // distance; the lowest face id wins.
    let mut ties = 0;
    for k in 0..48 {
        let (x, y) = (
            f32::from(k as u8 % 12) * 2.0,
            f32::from(k as u8 / 4) * 0.5 + 1.0,
        );
        let (o, d) = (Vec3::new(x, y, 4.0), Vec3::new(0.0, 0.0, -1.0));
        let Some(hit) = s.ray_cast(o, d).hit else {
            panic!("the ray from {o:?} hits the grid");
        };
        let meets = |f: [u32; 4]| {
            (triangles(f, |v| pos.get(v).to_array()).into_iter())
                .any(|t| triangle(wide(o), wide(d), t.map(at)).is_some())
        };
        let level: Vec<u32> = (faces(&s).into_iter())
            .filter(|&(_, f)| meets(f))
            .map(|(id, _)| id)
            .collect();
        ties += usize::from(level.len() > 1);
        assert_eq!(
            Some(hit.face),
            level.iter().copied().min(),
            "{o:?}: {level:?}"
        );
    }
    assert!(ties > 30, "only {ties} rays met faces at the same distance");
    // A point halfway along an edge is exactly as near to both ends: the lower id wins.
    for (_, f) in faces(&s) {
        let (a, b) = (f[0], f[1]);
        let mid = (pos.get(a) + pos.get(b)) * 0.5 + Vec3::new(0.0, 0.0, 0.25);
        let found = s.nearest_vertex(mid).found.map(|n| n.0);
        assert_eq!(found, Some(a.min(b)), "the edge {a}-{b}");
    }
}

#[test]
fn nearest_vertex_matches_brute_force() {
    let mut rng = SplitMix64::new(23);
    for m in scenes() {
        let s = store(&m);
        let (mut tested, mut searches) = (0, 0);
        for k in 0..100 {
            let p = point(&mut rng, if k % 3 == 0 { 3.0 } else { 1.1 });
            let want = (m.positions.iter())
                .map(|&q| norm(sub(q.map(f64::from), wide(p))))
                .fold(f64::INFINITY, f64::min);
            let near = s.nearest_vertex(p);
            let Some((v, dist)) = near.found else {
                panic!("no vertex found near {p:?}");
            };
            assert!((f64::from(dist) - want).abs() < 1e-5, "{dist} not {want}");
            let at = norm(sub(wide(s.pos().get(v)), wide(p)));
            assert!(
                (at - want).abs() < 1e-5,
                "vertex {v} is {at} away, not {want}"
            );
            assert!(near.tested > 0, "a search counts the vertices it measured");
            tested += near.tested;
            searches += 1;
        }
        // Only a mesh of many nodes leaves room to skip most of them.
        let mean = tested / searches;
        assert!(
            m.positions.len() < 5000 || mean * 8 < m.positions.len(),
            "a search measures {mean} of {} vertices on average",
            m.positions.len()
        );
    }
}

#[test]
fn nearest_to_a_ray_matches_brute_force() {
    let mut rng = SplitMix64::new(24);
    for m in scenes() {
        let s = store(&m);
        let mut tested = 0;
        for _ in 0..60 {
            let (o, dir) = ray(&mut rng);
            let d = wide(dir.normalized());
            let gap = |q: P| {
                let w = sub(q, wide(o));
                norm(sub(w, scale(d, dot(w, d).max(0.0))))
            };
            let want = (m.positions.iter())
                .map(|&q| gap(q.map(f64::from)))
                .fold(f64::INFINITY, f64::min);
            let near = s.nearest_to_ray(o, dir);
            tested += near.tested;
            let Some((v, dist)) = near.found else {
                panic!("no vertex found near the ray");
            };
            assert!((f64::from(dist) - want).abs() < 1e-5, "{dist} not {want}");
            assert!(
                (gap(wide(s.pos().get(v))) - want).abs() < 1e-5,
                "vertex {v}"
            );
        }
        // A ray through a mesh of many nodes measures only the nodes it passes near.
        let mean = tested / 60;
        assert!(
            m.positions.len() < 5000 || mean * 6 < m.positions.len(),
            "a search measures {mean} of {} vertices on average",
            m.positions.len()
        );
    }
}

#[test]
fn rays_without_a_direction_or_a_finite_origin_test_nothing() {
    let s = store(&icosphere(4));
    let down = Vec3::new(0.0, 0.0, -1.0);
    for (o, d) in [
        (s.pos().get(0), Vec3::ZERO),
        (Vec3::new(0.0, 0.0, f32::NAN), down),
        (Vec3::new(0.0, 0.0, f32::INFINITY), down),
    ] {
        let cast = s.ray_cast(o, d);
        assert_eq!(
            (cast.hit, cast.faces_tested),
            (None, 0),
            "{o:?} along {d:?}"
        );
    }
}

#[test]
fn flat_meshes_are_hit_along_every_axis() {
    for axis in 0..3 {
        // The grid at z = 0, turned to lie across `axis`.
        let turn = |mut p: [f32; 3]| {
            p.rotate_right((axis + 1) % 3);
            p
        };
        let mut m = grid(8);
        for p in &mut m.positions {
            *p = turn(*p);
        }
        let s = store(&m);
        let [o, d] = [[2.5, 3.5, 4.0], [0.0, 0.0, -1.0]].map(|p| Vec3::from_array(turn(p)));
        let hit = s.ray_cast(o, d).hit;
        assert!(
            hit.is_some_and(|h| (h.t - 4.0).abs() < 1e-6),
            "across axis {axis}: {hit:?}"
        );
    }
}

#[test]
fn a_bent_quad_is_hit_on_its_nearer_triangle() {
    // Folded along the diagonal from corner 0 to corner 2 with both wings raised: a level ray
    // across the fold meets both triangles, a b c (normal -1 1 1) and a c d (normal 1 -1 1).
    let m = PlainMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
        ],
        faces: vec![[0, 1, 2, 3]],
        ..PlainMesh::default()
    };
    let s = store(&m);
    let third = 3f32.sqrt().recip();
    let near = 0.75 * 2f32.sqrt();
    for (o, d, normal) in [
        ([1.5, -0.5, 0.5], [-1.0, 1.0, 0.0], [-third, third, third]),
        ([-0.5, 1.5, 0.5], [1.0, -1.0, 0.0], [third, -third, third]),
    ] {
        let hit = s.ray_cast(Vec3::from_array(o), Vec3::from_array(d)).hit;
        let Some(hit) = hit else {
            panic!("the ray from {o:?} hits the quad");
        };
        assert!((hit.t - near).abs() < 1e-5, "from {o:?}: hit at {}", hit.t);
        assert!(
            (hit.normal - Vec3::from_array(normal)).len2() < 1e-10,
            "from {o:?}: the nearer triangle's normal, not {:?}",
            hit.normal
        );
    }
}

/// The grid with a wall a unit high standing on its edge x = `n` (a strip of its own
/// vertices): a ray straight down along the wall never meets it, but the wall makes the boxes
/// of the nodes near it taller, so the ray enters them before the flat nodes beside them.
fn walled(n: u32) -> PlainMesh {
    let mut m = grid(n);
    let first = m.positions.len() as u32;
    for y in 0..=n {
        m.positions.push([n as f32, y as f32, 0.0]);
        m.positions.push([n as f32, y as f32, 1.0]);
    }
    let (b, t) = (|y: u32| first + 2 * y, |y: u32| first + 2 * y + 1);
    m.faces
        .extend((0..n).map(|y| [b(y), b(y + 1), t(y + 1), t(y)]));
    m
}

#[test]
fn ties_in_nodes_entered_later_still_go_to_the_lowest_id() {
    let s = store(&walled(24));
    let pos = s.pos();
    let at = |v: u32| wide(pos.get(v));
    // Whether the ray enters the node holding `face` above the grid (the node tree's boxes are
    // the halos).
    let tall = |face: u32| s.halo(s.config().face_node(face)).hi.z > 0.0;
    let mut later = 0;
    for x in 0..=24u8 {
        for y in 0..=24u8 {
            let (o, d) = (
                Vec3::new(f32::from(x), f32::from(y), 4.0),
                Vec3::new(0.0, 0.0, -1.0),
            );
            let meets = |f: [u32; 4]| {
                (triangles(f, |v| pos.get(v).to_array()).into_iter())
                    .any(|t| triangle(wide(o), wide(d), t.map(at)).is_some())
            };
            let level: Vec<u32> = (faces(&s).into_iter())
                .filter(|&(_, f)| meets(f))
                .map(|(id, _)| id)
                .collect();
            let lowest = level.iter().copied().min();
            assert_eq!(s.ray_cast(o, d).hit.map(|h| h.face), lowest, "{o:?}");
            // The lowest id sits in a node the ray enters after a taller one that also holds a
            // face at the same distance.
            let flat_lowest = lowest.is_some_and(|f| !tall(f));
            later += usize::from(flat_lowest && level.iter().any(|&f| tall(f)));
        }
    }
    assert!(later > 0, "no tie spans nodes entered at different depths");
}

/// Nodes of up to 4096 slots, so a node of a test mesh holds dozens of runs.
fn big(m: &PlainMesh) -> MeshStore {
    ok(MeshStore::from_plain(m, ok(NodeConfig::new(12, 2048))))
}

/// The cast done by testing every visible face in id order, each with the cast's own triangle
/// test: the nearest hit, the first triangle of a quad on a tie inside it, the lowest id on a
/// tie between faces.
fn every_face(s: &MeshStore, o: Vec3, dir: Vec3) -> Option<Hit> {
    let d = dir.normalized();
    let (pos, hide) = (s.pos(), s.attr_id("hide"));
    let mut best: Option<Hit> = None;
    for n in 0..s.node_count() as NodeId {
        let node = s.node(n);
        let hidden = hide.and_then(|h| node.attr(h));
        for (j, f) in node.faces.iter().enumerate() {
            if f[0] == NO || hidden.is_some_and(|h| h.bits(j) != 0) {
                continue;
            }
            let at = |k: usize| pos.get(f[k]);
            let tris = face_triangles(*f, |k| at(k).to_array());
            let first = triangle_hit(o, d, tris[0].map(at));
            let second = (tris.get(1)).and_then(|t| triangle_hit(o, d, t.map(at)));
            let hit = match (first, second) {
                (Some(a), Some(b)) => Some(if b.0.total_cmp(&a.0).is_lt() { b } else { a }),
                (a, b) => a.or(b),
            };
            let Some((t, normal)) = hit else {
                continue;
            };
            if best.is_none_or(|b| t.total_cmp(&b.t).is_lt()) {
                let face = node.first_face + j as u32;
                let pos = o + d * t;
                best = Some(Hit {
                    t,
                    pos,
                    face,
                    normal,
                });
            }
        }
    }
    best
}

/// The squared length of `v`, summed in `f64` in the order a search sums it.
fn len2(v: Vec3) -> f64 {
    let [x, y, z] = v.to_array().map(f64::from);
    x * x + y * y + z * z
}

/// The search done by measuring every vertex slot in id order: the least `dist2` of a live
/// vertex, the lowest id on a tie, and its distance.
fn every_vertex(s: &MeshStore, dist2: impl Fn(Vec3) -> f64) -> Option<(u32, f32)> {
    let mut best: Option<(f64, u32)> = None;
    for n in 0..s.node_count() as NodeId {
        let node = s.node(n);
        for (v, &q) in (node.base..).zip(node.pos) {
            let d2 = dist2(q);
            if !d2.is_nan() && best.is_none_or(|b| d2.total_cmp(&b.0).is_lt()) {
                best = Some((d2, v));
            }
        }
    }
    best.map(|(d2, v)| (v, d2.sqrt() as f32))
}

/// Each ray's cast, the vertex nearest it and the vertex nearest the point a unit along it are
/// exactly what testing everything gives; returns how many rays hit.
fn exact(s: &MeshStore, rays: &[(Vec3, Vec3)], what: &str) -> usize {
    let mut hits = 0;
    for &(o, dir) in rays {
        let cast = s.ray_cast(o, dir).hit;
        assert_eq!(
            cast,
            every_face(s, o, dir),
            "{what}: the cast {o:?} {dir:?}"
        );
        hits += usize::from(cast.is_some());
        let d = dir.normalized();
        let to_ray = every_vertex(s, |q| {
            let w = q - o;
            len2(w - d * w.dot(d).max(0.0))
        });
        let near = s.nearest_to_ray(o, dir).found;
        assert_eq!(near, to_ray, "{what}: nearest the ray {o:?} {dir:?}");
        let p = o + d;
        let to_point = every_vertex(s, |q| len2(q - p));
        let near = s.nearest_vertex(p).found;
        assert_eq!(near, to_point, "{what}: nearest {p:?}");
    }
    hits
}

/// Straight down onto the grids, through every corner, edge middle and cell middle of their
/// first 12 by 12 cells: rays that meet several faces, triangles and vertices at one distance.
fn down() -> Vec<(Vec3, Vec3)> {
    (0..=24u8)
        .flat_map(|i| (0..=24u8).map(move |j| (f32::from(i) * 0.5, f32::from(j) * 0.5)))
        .map(|(x, y)| (Vec3::new(x, y, 4.0), Vec3::new(0.0, 0.0, -1.0)))
        .collect()
}

#[test]
fn searches_give_what_testing_everything_gives() {
    let mut rng = SplitMix64::new(25);
    let mut hits = 0;
    for m in scenes() {
        let rays: Vec<_> = (0..60).map(|_| ray(&mut rng)).collect();
        for s in [store(&m), big(&m)] {
            hits += exact(&s, &rays, "fixture");
        }
    }
    assert!(hits > 300, "only {hits} rays hit");
    let down = down();
    for m in [grid(24), walled(24)] {
        for s in [store(&m), big(&m)] {
            assert_eq!(
                exact(&s, &down, "grid"),
                down.len(),
                "every ray hits the grid"
            );
        }
    }
}

const DRAW: &str = include_str!("../../../../assets/brushes/draw.toml");

#[test]
fn searches_stay_exact_through_a_refining_stroke_and_its_undo() {
    let mut s = big(&icosphere(3));
    let mut rng = SplitMix64::new(26);
    let mut rays: Vec<_> = (0..40).map(|_| ray(&mut rng)).collect();
    // And straight down onto the stroke's path, where the surface changes.
    rays.extend((0..9).map(|k| {
        let x = -0.4 + 0.1 * k as f32;
        (Vec3::new(x, 0.05, 4.0), Vec3::new(0.0, 0.0, -1.0))
    }));
    // Every search has built its runs before the stroke, so a run left stale would show.
    exact(&s, &rays, "before");
    let faces = s.face_count();
    let mut prm = StrokeParams::new(View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: None,
    });
    prm.live_detail = Some(LiveDetail {
        target: Target::Constant(0.03),
        ..LiveDetail::default()
    });
    let mut k = ok(Stroke::begin(
        &mut s,
        &ok(BrushPreset::from_toml(DRAW)),
        prm,
    ));
    for i in 0..7 {
        let x = -0.3 + 0.1 * i as f32;
        let pos = Vec3::new(x, 0.0, (1.0 - x * x).sqrt());
        let sample = StrokeSample {
            pos,
            pressure: 1.0,
            tilt: [0.0; 2],
            radius: 0.3,
        };
        ok(k.add(&mut s, &[sample]));
        exact(&s, &rays, &format!("after sample {i}"));
    }
    k.finish(&mut s);
    assert!(
        s.face_count() > faces + 100,
        "the stroke refined the surface: {faces} faces, then {}",
        s.face_count()
    );
    assert!(s.undo().is_some());
    assert_eq!(s.face_count(), faces);
    exact(&s, &rays, "undone");
    assert!(s.redo().is_some());
    exact(&s, &rays, "redone");
    // A move through a writer.
    let nodes: Vec<NodeId> = (0..s.node_count() as NodeId).collect();
    s.write(&nodes, ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            *p += Vec3::new(0.0, 0.0, 0.25);
        }
    });
    exact(&s, &rays, "moved");
}

#[test]
fn searches_in_large_nodes_test_only_the_runs_near_them() {
    let mut rng = SplitMix64::new(27);
    let s = big(&icosphere(5));
    let (mut tested, mut measured) = (0, 0);
    for _ in 0..100 {
        let (o, dir) = ray(&mut rng);
        tested += s.ray_cast(o, dir).faces_tested;
        measured += s
            .nearest_vertex(point(&mut rng, 1.0).normalized() * 1.02)
            .tested;
    }
    let (tested, measured) = (tested / 100, measured / 100);
    // Five nodes of about 4000 faces and 2048 vertex slots: a search that reached a node and
    // tested all of it would test that many.
    let (faces, slots) = (0..s.node_count() as NodeId)
        .map(|n| (s.node(n).faces.len(), s.node(n).pos.len()))
        .filter(|x| x.0 > 0)
        .fold((usize::MAX, usize::MAX), |a, b| {
            (a.0.min(b.0), a.1.min(b.1))
        });
    assert!(
        tested * 6 < faces,
        "a cast tests {tested} faces, against {faces} in a node"
    );
    assert!(
        measured * 3 < slots,
        "a search measures {measured} vertices, against {slots} in a node"
    );
}
