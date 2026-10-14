//! `mesh.quad_remesh` through the engine (decision 0038, section 7): one closed shell of quads
//! near the count, the same bytes twice, holes and handles kept, detail levels on the old
//! surface within decision 0030's bound, the symmetric route with its map, creases and groups
//! followed, columns carried, and refusals that change nothing. These run the program, built
//! once per pin (`third_party/quadwild/`); a missing program is the gigasculpt binary's test.

use std::collections::{BTreeMap, BTreeSet};

use gs_core::Vec3;
use gs_geo::quad::shape;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::{Session, fixtures, measure};
use serde_json::{Value, json};

use crate::support::{agent, stroke};

fn sphere(subdivisions: u32) -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": subdivisions }));
    s
}

/// Shells, open loops and Euler characteristic, failing on an edge with more than two faces.
fn form(mesh: &PlainMesh) -> (usize, usize, i64) {
    let found = shape(mesh).unwrap_or_else(|e| panic!("{e}"));
    (found.shells, found.loops, found.euler)
}

fn all_quads(mesh: &PlainMesh) -> bool {
    mesh.faces.iter().all(|f| f[3] != NO_VERTEX)
}

fn centroid(mesh: &PlainMesh, f: &[u32; 4]) -> [f32; 3] {
    let c = f.map(|v| mesh.positions[v as usize]);
    std::array::from_fn(|k| c.iter().map(|p| p[k]).sum::<f32>() / 4.0)
}

fn length(p: [f32; 3], q: [f32; 3]) -> f64 {
    f64::from((Vec3::from_array(p) - Vec3::from_array(q)).len2()).sqrt()
}

#[test]
fn a_sphere_becomes_one_closed_shell_of_quads_near_the_count_the_same_every_time() {
    let mut s = sphere(16);
    let before = s.hash();
    let done = s.ok("mesh.quad_remesh", json!({ "faces": 400 })).result;
    let mesh = s.mesh("sphere");
    let n = mesh.faces.len();
    assert!((360..=440).contains(&n), "{n} quads for 400");
    assert!(all_quads(&mesh), "quads only");
    assert_eq!(form(&mesh), (1, 0, 2), "one closed shell");
    assert_eq!(done["quads"], json!(n), "{done}");
    assert_eq!(done["out_of_reach"], json!(0), "{done}");
    let after = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo step");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo");
    let mut again = sphere(16);
    again.ok("mesh.quad_remesh", json!({ "faces": 400 }));
    assert_eq!(again.hash(), after, "the same bytes twice");
}

/// A domed sheet of 16 by 16 quads with the middle 4 by 4 taken out: one shell, two rims.
fn holed() -> PlainMesh {
    let mut sheet = fixtures::patch(16);
    let middle = |v: u32| (6..=10).contains(&(v / 17)) && (6..=10).contains(&(v % 17));
    sheet.faces.retain(|f| !f.iter().all(|&v| middle(v)));
    let used: BTreeSet<u32> = sheet.faces.iter().flatten().copied().collect();
    let new: BTreeMap<u32, u32> = (used.iter().enumerate())
        .map(|(k, &v)| (v, k as u32))
        .collect();
    sheet.positions = used.iter().map(|&v| sheet.positions[v as usize]).collect();
    sheet.faces.iter_mut().flatten().for_each(|v| *v = new[v]);
    sheet
}

/// Two tori overlapping side by side, voxel remeshed into one closed surface of genus 2.
fn two_handles(s: &mut Session) {
    let (a, b) = (fixtures::torus(), fixtures::torus());
    let n = a.positions.len() as u32;
    let moved = (b.positions.iter()).map(|p| [p[0] + 1.75, p[1], p[2]]);
    let mesh = PlainMesh {
        positions: a.positions.iter().copied().chain(moved).collect(),
        faces: (a.faces.iter().copied())
            .chain(b.faces.iter().map(|f| f.map(|v| v + n)))
            .collect(),
        attributes: BTreeMap::new(),
    };
    s.add_part("handles", &mesh);
    s.ok(
        "mesh.voxel_remesh",
        json!({ "part": "handles", "resolution": 48 }),
    );
}

#[test]
fn a_hole_and_two_handles_are_kept() {
    let mut s = agent();
    s.add_part("holed", &holed());
    assert_eq!(form(&s.mesh("holed")), (1, 2, 0), "the sheet as made");
    s.ok("mesh.quad_remesh", json!({ "part": "holed", "faces": 300 }));
    let out = s.mesh("holed");
    assert!(all_quads(&out), "quads only");
    assert_eq!(form(&out), (1, 2, 0), "one shell, its rim and its hole");
    two_handles(&mut s);
    assert_eq!(form(&s.mesh("handles")), (1, 0, -2), "genus 2 as made");
    s.ok(
        "mesh.quad_remesh",
        json!({ "part": "handles", "faces": 600 }),
    );
    let out = s.mesh("handles");
    assert!(all_quads(&out), "quads only");
    assert_eq!(form(&out), (1, 0, -2), "genus 2 kept");
}

/// Decision 0030 section 4's stated tolerance, as `mesh.project_detail` is held to it: every
/// point of the new top lies on the old surface (within 1e-5 of its diagonal, every point
/// being in reach), and the old points lie on average within one old edge length (the mean)
/// of a new top point.
#[test]
fn detail_lands_on_the_old_surface_within_the_bound() {
    let mut s = sphere(24);
    for at in [[0.0, 0.0, 1.0], [0.6, 0.0, 0.8], [0.0, -1.0, 0.0]] {
        s.ok("brush.stroke", stroke(at));
    }
    let old = s.mesh("sphere");
    let args = json!({ "faces": 300, "detail": true, "distance": "5%" });
    let done = s.ok("mesh.quad_remesh", args).result;
    let top = s.mesh("sphere");
    assert!(
        done["levels_added"].as_u64().is_some_and(|n| n >= 1),
        "{done}"
    );
    assert_eq!(done["out_of_reach"], json!(0), "{done}");
    let levels = s.query("level.describe", json!({}));
    let base = levels["levels"][0]["faces"].as_u64().unwrap_or_default();
    assert!((270..=330).contains(&base), "{base} quads at level 0");
    let b = measure::bounds(&old.positions).unwrap_or_default();
    let diag = length(b[0], b[1]);
    let points = PlainMesh {
        positions: top.positions.clone(),
        ..PlainMesh::default()
    };
    let on = measure::hausdorff_from(&points, &old);
    assert!(on <= 1e-5 * diag, "a new point is {on} off the old surface");
    let edges: Vec<f64> = (old.faces.iter())
        .flat_map(|f| (0..4).map(move |k| (f[k], f[(k + 1) % 4])))
        .map(|(a, b)| length(old.positions[a as usize], old.positions[b as usize]))
        .collect();
    let mean_edge = edges.iter().sum::<f64>() / edges.len() as f64;
    let total: f64 = (old.positions.iter())
        .map(|&p| (top.positions.iter().map(|&q| length(p, q))).fold(f64::INFINITY, f64::min))
        .sum();
    let mean = total / old.positions.len() as f64;
    assert!(
        mean <= mean_edge,
        "old points are {mean} from the new top on average"
    );
}

#[test]
fn a_symmetric_part_stays_exactly_symmetric_and_holds_its_map() {
    let mut s = sphere(12);
    let done = s.ok("mesh.quad_remesh", json!({ "faces": 300, "symmetry": "x" }));
    let mesh = s.mesh("sphere");
    assert!(all_quads(&mesh), "quads only: {}", done.result);
    assert_eq!(form(&mesh), (1, 0, 2), "one closed shell");
    assert!(mesh.attributes.contains_key("mirror.x"), "the map");
    let at: BTreeSet<[u32; 3]> = (mesh.positions.iter())
        .map(|p| p.map(f32::to_bits))
        .collect();
    for p in &mesh.positions {
        let q = [-p[0] + 0.0, p[1], p[2]];
        assert!(
            at.contains(&q.map(f32::to_bits)),
            "{p:?} has its mirror image"
        );
    }
    s.ok("symmetry.map", json!({ "axis": "x", "tolerance": 0 }));
}

/// The equator's edges of a sphere of 8 by 8 quads a side, as `level.subdivide` creases.
fn equator(mesh: &PlainMesh) -> Vec<Value> {
    let flat = |v: u32| mesh.positions[v as usize][1].abs() < 1e-6;
    let mut edges = BTreeSet::new();
    for f in &mesh.faces {
        for k in 0..4 {
            let (a, b) = (f[k], f[(k + 1) % 4]);
            if flat(a) && flat(b) {
                edges.insert((a.min(b), a.max(b)));
            }
        }
    }
    (edges.into_iter())
        .map(|(a, b)| json!({ "a": a, "b": b, "sharpness": 255 }))
        .collect()
}

#[test]
fn a_frozen_parts_creases_are_followed_unless_left_out() {
    let mut s = sphere(8);
    let creases = equator(&s.mesh("sphere"));
    assert_eq!(creases.len(), 32, "the equator");
    s.ok("level.subdivide", json!({ "creases": creases }));
    s.ok("level.freeze", json!({}));
    let frozen = s.hash();
    let kept = s.ok("mesh.quad_remesh", json!({ "faces": 300 })).result;
    assert_eq!(kept["sharp_edges"], json!(32), "{kept}");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), frozen, "undone");
    let args = json!({ "faces": 300, "keep_creases": false });
    let plain = s.ok("mesh.quad_remesh", args).result;
    assert_eq!(plain["sharp_edges"], json!(0), "{plain}");
}

/// A sphere with a mask that rises with y, one color, groups by the sign of x and UVs.
fn marked(s: &mut Session) -> PlainMesh {
    let mut mesh = s.mesh("sphere");
    let ramp = (mesh.positions.iter()).map(|p| f32::midpoint(p[1], 1.0));
    let groups = (mesh.faces.iter())
        .map(|f| u32::from(centroid(&mesh, f)[0] > 0.0))
        .collect();
    let column = |domain, values| Attribute { domain, values };
    let mut columns = vec![
        (
            "mask".into(),
            column(AttrDomain::Vertex, AttrValues::F32(ramp.collect())),
        ),
        (
            "color".into(),
            column(
                AttrDomain::Vertex,
                AttrValues::U32(vec![0xff33_6699; mesh.positions.len()]),
            ),
        ),
        (
            "group".into(),
            column(AttrDomain::Face, AttrValues::U32(groups)),
        ),
    ];
    for k in 0..8 {
        let uv = AttrValues::F32(vec![0.5; mesh.faces.len()]);
        columns.push((
            format!("uv{}.{}", k / 2, ["u", "v"][k % 2]),
            column(AttrDomain::Face, uv),
        ));
    }
    mesh.attributes.extend(columns);
    mesh
}

#[test]
fn mask_and_paint_come_across_uvs_do_not_and_groups_only_when_kept() {
    let mut s = sphere(12);
    let mesh = marked(&mut s);
    for keep_groups in [false, true] {
        let name = format!("marked-{keep_groups}");
        s.add_part(&name, &mesh);
        let args = json!({ "part": name, "faces": 300, "keep_groups": keep_groups });
        let done = s.ok("mesh.quad_remesh", args).result;
        assert_eq!(done["out_of_reach"], json!(0), "{done}");
        let out = s.mesh(&name);
        let Some(AttrValues::F32(mask)) = out.attributes.get("mask").map(|a| &a.values) else {
            panic!("no mask");
        };
        for (p, m) in out.positions.iter().zip(mask) {
            let want = f32::midpoint(p[1], 1.0);
            assert!((m - want).abs() < 0.02, "mask {m} at {p:?}, not {want}");
        }
        let color = out.attributes.get("color").map(|a| &a.values);
        let painted =
            matches!(color, Some(AttrValues::U32(c)) if c.iter().all(|&c| c == 0xff33_6699));
        assert!(painted, "the paint");
        assert!(
            !out.attributes.keys().any(|k| k.starts_with("uv")),
            "no UVs"
        );
        let groups = out.attributes.get("group").map(|a| &a.values);
        match groups {
            Some(AttrValues::U32(g)) if keep_groups => {
                for (f, &g) in out.faces.iter().zip(g) {
                    assert_eq!(
                        g,
                        u32::from(centroid(&out, f)[0] > 0.0),
                        "a quad in one group"
                    );
                }
            }
            None if !keep_groups => {}
            _ => panic!("groups with keep_groups {keep_groups}: {groups:?}"),
        }
    }
}

#[test]
fn every_refusal_leaves_the_scene_as_it_was() {
    let mut s = sphere(8);
    let refused = |s: &mut Session, args: Value, want: ErrorCode| {
        let before = s.hash();
        let err = s.err("mesh.quad_remesh", args.clone());
        assert_eq!(err.code, want, "{args}: {}", err.message);
        assert_eq!(s.hash(), before, "{args} changed the scene");
        err.hint.unwrap_or_default()
    };
    refused(&mut s, json!({ "faces": 49 }), ErrorCode::ArgsInvalid);
    refused(&mut s, json!({ "faces": 200_001 }), ErrorCode::ArgsInvalid);
    refused(&mut s, json!({ "distance": 0 }), ErrorCode::ArgsInvalid);
    let hide = json!({ "select": { "by": "faces", "indexes": [0, 1, 2] } });
    s.ok("visibility.hide", hide);
    let hint = refused(&mut s, json!({}), ErrorCode::Precondition);
    assert!(hint.contains("visibility.show_all"), "{hint}");
    s.ok("visibility.show_all", json!({}));
    s.ok("level.subdivide", json!({}));
    refused(&mut s, json!({}), ErrorCode::Precondition);
    s.ok("level.freeze", json!({}));
    let hint = refused(&mut s, json!({ "detail": true }), ErrorCode::Precondition);
    assert!(hint.contains("level.unfreeze"), "{hint}");
    let mut s = sphere(8);
    s.ok("mesh.jitter", json!({ "amount": "2%", "seed": 3 }));
    let hint = refused(&mut s, json!({ "symmetry": "x" }), ErrorCode::Precondition);
    assert!(hint.contains("without symmetry"), "{hint}");
}
