//! `mesh.refine` (decision 0021, L4): Live Detail's pass over a whole part comes to rest as one
//! undo step and a second run changes nothing; sizes A, B, A from one shape give one mesh; a
//! mirrored refine keeps the mirror map; parts with detail levels or UVs are refused with the remedy.

use std::collections::BTreeSet;

use gs_core::live::TYPICAL_EDGE;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::{Expect, Session, validate};
use serde_json::json;

/// The part as a closed manifold surface, or a panic listing what is wrong.
fn closed(s: &mut Session, part: &str) -> PlainMesh {
    let mesh = s.mesh(part);
    if let Err(defects) = validate(&mesh, Expect::closed(2)) {
        panic!("{part} is not a valid closed surface: {defects:?}");
    }
    mesh
}

#[test]
fn refine_comes_to_rest_and_a_second_run_changes_nothing() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let (start, before) = (s.hash(), closed(&mut s, "sphere").positions.len());
    let done = s.ok("mesh.refine", json!({ "size": 0.1 }));
    let r = &done.result;
    assert_eq!(r["converged"], json!(true), "{r}");
    let count = |k: &str| r[k].as_u64().unwrap_or_default();
    assert!(count("splits") > 0 && count("points_added") > 0, "{r}");
    let mesh = closed(&mut s, "sphere");
    assert_eq!(count("points"), mesh.positions.len() as u64, "{r}");
    assert!(
        mesh.positions.len() > 4 * before,
        "{before} points became {r}"
    );
    let refined = s.hash();
    let again = s.ok("mesh.refine", json!({ "size": 0.1 }));
    let effect = again.effect.unwrap_or_else(|| panic!("no effect"));
    assert!(effect.noop_reason.is_some(), "{effect:?}");
    assert_eq!(again.result["passes"], json!(1), "{}", again.result);
    assert_eq!(s.hash(), refined, "the second run changed the scene");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), start, "one undo step gives back the sphere");
}

/// The Check of `geometry.triangle-remesh`: a sphere of about 2000 triangles refined at half its
/// mean edge is all triangles, with every point within 0.5 percent of the sphere; at twice its
/// mean edge, splitting alone keeps the count and merging lowers it. Splitting alone at a size
/// past every edge is how the quad sphere becomes the Check's. The count comes out 5.0 times
/// (10214 from 2028), not the Check's four: the edges made average 0.0565 against 0.0604 asked
/// (a target of the edge over [`TYPICAL_EDGE`], as the picker sets), and they are less even.
#[test]
fn a_sphere_refined_at_half_its_edge_is_triangles_that_long_on_its_shape() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 13 }));
    let quads = |m: &PlainMesh| m.faces.iter().filter(|f| f[3] != NO_VERTEX).count();
    let sphere = s.mesh("sphere");
    let (points, faces) = (sphere.positions.len(), 2 * quads(&sphere));
    s.ok("mesh.refine", json!({ "size": 10.0, "combined": false }));
    let start = closed(&mut s, "sphere");
    assert_eq!((quads(&start), start.faces.len()), (0, faces));
    assert_eq!(start.positions.len(), points);
    // Polygon Size is the typical edge wanted; `size` is the longest edge kept, as the picker maps.
    let (mean, _) = crate::pick::mean_edge(&start, |_| true);
    let at = |edge: f32| edge / TYPICAL_EDGE;
    s.ok("mesh.refine", json!({ "size": at(mean / 2.0) }));
    let fine = closed(&mut s, "sphere");
    let ratio = fine.faces.len() as f32 / faces as f32;
    let (made, _) = crate::pick::mean_edge(&fine, |_| true);
    assert_eq!(quads(&fine), 0, "quads are left");
    assert!(
        (3.5..=5.5).contains(&ratio) && (made - mean / 2.0).abs() < 0.15 * mean / 2.0,
        "{faces} triangles became {ratio} times as many, edges {mean} became {made}"
    );
    let off = (fine.positions.iter())
        .map(|&[x, y, z]| ((x * x + y * y + z * z).sqrt() - 1.0).abs())
        .fold(0.0f32, f32::max);
    assert!(off <= 0.005, "a point lies {off} off the sphere");
    s.ok("history.undo", json!({}));
    let coarse = json!({ "size": at(mean * 2.0), "combined": false });
    s.ok("mesh.refine", coarse);
    assert_eq!(s.mesh("sphere").faces.len(), faces);
    s.ok("mesh.refine", json!({ "size": at(mean * 2.0) }));
    let fewer = s.mesh("sphere").faces.len();
    assert!(fewer < faces * 3 / 4, "{faces} triangles became {fewer}");
}

/// Sizes A, B, A give the same mesh the first and third time when each is computed from the
/// shape the part had at the first try, as the catalog's slider is (`geometry.triangle-remesh`):
/// undo between tries gives that shape back exactly. Run one after another instead, a coarser
/// size keeps the edges already between its merge length and itself, so the count differs.
#[test]
fn sizes_a_b_a_from_the_same_shape_give_the_same_mesh() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let mut seen = Vec::new();
    for size in [0.1, 0.05, 0.1, 0.2, 0.1] {
        let r = s.ok("mesh.refine", json!({ "size": size })).result;
        seen.push((r["points"].as_u64().unwrap_or_default(), s.hash()));
        s.ok("history.undo", json!({}));
    }
    let points: Vec<u64> = seen.iter().map(|p| p.0).collect();
    assert!(points[1] > points[0] && points[3] < points[0], "{points:?}");
    assert!(
        seen[2] == seen[0] && seen[4] == seen[0],
        "sizes gave {points:?}"
    );
}

#[test]
fn a_mirrored_refine_keeps_the_mirror_map() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let err = s.err("mesh.refine", json!({ "size": 0.1, "symmetry": "x" }));
    assert_eq!(err.code, ErrorCode::Precondition, "{err:?}");
    s.ok("symmetry.map", json!({ "axis": "x" }));
    let done = s
        .ok("mesh.refine", json!({ "size": 0.07, "symmetry": "x" }))
        .result;
    assert_eq!(done["mirror_cleared"], json!(false), "{done}");
    let mesh = closed(&mut s, "sphere");
    let p = match mesh.attributes.get("mirror.x").map(|a| &a.values) {
        Some(AttrValues::U32(p)) => p.clone(),
        other => panic!("no map across x: {other:?}"),
    };
    assert!(done["points_added"].as_u64() > Some(1000), "{done}");
    let mut most = 0.0f32;
    for (i, &j) in p.iter().enumerate() {
        assert_eq!(
            p[j as usize], i as u32,
            "point {i}'s partner {j} names another"
        );
        let ([ax, ay, az], [bx, by, bz]) = (mesh.positions[i], mesh.positions[j as usize]);
        most = most.max((ax + bx).abs().max((ay - by).abs()).max((az - bz).abs()));
    }
    assert!(
        most <= 1e-6,
        "a point lies {most} from its partner's mirror image"
    );
    let cycle = |c: Vec<u32>| {
        let first = (0..c.len()).min_by_key(|&i| c[i]).unwrap_or_default();
        [&c[first..], &c[..first]].concat()
    };
    let corners =
        |f: &[u32; 4]| -> Vec<u32> { f.iter().copied().filter(|&v| v != NO_VERTEX).collect() };
    let faces: BTreeSet<Vec<u32>> = mesh.faces.iter().map(|f| cycle(corners(f))).collect();
    for f in &mesh.faces {
        let image: Vec<u32> = corners(f).iter().rev().map(|&v| p[v as usize]).collect();
        assert!(
            faces.contains(&cycle(image)),
            "face {f:?} has no mirror image"
        );
    }
}

#[test]
fn parts_with_levels_or_uvs_are_refused_with_the_remedy() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    let mut mesh = s.mesh("sphere");
    let uv: Vec<f32> = (0..mesh.faces.len()).map(|f| f as f32 * 0.001).collect();
    for name in ["uv0.u", "uv0.v"] {
        let column = Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::F32(uv.clone()),
        };
        mesh.attributes.insert(name.into(), column);
    }
    s.add_part("mapped", &mesh);
    let err = s.err("mesh.refine", json!({ "part": "mapped", "size": 0.1 }));
    assert_eq!(err.code, ErrorCode::Precondition, "{err:?}");
    assert!(err.message.contains("uvs off"), "{err:?}");
    s.ok("level.subdivide", json!({ "part": "sphere" }));
    let err = s.err("mesh.refine", json!({ "part": "sphere", "size": 0.1 }));
    assert_eq!(err.code, ErrorCode::Precondition, "{err:?}");
    assert!(err.message.contains("detail levels"), "{err:?}");
    let hint = err.hint.unwrap_or_default();
    assert!(hint.contains("level.delete_higher"), "{hint}");
}
