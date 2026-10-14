//! Unfreezing onto a new base (decision 0030, sections 3 and 4): the face count rule, and the
//! projection search (decision 0037) against a slow f64 hybrid search over every old triangle.

use gs_core::projection::{Projection, Side, Surface, project};
use gs_core::{ColumnSet, Levels, MeshStore, NO, NodeId, PosRead, Settings, Vec3};
use gs_protocol::{PlainMesh, face_triangles};
use gs_testkit::fixtures;
use gs_testkit::measure::{self, add, cross, dot, norm, scale, sub};

use super::{bits, stack, top_positions};
use crate::common::{self, ok};

/// Moves every point of the active store by `f`.
fn shift(l: &mut Levels, f: impl Fn(Vec3) -> Vec3 + Sync) {
    let s = l.store_mut();
    let all: Vec<NodeId> = (0..s.node_count() as NodeId).collect();
    s.write(&all, ColumnSet::POSITION).each(|node| {
        for p in node.positions() {
            *p = f(*p);
        }
    });
}

/// A cube divided `times` with radial bumps on its top: the record.
fn record(times: usize) -> Levels {
    let mut l = stack(&fixtures::cube(1), &Settings::default());
    for _ in 0..times {
        ok(l.divide(true));
    }
    shift(&mut l, |p| {
        let wave = libm::sinf(7.0 * p.x) * libm::sinf(5.0 * p.y) * libm::cosf(3.0 * p.z);
        let bump = 1.0 + 0.012 * wave;
        p * bump
    });
    l
}

/// The record's level `j` (from 0) as a stack of one level, every point nudged: a base whose
/// topology is another than the record's base, standing in for a remesh.
fn new_base(record: &Levels, j: usize) -> Levels {
    let mut b = record.share();
    ok(b.set(j));
    b.delete_higher();
    b.delete_lower();
    shift(&mut b, |p| {
        p + Vec3::new(0.003, -0.002, 0.004) * libm::sinf(11.0 * p.x)
    });
    b
}

fn to64(p: Vec3) -> [f64; 3] {
    p.to_array().map(f64::from)
}

/// The top level as a plain mesh, in canonical order.
fn surface(l: &Levels) -> PlainMesh {
    let num = l.level_numbering(l.top_level());
    PlainMesh {
        positions: top_positions(l).into_iter().map(Vec3::to_array).collect(),
        faces: num.canonical_faces(l.top()),
        ..PlainMesh::default()
    }
}

/// The length of the diagonal of the points' bounding box.
fn diagonal(p: &[Vec3]) -> f64 {
    let p: Vec<[f32; 3]> = p.iter().map(|q| q.to_array()).collect();
    let [lo, hi] = measure::bounds(&p).unwrap_or_default();
    norm(sub(hi.map(f64::from), lo.map(f64::from)))
}

/// Distance from `p` to the closest point of `surface`: gs-testkit's one-sided Hausdorff
/// distance from a mesh of `p` alone.
fn to_surface(p: Vec3, surface: &PlainMesh) -> f64 {
    let point = PlainMesh {
        positions: vec![p.to_array()],
        ..PlainMesh::default()
    };
    measure::hausdorff_from(&point, surface)
}

/// The surface's triangles in f64, quads split as the search splits them (as drawn).
fn triangles64(store: &MeshStore) -> Vec<[[f64; 3]; 3]> {
    let pos = store.pos();
    let mut out = Vec::new();
    for n in 0..store.node_count() as NodeId {
        for &f in store.node(n).faces.iter().filter(|f| f[0] != NO) {
            let p = |k: usize| pos.get(f[k]);
            let tris = face_triangles(f, |k| p(k).to_array());
            out.extend(tris.iter().map(|t| t.map(|k| to64(p(k)))));
        }
    }
    out
}

/// Where the line through `p` along unit `n` crosses `t`, as an offset along `n`, counting
/// a line that passes within `slack` (in weights) outside an edge.
fn crossing64(p: [f64; 3], n: [f64; 3], [a, b, c]: &[[f64; 3]; 3], slack: f64) -> Option<f64> {
    let (e1, e2) = (sub(*b, *a), sub(*c, *a));
    let pv = cross(n, e2);
    let det = dot(e1, pv);
    if det.abs() < 1e-300 {
        return None;
    }
    let tv = sub(p, *a);
    let u = dot(tv, pv) / det;
    let qv = cross(tv, e1);
    let v = dot(n, qv) / det;
    (u >= -slack && v >= -slack && u + v <= 1.0 + slack).then(|| dot(e2, qv) / det)
}

/// The closest point of `t` to `p`, by projecting onto the plane and, outside, onto the edges.
fn closest64(p: [f64; 3], [a, b, c]: &[[f64; 3]; 3]) -> [f64; 3] {
    let on_edge = |x: [f64; 3], y: [f64; 3]| {
        let d = sub(y, x);
        let t = (dot(sub(p, x), d) / dot(d, d).max(1e-300)).clamp(0.0, 1.0);
        add(x, scale(d, t))
    };
    let normal = cross(sub(*b, *a), sub(*c, *a));
    let nn = dot(normal, normal);
    if nn > 1e-300 {
        let q = sub(p, scale(normal, dot(sub(p, *a), normal) / nn));
        let inside = [(*a, *b), (*b, *c), (*c, *a)]
            .iter()
            .all(|&(x, y)| dot(cross(sub(y, x), sub(q, x)), normal) >= 0.0);
        if inside {
            return q;
        }
    }
    [on_edge(*a, *b), on_edge(*b, *c), on_edge(*c, *a)]
        .into_iter()
        .min_by(|x, y| norm(sub(*x, p)).total_cmp(&norm(sub(*y, p))))
        .unwrap_or(*a)
}

/// The hybrid rule over every triangle, in f64: the nearest ray hit both ways if it lies no
/// farther than twice the closest point's distance plus two `edge`s (`give` added to that
/// bound), else the closest point, within `reach` and on `side`.
fn hybrid64(
    tris: &[[[f64; 3]; 3]],
    (p, n): ([f64; 3], [f64; 3]),
    (reach, side, edge): (f64, Side, f64),
    (slack, give): (f64, f64),
) -> Option<[f64; 3]> {
    let allows = |s: f64| match side {
        Side::Both => true,
        Side::Outside => s >= 0.0,
        Side::Inside => s <= 0.0,
    };
    let q = (tris.iter().map(|t| closest64(p, t)))
        .min_by(|x, y| norm(sub(*x, p)).total_cmp(&norm(sub(*y, p))))?;
    let close = norm(sub(q, p));
    let len = norm(n);
    if len > 0.0 {
        let n = scale(n, 1.0 / len);
        let hit = (tris.iter().filter_map(|t| crossing64(p, n, t, slack)))
            .filter(|s| s.abs() <= reach && allows(*s))
            .min_by(|x, y| x.abs().total_cmp(&y.abs()));
        if let Some(s) = hit.filter(|s| s.abs() <= 2.0 * close + 2.0 * edge + give) {
            return Some(add(p, scale(n, s)));
        }
    }
    (close <= reach && allows(dot(sub(q, p), n))).then_some(q)
}

/// Whether `found` lands where the reference does, read with no slack or with a little more
/// than the search's own (a ray grazing a vertex may count as a hit or a miss in either), and
/// with the guard's bound as it is or `tol` either way (a hit on the bound may be kept or not).
fn agrees(
    tris: &[[[f64; 3]; 3]],
    (p, n): (Vec3, Vec3),
    how: (f64, Side, f64),
    found: Option<Vec3>,
    tol: f64,
) -> bool {
    let reads = [0.0, 4e-6]
        .into_iter()
        .flat_map(|s| [(s, 0.0), (s, -tol), (s, tol)]);
    reads.into_iter().any(
        |read| match (found, hybrid64(tris, (to64(p), to64(n)), how, read)) {
            (None, None) => true,
            (Some(f), Some(w)) => norm(sub(to64(f), w)) <= tol,
            _ => false,
        },
    )
}

/// The mean length of the edges of `store`'s live faces, each face's own counted, in f64: the
/// moving surface's edge the guard reads.
fn mean_edge64(store: &MeshStore) -> f64 {
    let pos = store.pos();
    let (mut sum, mut count) = (0.0, 0usize);
    for n in 0..store.node_count() as NodeId {
        for f in store.node(n).faces.iter().filter(|f| f[0] != NO) {
            let k = if f[3] == NO { 3 } else { 4 };
            for i in 0..k {
                sum += norm(sub(to64(pos.get(f[(i + 1) % k])), to64(pos.get(f[i]))));
                count += 1;
            }
        }
    }
    sum / count as f64
}

/// Every live point of `store` with its normal, in node order; every third normal is tipped
/// most of the way into the surface, so its rays graze and miss and the fallback is tried.
fn points_and_normals(store: &MeshStore) -> Vec<(Vec3, Vec3)> {
    let mut out = Vec::new();
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        for (&p, &d) in node.pos.iter().zip(node.nrm).filter(|(p, _)| !p.is_dead()) {
            let tipped = (out.len() % 3 == 0).then(|| d + d.cross(Vec3::new(0.6, 0.8, 0.0)) * 8.0);
            out.push((p, tipped.unwrap_or(d)));
        }
    }
    out
}

#[test]
fn each_point_lands_where_a_slow_search_over_every_triangle_says() {
    let rec = record(3);
    let tris = triangles64(rec.top());
    let surface = Surface::of(rec.top(), false);
    let diag = diagonal(&top_positions(&rec));
    // Points of a nudged level two, pushed off the surface both ways.
    let base = new_base(&rec, 2);
    let mut points = points_and_normals(base.top());
    let pushed: Vec<(Vec3, Vec3)> = (points.iter())
        .enumerate()
        .map(|(i, &(p, n))| (p + n.normalized() * (0.01 * (i % 5) as f32 - 0.02), n))
        .collect();
    points.extend(pushed);
    let (mut rays, mut fallbacks, mut misses, mut guarded) = (0, 0, 0, 0);
    for side in [Side::Both, Side::Outside, Side::Inside] {
        for reach in [0.004 * diag, 0.02 * diag] {
            for (i, &(p, n)) in points.iter().enumerate() {
                // No edge, so the guard is twice the closest point's distance alone, or one of
                // 0.001 of the diagonal.
                let edge = (i % 2) as f64 * 0.001 * diag;
                let found = surface.find(p, n, reach as f32, side, edge as f32);
                let ok = agrees(
                    &tris,
                    (p, n),
                    (reach, side, edge),
                    found.map(|f| f.point),
                    1e-5 * diag,
                );
                assert!(ok, "{side:?} {p:?} {n:?} reach {reach}: found {found:?}");
                let unguarded = (to64(p), to64(n));
                let far = (reach, side, f64::INFINITY);
                let ray = hybrid64(&tris, unguarded, far, (0.0, 0.0));
                let turned = found.is_some_and(|f| !f.ray)
                    && ray.is_some_and(|w| norm(sub(w, to64(found.unwrap().point))) > 1e-5 * diag);
                guarded += usize::from(turned);
                match found {
                    None => misses += 1,
                    Some(f) if f.ray => rays += 1,
                    Some(_) => fallbacks += 1,
                }
            }
        }
    }
    // Each way is tried often: about 570 rays, 190 fallbacks (110 of them ray hits the guard
    // turned back) and 410 misses.
    assert!(
        rays > 400 && fallbacks > 100 && misses > 200 && guarded > 50,
        "{rays} {fallbacks} {misses} {guarded}"
    );
}

/// A floor of 0.1 squares over x and y from 0 to 1 at z 0, with a thin fin standing on it as
/// an ear stands on a head: its sides at x 0.3 and 0.32, its top at z 0.4.
fn floor_and_fin() -> PlainMesh {
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    let mut quad = |corners: [[f32; 3]; 4]| {
        let first = positions.len() as u32;
        positions.extend(corners);
        faces.push([first, first + 1, first + 2, first + 3]);
    };
    for i in 0..10 {
        for j in 0..10 {
            let (x, y) = (i as f32 * 0.1, j as f32 * 0.1);
            quad([
                [x, y, 0.0],
                [x + 0.1, y, 0.0],
                [x + 0.1, y + 0.1, 0.0],
                [x, y + 0.1, 0.0],
            ]);
        }
    }
    for j in 0..10 {
        let y = j as f32 * 0.1;
        let (a, b) = (y, y + 0.1);
        quad([[0.3, a, 0.0], [0.3, a, 0.4], [0.3, b, 0.4], [0.3, b, 0.0]]);
        quad([
            [0.32, a, 0.0],
            [0.32, b, 0.0],
            [0.32, b, 0.4],
            [0.32, a, 0.4],
        ]);
        quad([[0.3, a, 0.4], [0.32, a, 0.4], [0.32, b, 0.4], [0.3, b, 0.4]]);
    }
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

/// Decision 0037, section 1, as amended for D2b: a small triangle standing nearly upright just
/// above the floor, its normal 84 degrees from the floor's, as on a smooth surface bridging the
/// crease at an ear's root. Each point's ray along its normal reaches the fin about a quarter
/// unit off, and against it runs off the floor's edge, while the floor is 0.01 to 0.03 below.
/// The bound: no point lands farther from where it was than twice its closest-point distance
/// plus two of the moving surface's edges (here at most 0.028 long); so these land on the floor.
#[test]
fn a_ray_that_grazes_past_the_floor_to_a_fin_does_not_carry_its_point_there() {
    let source = common::store(&floor_and_fin(), common::small_nodes());
    let tris = triangles64(&source);
    let surface = Surface::of(&source, false);
    let n = Vec3::new(1.0, 0.0, 0.1).normalized();
    let up = Vec3::new(-0.1, 0.0, 1.0).normalized();
    let p = Vec3::new(0.05, 0.5, 0.01);
    let corners = [p, p + Vec3::new(0.0, 0.02, 0.0), p + up * 0.02];
    let triangle = PlainMesh {
        positions: corners.map(Vec3::to_array).to_vec(),
        faces: vec![[0, 1, 2, NO]],
        ..PlainMesh::default()
    };
    let mut moving = common::store(&triangle, common::small_nodes());
    let before: Vec<(Vec3, Vec3)> = points_and_normals_untipped(&moving);
    assert!(
        before
            .iter()
            .all(|(_, d)| (d.normalized() - n).len2() < 1e-10)
    );
    let longest = 0.02 * std::f64::consts::SQRT_2;
    project(&mut moving, &Projection::plain(&surface, 0.3));
    let after: Vec<Vec3> = points_and_normals_untipped(&moving)
        .into_iter()
        .map(|x| x.0)
        .collect();
    for ((p, _), q) in before.iter().zip(&after) {
        let close = (tris.iter().map(|t| closest64(to64(*p), t)))
            .map(|c| norm(sub(c, to64(*p))))
            .fold(f64::INFINITY, f64::min);
        let moved = norm(sub(to64(*q), to64(*p)));
        assert!(
            moved <= 2.0 * close + 2.0 * longest,
            "{p:?} moved {moved} to {q:?}; the floor is {close} away"
        );
        assert!(q.z.abs() < 1e-6, "{p:?} landed off the floor, at {q:?}");
    }
}

/// A source face with two corners in one place (an edge of no length, as two points pulled
/// onto the same spot leave) has no area, and the closest point search beside it must still
/// find the surface: no answer is not a number, which sorts first and empties the search.
#[test]
fn a_face_with_an_edge_of_no_length_does_not_hide_the_surface_beside_it() {
    let source = PlainMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, 0.5, 0.2],
            [0.5, 0.5, 0.2],
            [0.6, 0.5, 0.2],
        ],
        faces: vec![[0, 1, 2, 3], [4, 5, 6, NO]],
        ..PlainMesh::default()
    };
    let store = common::store(&source, common::small_nodes());
    let surface = Surface::of(&store, false);
    let hit = surface.find(Vec3::new(0.55, 0.5, 0.05), Vec3::ZERO, 0.3, Side::Both, 0.0);
    let floor = Vec3::new(0.55, 0.5, 0.0);
    assert!(
        hit.is_some_and(|h| (h.point - floor).len2() < 1e-12),
        "{hit:?}"
    );
}

/// Every live point of `store` with its normal, in node order.
fn points_and_normals_untipped(store: &MeshStore) -> Vec<(Vec3, Vec3)> {
    let mut out = Vec::new();
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        let live = node.pos.iter().zip(node.nrm).filter(|(p, _)| !p.is_dead());
        out.extend(live.map(|(&p, &d)| (p, d)));
    }
    out
}

#[test]
fn a_landing_names_its_face_and_the_weights_that_rebuild_it() {
    let rec = record(2);
    let (top, surface) = (rec.top(), Surface::of(rec.top(), false));
    let pos = top.pos();
    for (p, n) in points_and_normals(new_base(&rec, 1).top()) {
        let Some(hit) = surface.find(p, n, 0.05, Side::Both, 0.01) else {
            continue;
        };
        let face = top.node(top.config().face_node(hit.face)).faces
            [top.config().face_local(hit.face) as usize];
        assert!(hit.corners.iter().all(|c| face.contains(c)), "{hit:?}");
        assert!(hit.weights.iter().all(|&w| w >= 0.0), "{hit:?}");
        assert!(
            (hit.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5,
            "{hit:?}"
        );
        let rebuilt =
            (hit.corners.iter().zip(hit.weights)).fold(Vec3::ZERO, |a, (&c, w)| a + pos.get(c) * w);
        assert!((rebuilt - hit.point).len2().sqrt() < 1e-5, "{hit:?}");
    }
}

#[test]
fn unfreezing_lands_each_point_by_the_rule_and_the_same_on_one_thread_and_many() {
    let rec = record(3);
    let tris = triangles64(rec.top());
    let diag = diagonal(&top_positions(&rec));
    let reach = 0.02 * diag;
    // 96 faces, one division from the record's 384: the projection reads that division.
    let (divided, none) = ok(Levels::unfrozen(new_base(&rec, 2), &rec, 0.0));
    assert_eq!(none, 0, "a reach of 0 moves nothing");
    let (built, moved) = ok(Levels::unfrozen(new_base(&rec, 2), &rec, reach as f32));
    assert_eq!(
        (built.count(), built.active(), built.top().face_count()),
        (2, 1, 384)
    );
    let (store, after) = (divided.top(), built.top().pos());
    let edge = mean_edge64(store);
    let mut moves = 0;
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        for (l, (&p, &d)) in node.pos.iter().zip(node.nrm).enumerate() {
            if p.is_dead() {
                continue;
            }
            let q = after.get(node.base + l as u32);
            let landed = (q.bits() != p.bits()).then_some(q);
            let how = (reach, Side::Both, edge);
            let ok = agrees(&tris, (p, d), how, landed, 1e-5 * diag);
            assert!(ok, "{p:?} landed at {landed:?}");
            moves += usize::from(landed.is_some());
        }
    }
    assert_eq!(moves, moved);
    assert!(
        moved * 10 > store.vertex_count() * 9,
        "most points move: {moved}"
    );
    let runs = [1, 8].map(|threads| {
        gs_par::with_threads(threads, || {
            bits(&top_positions(
                &ok(Levels::unfrozen(new_base(&rec, 2), &rec, reach as f32)).0,
            ))
        })
    });
    assert_eq!(
        runs[0],
        bits(&top_positions(&built)),
        "one thread, same bits"
    );
    assert_eq!(runs[1], runs[0], "eight threads, same bits");
}

#[test]
fn two_divisions_toward_the_old_face_count_keep_the_detail() {
    let rec = record(4);
    let old = surface(&rec);
    let diag = diagonal(&top_positions(&rec));
    // 96 faces: two divisions make the record's 1536, each projected, so the first one's
    // smoothing does not pull the second out of reach.
    let reach = (0.02 * diag) as f32;
    let (built, moved) = ok(Levels::unfrozen(new_base(&rec, 2), &rec, reach));
    assert_eq!(
        (built.count(), built.active(), built.top().face_count()),
        (3, 2, 1536)
    );
    let after = top_positions(&built);
    assert_eq!(moved, after.len(), "every point was in reach");
    for q in &after {
        let on = to_surface(*q, &old);
        assert!(on <= 1e-5 * diag, "{q:?} is {on} off the old surface");
    }
    // Every old top point has a new one within an old edge length.
    let triangles = 2 * old.faces.len();
    let edge = 2.0 * diag / (triangles as f64).sqrt();
    for p in top_positions(&rec).iter().copied().map(to64) {
        let near = (after.iter().copied().map(to64))
            .map(|q| norm(sub(p, q)))
            .fold(f64::INFINITY, f64::min);
        assert!(near <= edge, "an old point is {near} from the new top");
    }
}

#[test]
fn unfreezing_reads_the_record_top_with_its_pending_edits() {
    let mut rec = record(3);
    let untouched = ok(Levels::unfrozen(new_base(&rec, 2), &rec, 0.06)).0;
    ok(rec.set(1));
    shift(&mut rec, |p| p * 1.01);
    assert!(rec.pending());
    let (built, _) = ok(Levels::unfrozen(new_base(&record(3), 2), &rec, 0.06));
    assert_ne!(
        top_positions(&built),
        top_positions(&untouched),
        "the edit on level 2 reaches the surface projected onto"
    );
}
