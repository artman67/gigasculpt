//! Smooth UVs (decision 0014, section 14): with them on, a divide places the UVs by
//! face-varying Catmull-Clark with island borders kept, checked against the slow f64 reference
//! and against the layout of the smoothed points (the rules are affine, so a flat layout stays
//! flat inside an island); island borders keep their lines bit for bit; lower levels show the
//! layout they had, also after a file round trip. With them off a divide is decision 0006's
//! linear rule, bit for bit.

use std::collections::BTreeMap;

use gs_core::{Levels, NO, Settings, subdivide_corners};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::SplitMix64;

use super::reference::{self, Mesh};
use super::{faces, positions, stack, stack_with};
use crate::common::{ok, small_nodes};
use crate::exact::rebuilt;

/// Grid width and height in quads; the seam between the two islands runs at x = W / 2.
const W: u32 = 16;
const H: u32 = 8;
const U: [&str; 4] = ["uv0.u", "uv1.u", "uv2.u", "uv3.u"];
const V: [&str; 4] = ["uv0.v", "uv1.v", "uv2.v", "uv3.v"];

/// One face's UVs, corner 0 to corner 3.
type Uvs = [[f32; 2]; 4];

pub(super) fn settings() -> Settings {
    Settings {
        corners: vec![U.map(String::from), V.map(String::from)],
        ..Settings::default()
    }
}

/// The flat layout: island 0 is the left half of the grid, island 1 the right half moved
/// apart. Every border line's coordinate is exact in f32.
fn layout(x: f32, y: f32, island: usize) -> [f32; 2] {
    [[0.0, 0.75][island] + x * 0.125, y * 0.125]
}

/// The island a UV is in.
fn island(uv: [f32; 2]) -> usize {
    usize::from(uv[0] > 0.625)
}

/// Each island's border lines: its `u` lines and its `v` lines.
fn borders(island: usize) -> ([f32; 2], [f32; 2]) {
    let (lo, hi) = (layout(0.0, 0.0, 0), layout((W / 2) as f32, H as f32, 0));
    let shift = [0.0, 1.25][island];
    ([lo[0] + shift, hi[0] + shift], [lo[1], hi[1]])
}

/// A `W` by `H` grid of quads in the plane, some split into triangles, its UVs laid out flat in
/// two islands with a seam down the middle (a triangle's unused corner holds its last).
pub(super) fn mesh(seed: u64) -> PlainMesh {
    let mut rng = SplitMix64::new(seed);
    let mut positions = Vec::new();
    for y in 0..=H {
        for x in 0..=W {
            let z = rng.below(1000) as f32 / 4000.0;
            positions.push([x as f32, y as f32, z]);
        }
    }
    let (mut faces, mut uv): (Vec<[u32; 4]>, Vec<Uvs>) = (Vec::new(), Vec::new());
    for y in 0..H {
        for x in 0..W {
            let a = y * (W + 1) + x;
            let (b, c, d) = (a + 1, a + W + 2, a + W + 1);
            let side = usize::from(x >= W / 2);
            let split = rng.below(3) == 0;
            let made = if split {
                vec![[a, b, c, NO], [a, c, d, NO]]
            } else {
                vec![[a, b, c, d]]
            };
            for f in made {
                let last = if f[3] == NO { f[2] } else { f[3] };
                let at = |v: u32| {
                    let p = positions[v as usize];
                    layout(p[0], p[1], side)
                };
                faces.push(f);
                uv.push([at(f[0]), at(f[1]), at(f[2]), at(last)]);
            }
        }
    }
    let mut attributes = BTreeMap::new();
    for (g, names) in [U, V].iter().enumerate() {
        for (k, name) in names.iter().enumerate() {
            let values = AttrValues::F32(uv.iter().map(|c| c[k][g]).collect());
            let domain = AttrDomain::Face;
            attributes.insert(name.to_string(), Attribute { domain, values });
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes,
    }
}

/// The active level's UVs, by canonical face.
pub(super) fn uvs(l: &Levels) -> Vec<Uvs> {
    let plain = l.numbering().apply(l.store().to_plain());
    let column = |name: &str| match &plain.attributes[name].values {
        AttrValues::F32(v) => v.clone(),
        AttrValues::U32(_) => panic!("{name} holds integers"),
    };
    let (u, v) = (U.map(column), V.map(column));
    (0..plain.faces.len())
        .map(|f| std::array::from_fn(|k| [u[k][f], v[k][f]]))
        .collect()
}

/// Every coordinate of `uv` through `f`: its bits, or its value read for the reference.
fn each<T>(uv: &[Uvs], f: fn(f32) -> T) -> Vec<[[T; 2]; 4]> {
    uv.iter().map(|c| c.map(|x| x.map(f))).collect()
}

fn bits(uv: &[Uvs]) -> Vec<[[u32; 2]; 4]> {
    each(uv, f32::to_bits)
}

fn reference_mesh(m: &PlainMesh) -> Mesh {
    Mesh {
        faces: m.faces.clone(),
        pos: (m.positions.iter()).map(|p| p.map(f64::from)).collect(),
        sharp: BTreeMap::new(),
    }
}

/// Two smooth-UV divides of `m` on node configuration `cfg`: the stack and each level's UVs
/// right after it was made.
fn two_divides(m: &PlainMesh, cfg: gs_core::NodeConfig) -> (Levels, [Vec<Uvs>; 3]) {
    let mut l = stack_with(m, &settings(), cfg);
    let base = uvs(&l);
    ok(l.divide_uvs(true, true));
    let first = uvs(&l);
    ok(l.divide_uvs(true, true));
    let top = uvs(&l);
    (l, [base, first, top])
}

#[test]
fn smooth_uvs_follow_face_varying_catmull_clark_with_island_borders_kept() {
    let m = mesh(11);
    let (_, [base, _, top]) = two_divides(&m, small_nodes());
    // The reference: two steps of face-varying Catmull-Clark with linear boundaries.
    let m0 = reference_mesh(&m);
    let m1 = reference::step(&m0, true, false);
    let uv1 = reference::uv_step(&m0, &each(&base, f64::from), true);
    let want = reference::uv_step(&m1, &uv1, true);
    assert_eq!(top.len(), want.len(), "face counts");
    let mut on_border = 0;
    for (f, (got, want)) in top.iter().zip(&want).enumerate() {
        for k in 0..4 {
            let (g, w) = (got[k], want[k]);
            let d = (0..2)
                .map(|i| (f64::from(g[i]) - w[i]).abs())
                .fold(0.0, f64::max);
            assert!(d <= 1e-5, "face {f} corner {k}: {g:?}, the reference {w:?}");
            // A corner the reference puts on a border line keeps the line's coordinate exactly.
            let (us, vs) = borders(island(g));
            for (i, lines) in [(0, us), (1, vs)] {
                for line in lines {
                    if (w[i] - f64::from(line)).abs() < 1e-9 {
                        on_border += 1;
                        assert_eq!(
                            g[i].to_bits(),
                            line.to_bits(),
                            "face {f} corner {k} leaves its island's border line {line}"
                        );
                    }
                }
            }
        }
    }
    assert!(on_border > 0, "the test reaches the island borders");
}

#[test]
fn a_flat_layout_stays_the_layout_of_the_smoothed_points_inside_its_islands() {
    let m = mesh(12);
    let (l, [_, _, top]) = two_divides(&m, small_nodes());
    let p = positions(&l);
    let mut inside = 0;
    for (f, face) in faces(&l).iter().enumerate() {
        for (k, &c) in face.iter().enumerate() {
            let (x, y) = (p[c as usize].x, p[c as usize].y);
            let side = island(top[f][k]);
            let left = [0.0, (W / 2) as f32][side];
            let margin = (x - left)
                .min(left + (W / 2) as f32 - x)
                .min(y)
                .min(H as f32 - y);
            if margin < 2.0 {
                continue;
            }
            inside += 1;
            let want = layout(x, y, side);
            let got = top[f][k];
            let d = (0..2).map(|i| (got[i] - want[i]).abs()).fold(0.0, f32::max);
            assert!(
                d <= 1e-5,
                "face {f} corner {k}: {got:?}, the layout {want:?}"
            );
        }
    }
    assert!(inside > 100, "the test reaches inside the islands");
}

#[test]
fn lower_levels_show_the_layout_they_had_also_after_a_file_round_trip() {
    let m = mesh(13);
    let (mut l, made) = two_divides(&m, small_nodes());
    for j in [0, 1, 2, 0] {
        ok(l.set(j));
        assert_eq!(bits(&uvs(&l)), bits(&made[j]), "level {j}'s UVs");
    }
    let placed = l.level_numbering(l.top_level()).placement();
    let mut reopened = ok(Levels::from_file(rebuilt(l.top()), placed, l.stored()));
    for j in [0, 1, 2] {
        ok(reopened.set(j));
        assert_eq!(
            bits(&uvs(&reopened)),
            bits(&made[j]),
            "reopened, level {j}'s UVs"
        );
    }
    // Deleting the levels above level 1 makes it the top with the layout it had.
    ok(l.set(1));
    assert!(l.delete_higher());
    ok(l.set(0));
    assert_eq!(bits(&uvs(&l)), bits(&made[0]), "level 0 under the new top");
    ok(l.set(1));
    assert_eq!(bits(&uvs(&l)), bits(&made[1]), "the new top");
}

#[test]
fn smooth_uvs_give_the_same_bits_on_another_node_cut_and_on_one_thread() {
    let m = mesh(14);
    let (_, [_, _, small]) = two_divides(&m, small_nodes());
    let (_, [_, _, default]) = two_divides(&m, gs_core::NodeConfig::default());
    let (_, [_, _, one]) = gs_par::with_threads(1, || two_divides(&m, small_nodes()));
    assert_eq!(bits(&small), bits(&default), "another node cut");
    assert_eq!(bits(&small), bits(&one), "one thread");
}

#[test]
fn with_smooth_uvs_off_a_divide_is_the_linear_rule_bit_for_bit() {
    let m = mesh(15);
    let mut l = stack(&m, &settings());
    let mut want = uvs(&l);
    let mut smooth = stack(&m, &settings());
    let mut plain = stack(&m, &settings());
    for _ in 0..2 {
        let arity = faces(&l);
        let mut next = Vec::new();
        let corners = |f: usize| if arity[f][3] == NO { 3 } else { 4 };
        subdivide_corners(&want, corners, |c| next.push(c));
        want = next;
        ok(l.divide_uvs(true, false));
        ok(smooth.divide_uvs(true, true));
        ok(plain.divide(true));
        assert_eq!(
            super::bits(&positions(&l)),
            super::bits(&positions(&smooth)),
            "smooth UVs leave the points alone"
        );
        assert_eq!(faces(&l), faces(&smooth), "and the faces");
    }
    assert_eq!(bits(&uvs(&l)), bits(&want), "the linear rule");
    assert_eq!(
        bits(&uvs(&plain)),
        bits(&want),
        "a plain divide is the same"
    );
}
