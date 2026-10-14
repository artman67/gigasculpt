//! Writing level 0's UVs into a level stack (decision 0061 as amended 2026-10-13, step UV5):
//! every level gets what dividing would have given it, bit for bit, kept sets included (wrong
//! result 8); three linear divides keep a hand-made layout's charts and flip no triangle (wrong
//! result 6); a wrong corner count is refused and changes nothing.

use std::collections::BTreeMap;

use gs_core::{AttrDef, AttrVec, Levels, NO, NewValues};
use gs_protocol::{AttrValues, PlainMesh, face_triangles};

use super::uvs::{mesh, settings, uvs};
use super::{faces, positions, stack};
use crate::common::ok;

/// One face's UVs, corner 0 to corner 3.
type Uvs = [[f32; 2]; 4];
type Bits = Vec<[[u32; 2]; 4]>;
/// The faces on one edge, each with the UV bits it gives the edge's two ends.
type Sides = Vec<(usize, [[u32; 2]; 2])>;

fn bits(uv: &[Uvs]) -> Bits {
    uv.iter().map(|c| c.map(|x| x.map(f32::to_bits))).collect()
}

/// `m`'s stack divided smooth twice, the UVs smoothed at the steps `smooth` names.
fn divided(m: &PlainMesh, smooth: [bool; 2]) -> Levels {
    let mut l = stack(m, &settings());
    for s in smooth {
        ok(l.divide_uvs(true, s));
    }
    l
}

/// Every level's UVs, the base first, then the level that was active made active again.
fn every_level(l: &mut Levels) -> Vec<Bits> {
    let active = l.active();
    let out = (0..l.count())
        .map(|j| {
            ok(l.set(j));
            bits(&uvs(l))
        })
        .collect();
    ok(l.set(active));
    out
}

/// `m` with every UV moved, so a write has something to replace: the same faces and islands.
fn moved(m: &PlainMesh) -> PlainMesh {
    let mut out = m.clone();
    for a in out.attributes.values_mut() {
        if let AttrValues::F32(v) = &mut a.values {
            for x in v.iter_mut() {
                *x = *x * 0.5 + 0.25;
            }
        }
    }
    out
}

#[test]
fn writing_level_0_gives_every_level_what_dividing_gave() {
    let m = mesh(11);
    for smooth in [[false, false], [true, true], [true, false]] {
        let mut want = divided(&m, smooth);
        let expected = every_level(&mut want);
        ok(want.set(0));
        let base = uvs(&want);
        for active in [2, 1] {
            let mut l = divided(&moved(&m), smooth);
            ok(l.set(active));
            // An edit on the active level not carried up yet, which the write must leave.
            let n = l.store().node(0).pos.len();
            let edit = vec![NewValues::Values(AttrVec::F32(vec![0.5; n]))];
            l.store_mut().put_column(AttrDef::mask(), &[0], edit);
            let pending = l.pending();
            ok(l.write_uvs(base.clone()));
            let what = format!("{smooth:?}, written at level {active}");
            assert_eq!(bits(&uvs(&l)), expected[active], "{what}: shown at once");
            assert_eq!(l.pending(), pending, "{what}: the edit is still pending");
            assert_eq!(every_level(&mut l), expected, "{what}");
        }
    }
}

/// A unit cube's level 0 and its layout in three charts, every face counter-clockwise in UV:
/// the four sides in one strip with a seam down one edge, the top split into two triangles
/// (a triangle's unused corner holds zeros), and the bottom.
fn cube() -> (PlainMesh, Vec<Uvs>) {
    let ring = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let positions = (0..8).map(|i| [ring[i % 4][0], ring[i % 4][1], (i / 4) as f32]);
    let mut faces = vec![[0, 3, 2, 1], [4, 5, 6, NO], [4, 6, 7, NO]];
    let square = |u: f32, v: f32| [[u, v], [u + 0.25, v], [u + 0.25, v + 0.25], [u, v + 0.25]];
    let top = square(0.0, 0.375);
    let mut layout = vec![
        square(0.5, 0.375),
        [top[0], top[1], top[2], [0.0; 2]],
        [top[0], top[2], top[3], [0.0; 2]],
    ];
    for i in 0..4 {
        faces.push([i, (i + 1) % 4, (i + 1) % 4 + 4, i + 4]);
        layout.push(square(i as f32 * 0.25, 0.0));
    }
    let mesh = PlainMesh {
        positions: positions.collect(),
        faces,
        attributes: BTreeMap::new(),
    };
    (mesh, layout)
}

/// The charts of `uv` over `faces`: faces joined across every edge they share whose two ends
/// they give the same UVs.
fn charts(faces: &[[u32; 4]], uv: &[Uvs]) -> usize {
    let n = |f: [u32; 4]| if f[3] == NO { 3 } else { 4 };
    let mut edges: BTreeMap<(u32, u32), Sides> = BTreeMap::new();
    for (fi, &f) in faces.iter().enumerate() {
        for k in 0..n(f) {
            let j = (k + 1) % n(f);
            let (a, b) = if f[k] < f[j] { (k, j) } else { (j, k) };
            let ends = [a, b].map(|c| uv[fi][c].map(f32::to_bits));
            edges.entry((f[a], f[b])).or_default().push((fi, ends));
        }
    }
    let mut root: Vec<usize> = (0..faces.len()).collect();
    for pair in edges.values().filter(|p| p.len() == 2 && p[0].1 == p[1].1) {
        let (a, b) = (find(&root, pair[0].0), find(&root, pair[1].0));
        root[a.max(b)] = a.min(b);
    }
    (0..faces.len()).filter(|&f| find(&root, f) == f).count()
}

/// The root of `x`'s set.
fn find(root: &[usize], mut x: usize) -> usize {
    while root[x] != x {
        x = root[x];
    }
    x
}

#[test]
fn three_linear_divides_keep_the_charts_and_flip_nothing() {
    let (m, layout) = cube();
    assert_eq!(charts(&m.faces, &layout), 3);
    let mut l = stack(&m, &settings());
    for _ in 0..3 {
        ok(l.divide(true));
    }
    ok(l.write_uvs(layout));
    let (top, uv, pos) = (faces(&l), uvs(&l), positions(&l));
    assert_eq!(top.len(), 7 * 4 * 4 * 4 - 2 * 4 * 4, "every face divided");
    assert_eq!(charts(&top, &uv), 3, "the charts are kept");
    for (fi, &f) in top.iter().enumerate() {
        for t in face_triangles(f, |k| pos[f[k] as usize].to_array()) {
            let [a, b, c] = t.map(|k| uv[fi][k]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(area > 0.0, "face {fi} triangle {t:?} flipped: {area}");
        }
    }
}

#[test]
fn a_corner_count_other_than_level_0s_faces_is_refused_and_changes_nothing() {
    let mut l = divided(&mesh(3), [true, false]);
    let before = every_level(&mut l);
    ok(l.set(0));
    let mut short = uvs(&l);
    short.pop();
    ok(l.set(2));
    assert!(l.write_uvs(short).is_err());
    assert_eq!(every_level(&mut l), before);
}
