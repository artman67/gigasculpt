//! Select Rect and other closed outlines (decision 0015): which faces count as under the outline
//! (the smart rule, through the whole depth), what Select Rect shows and hides, perspective,
//! one run per symmetry pass, renumbering, and that hidden points never move. Every expectation
//! is computed here from plain data, independently of gs-core's gesture code.

use std::collections::BTreeSet;

use gs_core::{NO, StrokeSample, Vec3, View};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::brush_kit::{
    arc, at, grid, hidden_faces, ico, lasso, mirror_x, params, preset, run, store,
};

fn corners(m: &PlainMesh, f: &[u32; 4]) -> Vec<Vec3> {
    (f.iter().filter(|&&v| v != NO))
        .map(|&v| Vec3::from_array(m.positions[v as usize]))
        .collect()
}

/// An outline on the plane z = 2, seen along -z.
pub fn outline(pts: &[(f32, f32)]) -> Vec<StrokeSample> {
    pts.iter()
        .map(|&(x, y)| at(Vec3::new(x, y, 2.0), 0.1))
        .collect()
}

/// Select Rect's press and release, corner to corner.
fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<StrokeSample> {
    outline(&[(x0, y0), (x1, y1)])
}

/// Which corners of each face fall inside the outline `poly` once `project` carries them to the
/// plane: `None` for a face with a corner too close to the outline to call.
fn inside_counts(
    m: &PlainMesh,
    poly: &[(f32, f32)],
    project: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Option<(usize, usize)>> {
    let side = |a: (f32, f32), b: (f32, f32), q: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        (dx * (q.1 - a.1) - dy * (q.0 - a.0)) / len
    };
    let edges: Vec<_> = (0..poly.len())
        .map(|i| (poly[i], poly[(i + 1) % poly.len()]))
        .collect();
    m.faces
        .iter()
        .map(|f| {
            let mut n = 0;
            let c = corners(m, f);
            for p in &c {
                let q = project(*p);
                // Convex outlines, counterclockwise: inside is left of every edge.
                let s: Vec<f32> = edges.iter().map(|&(a, b)| side(a, b, q)).collect();
                if s.iter().any(|x| x.abs() < 1e-3) {
                    return None;
                }
                n += usize::from(s.iter().all(|&x| x > 0.0));
            }
            Some((n, c.len()))
        })
        .collect()
}

fn straight(p: Vec3) -> (f32, f32) {
    (p.x, p.y)
}

/// What the smart rule shows: faces wholly inside, if any face is, else faces with a corner
/// inside. Faces too close to call are `None`.
fn smart(counts: &[Option<(usize, usize)>]) -> Vec<Option<bool>> {
    let any_whole = counts.iter().flatten().any(|&(n, k)| n == k);
    (counts.iter())
        .map(|c| c.map(|(n, k)| if any_whole { n == k } else { n > 0 }))
        .collect()
}

/// Every face's hide bit matches `want` (where `want` calls it); returns how many faces end up
/// shown and hidden among those.
fn check(m: &PlainMesh, want: &[Option<bool>], what: &str) -> (usize, usize) {
    let mut kinds = (0, 0);
    for (i, (h, w)) in hidden_faces(m).into_iter().zip(want).enumerate() {
        let Some(w) = *w else { continue };
        assert_eq!(h, w, "{what}: face {i} at {:?}", corners(m, &m.faces[i]));
        if h { kinds.1 += 1 } else { kinds.0 += 1 }
    }
    kinds
}

#[test]
fn select_rect_shows_the_faces_wholly_under_it_and_hides_the_rest_or_with_alt_hides_them() {
    let poly = [(-0.5, -0.4), (0.6, -0.4), (0.6, 0.5), (-0.5, 0.5)];
    for invert in [false, true] {
        let mut m = store(&ico(4));
        let mut prm = params(None);
        prm.invert = invert;
        run(
            &mut m,
            &preset("select-rect").preset,
            prm,
            &rect(-0.5, -0.4, 0.6, 0.5),
        );
        let after = m.to_plain();
        let shown = smart(&inside_counts(&after, &poly, straight));
        let want: Vec<Option<bool>> = shown.iter().map(|s| s.map(|s| s == invert)).collect();
        let (vis, hid) = check(&after, &want, "rect");
        assert!(vis > 20 && hid > 20, "both kinds ({vis}, {hid})");
    }
}

#[test]
fn a_rectangle_that_holds_no_whole_face_takes_every_face_with_a_corner_inside() {
    let mesh = ico(4);
    // A tiny square around one front vertex: it and its mirror image behind it fall inside.
    let v = (mesh.positions.iter())
        .map(|p| Vec3::from_array(*p))
        .filter(|p| p.z > 0.5)
        .min_by(|a, b| (a.x.abs() + a.y.abs()).total_cmp(&(b.x.abs() + b.y.abs())))
        .unwrap_or(Vec3::ZERO);
    let e = 0.01;
    let poly = [
        (v.x - e, v.y - e),
        (v.x + e, v.y - e),
        (v.x + e, v.y + e),
        (v.x - e, v.y + e),
    ];
    let mut m = store(&mesh);
    run(
        &mut m,
        &preset("select-rect").preset,
        params(None),
        &rect(v.x - e, v.y - e, v.x + e, v.y + e),
    );
    let after = m.to_plain();
    let counts = inside_counts(&after, &poly, straight);
    assert!(
        counts.iter().flatten().all(|&(n, k)| n < k),
        "no face is wholly inside"
    );
    let want: Vec<Option<bool>> = smart(&counts).iter().map(|s| s.map(|s| !s)).collect();
    let (vis, hid) = check(&after, &want, "tiny rect");
    assert!(
        vis >= 10 && hid > 100,
        "the rings of both vertices stay ({vis}, {hid})"
    );
}

#[test]
fn a_second_rectangle_shows_faces_again() {
    let select = preset("select-rect").preset;
    let mut m = store(&ico(4));
    run(&mut m, &select, params(None), &rect(-0.2, -0.2, 0.2, 0.2));
    run(&mut m, &select, params(None), &rect(-0.7, -0.6, 0.6, 0.7));
    let poly = [(-0.7, -0.6), (0.6, -0.6), (0.6, 0.7), (-0.7, 0.7)];
    let after = m.to_plain();
    let want = smart(&inside_counts(&after, &poly, straight));
    let want: Vec<Option<bool>> = want.iter().map(|s| s.map(|s| !s)).collect();
    let (vis, _) = check(&after, &want, "second rect");
    assert!(
        vis > 100,
        "faces hidden by the first rectangle came back ({vis})"
    );
}

/// Review 0003 s2: hidden faces take no part in choosing the rule. A rectangle that holds only
/// a hidden face whole takes every face with a corner inside, so it keeps the visible faces
/// around that face rather than hiding them all.
#[test]
fn a_hidden_face_under_the_outline_does_not_choose_the_rule() {
    let mut mesh = grid(8);
    let cell =
        |c: &[Vec3]| (c.iter()).all(|p| (0.0..=0.25).contains(&p.x) && (0.0..=0.25).contains(&p.y));
    let flags: Vec<u32> = (mesh.faces.iter())
        .map(|f| u32::from(cell(&corners(&mesh, f))))
        .collect();
    assert_eq!(flags.iter().sum::<u32>(), 1, "one hidden cell");
    let hide = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(flags),
    };
    mesh.attributes.insert("hide".into(), hide);
    let mut m = store(&mesh);
    let select = preset("select-rect").preset;
    run(&mut m, &select, params(None), &rect(-0.05, -0.05, 0.3, 0.3));
    let after = m.to_plain();
    let poly = [(-0.05, -0.05), (0.3, -0.05), (0.3, 0.3), (-0.05, 0.3)];
    // No visible face is wholly inside, so a corner inside is enough.
    let counts = inside_counts(&after, &poly, straight);
    let want: Vec<Option<bool>> = counts.iter().map(|c| c.map(|(n, _)| n == 0)).collect();
    let (vis, hid) = check(&after, &want, "rect over a hidden cell");
    assert_eq!(
        (vis, hid),
        (9, 55),
        "the cell and its ring shown, the rest hidden"
    );
}

#[test]
fn a_slanted_outline_and_a_perspective_camera_use_the_same_rule() {
    let select = preset("select-rect").preset;
    let tri = [(-0.6, -0.5), (0.7, -0.3), (0.0, 0.6)];
    let mut m = store(&ico(4));
    run(&mut m, &lasso(), params(None), &outline(&tri));
    let after = m.to_plain();
    let want = smart(&inside_counts(&after, &tri, straight));
    let want: Vec<Option<bool>> = want.iter().map(|s| s.map(|s| !s)).collect();
    let (vis, hid) = check(&after, &want, "triangle");
    assert!(vis > 20 && hid > 20, "both kinds ({vis}, {hid})");

    let eye = Vec3::new(0.2, -0.1, 5.0);
    let mut prm = params(None);
    prm.view = View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: Some(eye),
    };
    let poly = [(-0.4, -0.3), (0.5, -0.3), (0.5, 0.45), (-0.4, 0.45)];
    let mut m = store(&ico(4));
    run(&mut m, &select, prm, &rect(-0.4, -0.3, 0.5, 0.45));
    // Carry each corner along the ray from the eye to the plane z = 2.
    let along = |p: Vec3| {
        let q = eye + (p - eye) * ((2.0 - eye.z) / (p.z - eye.z));
        (q.x, q.y)
    };
    let after = m.to_plain();
    let want = smart(&inside_counts(&after, &poly, along));
    let want: Vec<Option<bool>> = want.iter().map(|s| s.map(|s| !s)).collect();
    let (vis, hid) = check(&after, &want, "perspective");
    assert!(vis > 20 && hid > 20, "both kinds ({vis}, {hid})");
}

/// Each face as the sorted bit patterns of its corners, so faces compare across numberings.
fn key(m: &PlainMesh, f: &[u32; 4]) -> Vec<[u32; 3]> {
    let mut k: Vec<[u32; 3]> = corners(m, f).iter().map(|p| p.bits()).collect();
    k.sort_unstable();
    k
}

fn hidden_keys(m: &PlainMesh) -> BTreeSet<Vec<[u32; 3]>> {
    (m.faces.iter().zip(hidden_faces(m)))
        .filter(|f| f.1)
        .map(|(f, _)| key(m, f))
        .collect()
}

#[test]
fn with_mirror_symmetry_the_mirrored_outline_applies_too() {
    let poly = [(0.15, -0.3), (0.7, -0.3), (0.7, 0.4), (0.15, 0.4)];
    let mut m = store(&ico(4));
    run(
        &mut m,
        &preset("select-rect").preset,
        params(Some(mirror_x())),
        &rect(0.15, -0.3, 0.7, 0.4),
    );
    let after = m.to_plain();
    // 0 - x keeps points on the plane at +0, so their bits match.
    let flip = |p: Vec3| Vec3::new(0.0 - p.x, p.y, p.z);
    let shown = smart(&inside_counts(&after, &poly, straight));
    let mirrored = smart(&inside_counts(&after, &poly, |p| straight(flip(p))));
    let want: Vec<Option<bool>> = (shown.iter().zip(&mirrored))
        .map(|(a, b)| a.zip(*b).map(|(a, b)| !(a || b)))
        .collect();
    let (vis, _) = check(&after, &want, "mirrored rect");
    assert!(vis > 40, "both sides show ({vis})");
    let keys = hidden_keys(&after);
    for k in &keys {
        let mut mk: Vec<[u32; 3]> = k
            .iter()
            .map(|b| flip(Vec3::from_array(b.map(f32::from_bits))).bits())
            .collect();
        mk.sort_unstable();
        assert!(keys.contains(&mk), "the hidden set is symmetric");
    }
}

#[test]
fn renumbering_the_points_does_not_change_which_faces_a_gesture_hides() {
    let mesh = ico(4);
    let n = mesh.positions.len() as u32;
    // Reverse the vertex numbering and rotate each face's corners.
    let mut renumbered = mesh.clone();
    renumbered.positions.reverse();
    for f in &mut renumbered.faces {
        for v in f.iter_mut().filter(|v| **v != NO) {
            *v = n - 1 - *v;
        }
        f[..3].rotate_left(1);
    }
    let poly = [(-0.45, -0.35), (0.55, -0.25), (0.35, 0.5), (-0.4, 0.3)];
    let hide = |m: &PlainMesh| {
        let mut s = store(m);
        run(&mut s, &lasso(), params(None), &outline(&poly));
        hidden_keys(&s.to_plain())
    };
    let (a, b) = (hide(&mesh), hide(&renumbered));
    assert!(!a.is_empty(), "something is hidden");
    assert_eq!(a, b, "the same faces are hidden");
}

#[test]
fn hidden_points_never_move_and_points_with_a_visible_face_do() {
    let mut m = store(&ico(4));
    run(
        &mut m,
        &preset("select-rect").preset,
        params(None),
        &rect(-0.5, -0.4, 0.6, 0.5),
    );
    let before = m.to_plain();
    let top = |x: f32, y: f32| Vec3::new(x, y, 1.0).normalized();
    run(
        &mut m,
        &preset("draw").preset,
        params(None),
        &arc(top(0.6, -0.1), top(0.6, 0.3), 4, 0.3),
    );
    let after = m.to_plain();
    let mut only_hidden = vec![true; before.positions.len()];
    for (f, h) in before.faces.iter().zip(hidden_faces(&before)) {
        for &v in f.iter().filter(|&&v| v != NO) {
            only_hidden[v as usize] &= h;
        }
    }
    let mut border_moved = false;
    for (i, (b, a)) in before.positions.iter().zip(&after.positions).enumerate() {
        let moved = a.map(f32::to_bits) != b.map(f32::to_bits);
        assert!(
            !(moved && only_hidden[i]),
            "vertex {i} has only hidden faces and moved"
        );
        border_moved |= moved && !only_hidden[i];
    }
    assert!(border_moved, "vertices with a visible face still move");
    let mut m = store(&ico(4));
    let two = &outline(&[(-0.5, -0.4), (0.6, -0.4)]);
    let rep = run(&mut m, &lasso(), params(None), two);
    assert_eq!(rep.dabs, 0, "two points are not an outline");
}
