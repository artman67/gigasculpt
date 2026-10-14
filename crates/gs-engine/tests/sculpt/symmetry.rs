//! Live Detail with a mirror map (decision 0021, section 10): `mesh.symmetrize` makes a part
//! exactly symmetric with its map, mirrored refining strokes keep the map valid, a stroke
//! without the mirror releases it and says so, `symmetry.map` finds it again or names the point
//! at fault, and strokes started on the other side of the plane give the same topology.

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, NO_VERTEX, PlainMesh};
use gs_testkit::{Expect, Session, text, validate};
use serde_json::{Value, json};

use crate::{sphere_session, stroke, with};

/// The map column across x, as `part.get` gives it.
const MAP: &str = "mirror.x";

/// Strokes over the sphere from a seeded stream: each from one random point of its front
/// to another, over the surface, half of them crossing the plane x = 0.
fn strokes(seed: u64, count: usize) -> Vec<Vec<[f32; 3]>> {
    let mut state = seed;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut point = || {
        let p = [1.6 * next() - 0.8, 1.2 * next() - 0.4, 0.5 + 0.5 * next()];
        let len = p.iter().map(|c| c * c).sum::<f32>().sqrt();
        p.map(|c| c / len)
    };
    (0..count)
        .map(|_| {
            let (a, b) = (point(), point());
            (0..10)
                .map(|i| {
                    let t = i as f32 / 9.0;
                    let p: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
                    let len = p.iter().map(|c| c * c).sum::<f32>().sqrt();
                    p.map(|c| c / len)
                })
                .collect()
        })
        .collect()
}

/// `brush.stroke` arguments along `points` refining at `size`, mirrored across x if `mirror`.
fn refining(brush: &str, points: &[[f32; 3]], size: f32, mirror: bool) -> Value {
    let mut extra = json!({ "live_detail": { "size": size } });
    if mirror {
        extra["symmetry"] = json!({ "mirror": { "x": true } });
    }
    stroke(brush, points, &with(json!({}), &extra))
}

/// Brush and Live Detail size of stroke `i`: splitting and collapsing in turn.
fn kind(i: usize) -> (&'static str, f32) {
    [("Draw", 0.05), ("Clay", 0.2), ("Draw", 0.1)][i % 3]
}

/// The partners of `mesh`'s map across x.
fn partners(mesh: &PlainMesh) -> Vec<u32> {
    match mesh.attributes.get(MAP).map(|a| (a.domain, &a.values)) {
        Some((AttrDomain::Vertex, AttrValues::U32(p))) => p.clone(),
        other => panic!("no point map {MAP}: {other:?}"),
    }
}

/// A face's corners.
fn corners(f: &[u32; 4]) -> impl Iterator<Item = u32> + '_ {
    f.iter().copied().filter(|&v| v != NO_VERTEX)
}

/// A face as the cycle of its corners starting from the lowest.
fn cycle(corners: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut c: Vec<u32> = corners.collect();
    let first = (0..c.len()).min_by_key(|&i| c[i]).unwrap_or_default();
    c.rotate_left(first);
    c
}

/// Checks that `mesh` is a closed manifold sphere and carries a valid map across x: each
/// point's partner names it back and lies within `tol` of its mirror image, and every face's
/// mirror image (its corners' partners, winding reversed) is a face. Returns the points on the
/// plane.
fn valid_map(mesh: &PlainMesh, tol: f32) -> usize {
    if let Err(defects) = validate(mesh, Expect::closed(2)) {
        panic!("not a valid sphere: {defects:?}");
    }
    let p = partners(mesh);
    assert_eq!(p.len(), mesh.positions.len(), "one partner per point");
    for (i, &j) in p.iter().enumerate() {
        assert_eq!(
            p[j as usize], i as u32,
            "point {i}'s partner {j} names another"
        );
        let (a, b) = (mesh.positions[i], mesh.positions[j as usize]);
        let off = (-a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .max((a[2] - b[2]).abs());
        assert!(
            off <= tol,
            "point {i} lies {off} from its partner {j}'s mirror image"
        );
    }
    let faces: BTreeSet<Vec<u32>> = mesh.faces.iter().map(|f| cycle(corners(f))).collect();
    for f in &mesh.faces {
        let image: Vec<u32> = corners(f).map(|v| p[v as usize]).collect();
        let image = cycle(image.into_iter().rev());
        assert!(
            faces.contains(&image),
            "face {f:?} has no mirror image {image:?}"
        );
    }
    p.iter()
        .enumerate()
        .filter(|&(i, &j)| j == i as u32)
        .count()
}

/// A session with the sphere symmetrized across x.
fn symmetric_sphere() -> Session {
    let mut s = sphere_session(None);
    let done = s.query("mesh.symmetrize", json!({ "axis": "x" }));
    assert!(done["on_plane"].as_u64().unwrap_or_default() > 0, "{done}");
    s
}

#[test]
fn mirrored_refining_strokes_keep_the_map_valid() {
    let mut s = symmetric_sphere();
    valid_map(&s.mesh("sphere"), 0.0);
    let (mut splits, mut collapses) = (0, 0);
    for (i, points) in strokes(7, 6).iter().enumerate() {
        let (brush, size) = kind(i);
        let done = s.query("brush.stroke", refining(brush, points, size, true));
        let ld = &done["live_detail"];
        assert_eq!(ld["mirror_cleared"], json!(false), "stroke {i}: {done}");
        assert!(ld.get("symmetry").is_none(), "stroke {i}: {done}");
        splits += ld["splits"].as_u64().unwrap_or_default();
        collapses += ld["collapses"].as_u64().unwrap_or_default();
        valid_map(&s.mesh("sphere"), 1e-4);
    }
    assert!(
        splits > 0 && collapses > 0,
        "{splits} splits, {collapses} collapses"
    );
}

#[test]
fn an_unmirrored_stroke_releases_the_map_and_says_so() {
    let mut s = symmetric_sphere();
    let points = &strokes(11, 1)[0];
    let done = s.query("brush.stroke", refining("Draw", points, 0.05, false));
    let ld = &done["live_detail"];
    assert_eq!(ld["mirror_cleared"], json!(true), "{done}");
    assert!(text(&ld["symmetry"]).contains("mesh.symmetrize"), "{done}");
    assert!(
        !s.mesh("sphere").attributes.contains_key(MAP),
        "the map is gone"
    );
    s.ok("history.undo", json!({}));
    valid_map(&s.mesh("sphere"), 0.0);
    s.ok("history.redo", json!({}));
    // A mirrored stroke on a part without a map is symmetric only as far as floats allow.
    let done = s.query("brush.stroke", refining("Draw", points, 0.05, true));
    let note = text(&done["live_detail"]["symmetry"]);
    assert!(
        note.contains("symmetry.map") && note.contains("mesh.symmetrize"),
        "{done}"
    );
}

/// A mirrored refining stroke edits in pairs, 2 operations each, so a budget of 1 would never
/// edit: it is refused with the reason, and a budget of 2 refines (review 0006, m2).
#[test]
fn a_mirrored_stroke_needs_a_budget_of_two() {
    let mut s = symmetric_sphere();
    let points = &strokes(7, 1)[0];
    let budget = |b: u32| {
        let mut args = refining("Draw", points, 0.05, true);
        args["live_detail"]["budget"] = json!(b);
        args
    };
    let err = s.err("brush.stroke", budget(1));
    assert!(err.message.contains("at least 2"), "{}", err.message);
    let done = s.query("brush.stroke", budget(2));
    let ld = &done["live_detail"];
    assert_eq!(ld["refined"], json!(true), "{done}");
    assert_eq!(ld["mirror_cleared"], json!(false), "{done}");
    valid_map(&s.mesh("sphere"), 1e-4);
}

#[test]
fn symmetrize_gives_a_sculpted_mesh_a_valid_map_that_symmetry_map_finds_again() {
    let mut s = sphere_session(None);
    for (i, points) in strokes(3, 3).iter().enumerate() {
        let (brush, size) = kind(i);
        s.ok("brush.stroke", refining(brush, points, size, false));
    }
    let refused = s.err("symmetry.map", json!({ "axis": "x" }));
    assert!(refused.message.contains("has no partner"), "{refused:?}");
    let done = s.query(
        "mesh.symmetrize",
        json!({ "axis": "x", "side": "negative" }),
    );
    let mesh = s.mesh("sphere");
    let on_plane = valid_map(&mesh, 0.0);
    assert_eq!(done["on_plane"].as_u64(), Some(on_plane as u64), "{done}");
    s.ok("symmetry.map", json!({ "axis": "x", "clear": true }));
    assert!(!s.mesh("sphere").attributes.contains_key(MAP), "released");
    let wide = s.err("symmetry.map", json!({ "axis": "x", "tolerance": "1%" }));
    assert_eq!(wide.path.as_deref(), Some("/tolerance"), "{wide:?}");
    // Symmetrize mirrors coordinates exactly, so a tolerance of 0 finds the map.
    let found = s.query("symmetry.map", json!({ "axis": "x", "tolerance": 0 }));
    assert_eq!(found["held"], json!(true), "{found}");
    assert_eq!(partners(&s.mesh("sphere")), partners(&mesh), "the same map");
}

/// Mirrored refining strokes started on the other side of the plane run the same pairs: with
/// strength 0, so only Live Detail changes the mesh, the part ends the same bit for bit. (A
/// sculpting dab takes its frame from the side it starts on, so its two sides agree only to
/// float precision, and a decision that close to a threshold may go either way.)
#[test]
fn strokes_started_on_the_other_side_give_the_same_topology() {
    let run = |flip: bool| {
        let mut s = symmetric_sphere();
        for (i, points) in strokes(19, 4).iter().enumerate() {
            let points: Vec<[f32; 3]> = (points.iter())
                .map(|&[x, y, z]| [if flip { -x } else { x }, y, z])
                .collect();
            let (brush, size) = kind(i);
            let mut args = refining(brush, &points, size, true);
            args["strength"] = json!(0.0);
            s.ok("brush.stroke", args);
        }
        (s.mesh("sphere"), s.hash())
    };
    let ((a, ha), (b, hb)) = (run(false), run(true));
    valid_map(&a, 1e-6);
    assert!(
        a.faces.len() > 20_000,
        "the strokes refined: {} faces",
        a.faces.len()
    );
    assert!(a.faces == b.faces, "the same faces");
    assert_eq!(ha, hb, "the same part, bit for bit");
}
