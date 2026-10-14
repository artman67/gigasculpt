//! The `closed` claim's checker (decision 0057, section 7) refuses an open mesh, an edge three
//! faces share and two faces that cross without sharing a point, and passes a closed cube and
//! two cubes that only touch, so the claim cannot pass vacuously.

use gs_protocol::{NO_VERTEX, PlainMesh};
use gs_testkit::Scene;
use gs_testkit::claims::closed::{closed, crossings, unclosed};
use gs_testkit::fixtures::{cube, patch};

/// `a` and `b` as one mesh, `b`'s points moved by `by`.
fn both(a: &PlainMesh, b: &PlainMesh, by: [f32; 3]) -> PlainMesh {
    let n = a.positions.len() as u32;
    let mut out = a.clone();
    let moved = (b.positions.iter()).map(|p| [p[0] + by[0], p[1] + by[1], p[2] + by[2]]);
    out.positions.extend(moved);
    let renumbered = (b.faces.iter()).map(|f| f.map(|v| if v == NO_VERTEX { v } else { v + n }));
    out.faces.extend(renumbered);
    out
}

fn why(mesh: &PlainMesh) -> String {
    unclosed(mesh).unwrap_or_default()
}

#[test]
fn a_closed_cube_and_cubes_that_only_touch_pass() {
    assert_eq!(unclosed(&cube(2)), None, "a closed cube");
    let touching = both(&cube(2), &cube(2), [2.0, 0.0, 0.0]);
    assert_eq!(unclosed(&touching), None, "two cubes flat on each other");
}

#[test]
fn an_open_mesh_fails() {
    assert!(why(&patch(3)).contains("is open"), "{}", why(&patch(3)));
    let mut opened = cube(2);
    opened.faces.pop();
    assert!(why(&opened).contains("is open"), "{}", why(&opened));
}

#[test]
fn an_edge_three_faces_share_fails() {
    let mut fin = cube(2);
    let [a, b, ..] = fin.faces[0];
    fin.positions.push([5.0, 5.0, 5.0]);
    let tip = fin.positions.len() as u32 - 1;
    fin.faces.push([a, b, tip, NO_VERTEX]);
    assert!(why(&fin).contains("not manifold"), "{}", why(&fin));
}

#[test]
fn faces_that_cross_without_sharing_a_point_fail() {
    // Off the lattice: a half step would meet only along edges and diagonals, which do not count.
    let crossed = both(&cube(2), &cube(2), [0.3, 0.2, 0.1]);
    assert!(
        why(&crossed).contains("crosses itself"),
        "{}",
        why(&crossed)
    );
    let tris: Vec<[u32; 3]> = crossed.triangles().collect();
    let pairs = crossings(&crossed.positions, &tris);
    assert!(
        pairs.iter().all(|&(s, t)| s < t),
        "pairs are named lower first"
    );
}

#[test]
fn only_parts_the_call_adds_are_checked() {
    let open = patch(3);
    let before: Scene = [("patch".to_string(), open.clone())].into();
    let mut after = before.clone();
    assert_eq!(closed(&before, &after), Ok(false), "nothing was added");
    after.insert("new".into(), cube(2));
    assert_eq!(
        closed(&before, &after),
        Ok(true),
        "an open part already there is not the call's"
    );
    after.insert("newer".into(), open);
    let err = closed(&before, &after).err().unwrap_or_default();
    assert!(err.contains("'newer'") && err.contains("is open"), "{err}");
}
