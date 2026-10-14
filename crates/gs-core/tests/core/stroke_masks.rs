//! The influence stage and its neighbours: automasks, radial symmetry, the axis lock, the
//! mirror-plane clamp, feathering, and painting face groups and colors. Alphas have their own
//! tests (`alphas`).

use std::sync::Arc;

use gs_core::alpha::{Picture, Pyramid};
use gs_core::symmetry::Symmetry;
use gs_core::{BrushPreset, StrokeParams, Vec3, View};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::fixtures;

use crate::brush_kit::{
    BLUE, RED, after, at, floats, ico, masked, mirror_x, params, preset, run, store,
};
use crate::common::ok;
use crate::paint_stroke::{decode, encode};

/// The Draw preset with extra tables appended (later keys win over none: Draw has no
/// `[automask]`).
fn draw_plus(extra: &str) -> BrushPreset {
    ok(BrushPreset::from_toml(&format!(
        "{}\n{extra}\n",
        preset("draw").text
    )))
}

/// Whether `h` is half of `f`, up to the rounding of `f32` positions near 1.
fn halved(h: f32, f: f32) -> bool {
    (h - 0.5 * f).abs() <= 1e-5 * f + 3e-7
}

pub(crate) fn moved(a: &PlainMesh, b: &PlainMesh, i: usize) -> f32 {
    (Vec3::from_array(a.positions[i]) - Vec3::from_array(b.positions[i]))
        .len2()
        .sqrt()
}

fn looking(dir: Vec3) -> StrokeParams {
    let mut p = params(None);
    p.view = View { dir, eye: None };
    p
}

#[test]
fn backface_automask_spares_vertices_facing_away() {
    let mesh = store(&ico(4)).to_plain();
    let s = [at(Vec3::new(0.0, 0.0, 1.0), 1.8)];
    let free = after(&mesh, &draw_plus(""), params(None), &s);
    let half = after(
        &mesh,
        &draw_plus("[automask]\nbackface = 0.5"),
        params(None),
        &s,
    );
    let full = after(
        &mesh,
        &draw_plus("[automask]\nbackface = 1.0"),
        params(None),
        &s,
    );
    let mut back = 0;
    for (i, p) in mesh.positions.iter().enumerate() {
        let (f, h, g) = (
            moved(&free, &mesh, i),
            moved(&half, &mesh, i),
            moved(&full, &mesh, i),
        );
        if p[2] < -0.1 && f > 0.0 {
            back += 1;
            assert!(g == 0.0, "vertex {i} faces away and moved");
            assert!(halved(h, f), "half spared: {h} of {f}");
        }
        if p[2] > 0.3 {
            assert_eq!(f.to_bits(), g.to_bits(), "vertex {i} faces the camera");
        }
    }
    assert!(back > 10, "the brush reached around the back ({back})");
}

/// Two icospheres side by side, apart.
fn pair() -> PlainMesh {
    let a = fixtures::icosphere(3);
    let mut m = a.clone();
    let n = a.positions.len() as u32;
    m.positions
        .extend(a.positions.iter().map(|p| [p[0] + 2.2, p[1], p[2]]));
    m.faces.extend(
        a.faces
            .iter()
            .map(|f| f.map(|v| if v == gs_core::NO { v } else { v + n })),
    );
    m
}

#[test]
fn topology_automask_keeps_to_what_is_joined_to_the_start() {
    let mesh = store(&pair()).to_plain();
    let s = [at(Vec3::new(1.0, 0.0, 0.0), 1.5)];
    let view = looking(Vec3::new(-1.0, 0.0, 0.0));
    let free = after(&mesh, &draw_plus(""), view.clone(), &s);
    let kept = after(
        &mesh,
        &draw_plus("[automask]\ntopology = 4.0"),
        view.clone(),
        &s,
    );
    let short = after(&mesh, &draw_plus("[automask]\ntopology = 0.2"), view, &s);
    let (mut other, mut far) = (false, false);
    for (i, p) in mesh.positions.iter().enumerate() {
        let (f, k, sh) = (
            moved(&free, &mesh, i),
            moved(&kept, &mesh, i),
            moved(&short, &mesh, i),
        );
        if p[0] > 1.1 {
            other |= f > 0.0;
            assert!(k == 0.0, "vertex {i} of the other sphere moved");
        } else {
            assert_eq!(
                f.to_bits(),
                k.to_bits(),
                "vertex {i} is joined to the start"
            );
            let d = (Vec3::from_array(*p) - Vec3::new(1.0, 0.0, 0.0))
                .len2()
                .sqrt();
            if d > 0.5 && f > 0.0 {
                far = true;
                assert!(
                    sh == 0.0,
                    "vertex {i} is past a short range along the edges"
                );
            }
        }
    }
    assert!(
        other && far,
        "the brush reached the other sphere and past the short range"
    );
}

#[test]
fn face_group_automask_keeps_to_the_group_under_the_start() {
    let mut mesh = store(&ico(4)).to_plain();
    let center = |f: &[u32; 4]| {
        f.iter()
            .take(3)
            .map(|&v| mesh.positions[v as usize][0])
            .sum::<f32>()
    };
    let groups: Vec<u32> = mesh
        .faces
        .iter()
        .map(|f| u32::from(center(f) > 0.0))
        .collect();
    mesh.attributes.insert(
        "group".into(),
        Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::U32(groups.clone()),
        },
    );
    let s = [at(Vec3::new(-0.25, 0.0, 0.97).normalized(), 0.6)];
    let free = after(&mesh, &draw_plus(""), params(None), &s);
    let kept = after(
        &mesh,
        &draw_plus("[automask]\nface_group = 1.0"),
        params(None),
        &s,
    );
    let half = after(
        &mesh,
        &draw_plus("[automask]\nface_group = 0.5"),
        params(None),
        &s,
    );
    let mut only_other = vec![true; mesh.positions.len()];
    for (f, g) in mesh.faces.iter().zip(&groups) {
        for &v in f.iter().filter(|&&v| v != gs_core::NO) {
            only_other[v as usize] &= *g == 1;
        }
    }
    let mut spared = 0;
    for (i, &other) in only_other.iter().enumerate() {
        let (f, k, h) = (
            moved(&free, &mesh, i),
            moved(&kept, &mesh, i),
            moved(&half, &mesh, i),
        );
        if other && f > 0.0 {
            spared += 1;
            assert!(k == 0.0, "vertex {i} is only in the other group and moved");
            assert!(halved(h, f), "half spared: {h} of {f}");
        } else {
            assert_eq!(
                f.to_bits(),
                k.to_bits(),
                "vertex {i} touches the start group"
            );
        }
    }
    assert!(spared > 10, "the brush reached the other group ({spared})");
}

#[test]
fn cavity_automask_spares_hollows_or_peaks() {
    let mut mesh = store(&ico(4)).to_plain();
    let top = (0..mesh.positions.len())
        .max_by(|&a, &b| mesh.positions[a][2].total_cmp(&mesh.positions[b][2]));
    let dent = top.unwrap_or_default();
    let p0 = Vec3::from_array(mesh.positions[dent]);
    mesh.positions[dent] = (p0 * 0.92).to_array();
    // Moving a vertex changes the storage order; take the order the store gives it.
    let mesh = store(&mesh).to_plain();
    let dent = (0..mesh.positions.len())
        .min_by(|&a, &b| {
            let d = |i: usize| (Vec3::from_array(mesh.positions[i]) - p0 * 0.92).len2();
            d(a).total_cmp(&d(b))
        })
        .unwrap_or_default();
    let ring = (0..mesh.positions.len()).find(|&i| {
        let d = (Vec3::from_array(mesh.positions[i]) - p0).len2().sqrt();
        d > 0.18 && d < 0.22
    });
    let peak = ring.unwrap_or_default();
    let s = [at(p0, 0.4)];
    let plain = after(&mesh, &draw_plus(""), params(None), &s);
    let hollows = after(
        &mesh,
        &draw_plus("[automask]\ncavity = 1.0"),
        params(None),
        &s,
    );
    let peaks = after(
        &mesh,
        &draw_plus("[automask]\ncavity = -1.0"),
        params(None),
        &s,
    );
    let m = |a: &PlainMesh, i| moved(a, &mesh, i);
    assert!(
        m(&hollows, dent) < 0.6 * m(&plain, dent),
        "the hollow is spared"
    );
    assert_eq!(
        m(&peaks, dent).to_bits(),
        m(&plain, dent).to_bits(),
        "the hollow is not a peak"
    );
    // Every point of a sphere is a peak, past full strength at full intensity (decision 0056).
    assert!(
        m(&plain, peak) > 0.0 && m(&peaks, peak) == 0.0,
        "peaks are spared"
    );
    assert_eq!(
        m(&hollows, peak).to_bits(),
        m(&plain, peak).to_bits(),
        "a peak is not a hollow"
    );
}

#[test]
fn radial_symmetry_repeats_the_stroke_around_its_axis() {
    let mesh = store(&fixtures::cube(8)).to_plain();
    let mut prm = looking(Vec3::new(-1.0, 0.0, 0.0));
    prm.symmetry = Symmetry {
        radial: Some((2, 4)),
        ..Symmetry::default()
    };
    let draw = draw_plus("");
    let out = after(
        &mesh,
        &draw,
        prm,
        &[
            at(Vec3::new(1.0, 0.3, 0.2), 0.3),
            at(Vec3::new(1.0, 0.5, 0.1), 0.3),
        ],
    );
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let index: std::collections::BTreeMap<[u32; 3], usize> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, &p)| (key(p), i))
        .collect();
    let turn = |p: [f32; 3]| [-p[1] + 0.0, p[0], p[2]];
    let mut worst = 0.0f32;
    for (i, &p) in mesh.positions.iter().enumerate() {
        let j = index
            .get(&key(turn(p)))
            .copied()
            .unwrap_or_else(|| panic!("no turned twin of {p:?}"));
        let want = Vec3::from_array(turn(out.positions[i]));
        worst = worst.max((Vec3::from_array(out.positions[j]) - want).len2().sqrt());
    }
    assert!(
        crate::brush_kit::max_diff(&out, &mesh) > 0.01,
        "the stroke moved the cube"
    );
    assert!(
        worst < 1e-5,
        "four copies a quarter turn apart: off by {worst}"
    );
}

#[test]
fn the_axis_lock_and_the_mirror_plane_clamp() {
    let mesh = store(&ico(4)).to_plain();
    let draw = draw_plus("");
    let mut prm = params(None);
    prm.lock = [true, false, false];
    let path = crate::brush_kit::arc(Vec3::new(0.3, 0.2, 1.0), Vec3::new(-0.2, 0.3, 1.0), 4, 0.3);
    let locked = after(&mesh, &draw, prm, &path);
    for (a, b) in locked.positions.iter().zip(&mesh.positions) {
        assert_eq!(a[0].to_bits(), b[0].to_bits(), "x is locked");
    }
    assert!(
        crate::brush_kit::max_diff(&locked, &mesh) > 0.01,
        "the rest moved"
    );
    let mirrored = after(&mesh, &draw, params(Some(mirror_x())), &path);
    for (a, b) in mirrored.positions.iter().zip(&mesh.positions) {
        if b[0] == 0.0 {
            assert!(a[0] == 0.0, "a vertex on the mirror plane stays on it");
        }
    }
    let grab = preset("move").preset;
    let g = Vec3::new(0.0, 0.3, 0.95).normalized();
    let prm = params(Some(Symmetry {
        clip: 0.08,
        ..mirror_x()
    }));
    let dragged = after(
        &mesh,
        &grab,
        prm,
        &[at(g, 0.4), at(g + Vec3::new(0.15, 0.1, 0.0), 0.4)],
    );
    let (mut clamped, mut free) = (0, 0);
    for (a, b) in dragged.positions.iter().zip(&mesh.positions) {
        if a.map(f32::to_bits) == b.map(f32::to_bits) {
            continue;
        }
        if b[0].abs() <= 0.08 {
            assert!(
                a[0] == 0.0,
                "{b:?} is near the plane and lands on it: {a:?}"
            );
            clamped += 1;
        } else {
            free += 1;
        }
    }
    assert!(
        clamped > 3 && free > 3,
        "both kinds moved ({clamped}, {free})"
    );
}

#[test]
fn feathering_keeps_a_stroke_on_the_mirror_plane_from_counting_twice() {
    let mesh = store(&ico(4)).to_plain();
    let on_plane = (0..mesh.positions.len())
        .find(|&i| mesh.positions[i][0] == 0.0 && mesh.positions[i][2] > 0.5);
    let i = on_plane.unwrap_or_default();
    let s = [at(Vec3::from_array(mesh.positions[i]), 0.3)];
    let draw = draw_plus("");
    let one = after(&mesh, &draw, params(None), &s);
    let feathered = after(&mesh, &draw, params(Some(mirror_x())), &s);
    let doubled = after(
        &mesh,
        &draw,
        params(Some(Symmetry {
            feather: false,
            ..mirror_x()
        })),
        &s,
    );
    let (a, f, d) = (
        moved(&one, &mesh, i),
        moved(&feathered, &mesh, i),
        moved(&doubled, &mesh, i),
    );
    assert!(
        (f - a).abs() < 1e-3 * a,
        "feathered: like one pass ({f} vs {a})"
    );
    assert!(
        (d - 2.0 * a).abs() < 1e-3 * a,
        "without feathering: twice ({d} vs {a})"
    );
}

/// The Mask Pen preset painting `column` with `value` instead.
fn painter(column: &str, value: &str) -> BrushPreset {
    let text = preset("mask-pen")
        .text
        .replace("column = \"mask\"", &format!("column = \"{column}\""))
        .replace("value = 1.0", &format!("value = {value}"));
    ok(BrushPreset::from_toml(&text))
}

/// The shipped Paint brush: strength 1, tap 0.5, the Mask Pen's gain of 8 on the smooth tip.
fn paint() -> BrushPreset {
    preset("paint-draw").preset
}

fn labels(m: &PlainMesh, name: &str) -> Vec<u32> {
    match m.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.clone(),
        _ => vec![0; m.faces.len().max(m.positions.len())],
    }
}

#[test]
fn face_groups_paint_where_the_dab_is_strong_and_skip_hidden_faces() {
    let mesh = store(&ico(4)).to_plain();
    let g = Vec3::new(0.2, 0.1, 1.0).normalized();
    let out = after(&mesh, &painter("group", "3"), params(None), &[at(g, 0.4)]);
    let groups = labels(&out, "group");
    let center = |f: &[u32; 4]| {
        f.iter().take(3).fold(Vec3::ZERO, |s, &v| {
            s + Vec3::from_array(mesh.positions[v as usize])
        }) * (1.0 / 3.0)
    };
    let (mut painted, mut left) = (0, 0);
    for (f, &grp) in mesh.faces.iter().zip(&groups) {
        let d = (center(f) - g).len2().sqrt() / 0.4;
        if d < 0.55 {
            assert_eq!(grp, 3, "a face {d} radii out takes the group");
            painted += 1;
        } else if d > 0.65 {
            assert_eq!(grp, 0, "a face {d} radii out keeps its group");
            left += 1;
        }
    }
    assert!(
        painted > 5 && left > 5,
        "both kinds of face ({painted}, {left})"
    );
    let mut prm = params(None);
    prm.invert = true;
    let erased = after(&out, &painter("group", "3"), prm, &[at(g, 0.4)]);
    assert!(
        labels(&erased, "group").iter().all(|&x| x == 0),
        "inverted, it paints group 0"
    );
    let rect = preset("select-rect").preset;
    let mut hidden = store(&mesh);
    let corners = [(-0.1, -0.1), (0.1, 0.1)].map(|(x, y)| at(Vec3::new(x, y, 2.0), 0.1));
    run(&mut hidden, &rect, params(None), &corners);
    run(
        &mut hidden,
        &painter("group", "3"),
        params(None),
        &[at(g, 0.4)],
    );
    let after_hide = hidden.to_plain();
    for (h, grp) in labels(&after_hide, "hide")
        .iter()
        .zip(labels(&after_hide, "group"))
    {
        assert!(!(*h == 1 && grp == 3), "a hidden face was painted");
    }
}

/// A color stroke's one dab: at `g`, radius 0.4, on the smooth tip.
const DAB: f32 = 0.4;

/// The smooth tip's falloff at `d` radii: `3d^4 - 4d^3 + 1`, which is 0 from one radius out.
fn bell(d: f64) -> f64 {
    let d = d.min(1.0);
    (3.0 * d - 4.0) * d * d * d + 1.0
}

/// Channel `k` of color `c`, in levels.
fn ch(c: u32, k: u32) -> f64 {
    f64::from(c >> (8 * k) & 255)
}

/// The stroke's end rounds with a dither: within a level of the exact value.
fn near(c: u32, k: u32, want: f64) -> bool {
    (ch(c, k) - want).abs() < 1.0
}

#[test]
fn colors_mix_toward_main_and_with_alt_toward_secondary_in_linear_light() {
    // A mask in steps of 0.25, so a partial mask counts too.
    let mut mesh = store(&ico(4)).to_plain();
    let mask = (0..mesh.positions.len())
        .map(|i| (i % 5) as f32 * 0.25)
        .collect();
    mesh.attributes.insert(
        "mask".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(mask),
        },
    );
    let g = Vec3::new(0.2, 0.1, 1.0).normalized();
    let mask = floats(&mesh, "mask");
    // gain 8 * w * amount (strength 1 * tap 0.5).
    let frac = |i: usize| {
        let d = f64::from((Vec3::from_array(mesh.positions[i]) - g).len2().sqrt()) / 0.4;
        1.0 - libm::exp(-4.0 * bell(d) * f64::from(1.0 - mask[i]))
    };
    let mut prm = params(None);
    prm.colors.main = RED;
    prm.colors.secondary = BLUE;
    let out = after(&mesh, &paint(), prm.clone(), &[at(g, DAB)]);
    let colors = labels(&out, "color");
    let mut checked = 0;
    for (i, &c) in colors.iter().enumerate() {
        // From white, the column's empty value (decision 0039): green and blue fall to 0 in
        // linear light.
        let want = encode(1.0 - frac(i));
        assert!(
            near(c, 1, want) && near(c, 2, want),
            "vertex {i}: {c:08x} for {want}"
        );
        assert_eq!(c | 0x00ff_ff00, u32::MAX, "red and alpha stay 255");
        checked += usize::from(want < 245.0 && mask[i] > 0.0);
    }
    assert!(
        checked > 5,
        "the dab colored partly masked vertices ({checked})"
    );
    prm.invert = true;
    let alt = labels(&after(&out, &paint(), prm, &[at(g, DAB)]), "color");
    // Alt paints the Secondary color, opaque (it painted transparent black, then black).
    for (i, (&now, &was)) in alt.iter().zip(&colors).enumerate() {
        for k in 0..3 {
            let (from, to) = (decode(ch(was, k)), decode(ch(BLUE, k)));
            let want = encode(from + (to - from) * frac(i));
            assert!(
                near(now, k, want),
                "vertex {i} channel {k}: {} for {want}",
                ch(now, k)
            );
        }
        assert_eq!(now >> 24, 255, "alpha stays 255");
    }
}

#[test]
fn gradient_paints_main_at_the_centre_and_secondary_at_the_rim() {
    let mesh = store(&ico(4)).to_plain();
    let g = Vec3::new(0.2, 0.1, 1.0).normalized();
    let mut prm = params(None);
    prm.colors.main = RED;
    prm.colors.secondary = BLUE;
    prm.colors.gradient = true;
    let out = after(&mesh, &paint(), prm, &[at(g, DAB)]);
    let colors = labels(&out, "color");
    let mut between = 0;
    for (i, &c) in colors.iter().enumerate() {
        let d = f64::from((Vec3::from_array(mesh.positions[i]) - g).len2().sqrt()) / 0.4;
        let w = bell(d);
        let frac = 1.0 - libm::exp(-4.0 * w);
        for k in 0..3 {
            // Secondary mixed toward Main by the falloff, in linear light, then painted from
            // white by the dab's coverage.
            let (main, rim) = (decode(ch(RED, k)), decode(ch(BLUE, k)));
            let target = rim + (main - rim) * w;
            let want = encode(1.0 + (target - 1.0) * frac);
            assert!(
                near(c, k, want),
                "vertex {i} channel {k}: {} for {want}",
                ch(c, k)
            );
        }
        between += usize::from((0.2..0.8).contains(&w));
    }
    assert!(
        between > 5,
        "the dab's rim is wide enough to show ({between})"
    );
}

#[test]
fn every_option_stays_symmetric_under_a_mirror() {
    let mesh = store(&ico(4)).to_plain();
    let pair = crate::brush_kit::mirror_pairs(&mesh);
    let path = crate::brush_kit::arc(
        Vec3::new(0.35, 0.2, 1.0),
        Vec3::new(-0.1, 0.35, 1.0),
        6,
        0.3,
    );
    let terms = |t: &str| format!("[kernel]\ntype = \"offset\"\nterms = [{t}]");
    let head = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.5\n";
    // An alpha that differs along both of its axes, raising and carving (mid value 0.3), for
    // the last variant.
    let lopsided = ok(Picture::from_unit(
        3,
        3,
        &[0.1, 0.9, 0.3, 1.0, 0.2, 0.6, 0.4, 0.8, 0.0],
    ));
    let variants = [
        terms("{ dir = \"view\", gain = 0.5 }, { dir = \"x\", gain = 0.3 }"),
        terms("{ dir = \"area_normal\", gain = 0.5 }") + "\n[automask]\nbackface = 0.7",
        terms("{ dir = \"area_normal\", gain = 0.1 }")
            + "\n[post]\ngravity = 0.8\ngravity_dir = [1, -1, 0]",
        terms("{ dir = \"area_normal\", gain = 0.5 }")
            + &format!(
                "\n[tip.alpha]\nimage = \"blake3:{}\"\nmid = 0.3",
                lopsided.hash()
            ),
    ];
    for (i, v) in variants.iter().enumerate() {
        let p = ok(BrushPreset::from_toml(&format!("{head}{v}\n")));
        let mut prm = looking(Vec3::new(0.3, 0.1, -1.0).normalized());
        prm.symmetry = mirror_x();
        prm.alpha = (p.tip.alpha.as_ref()).map(|a| Arc::new(Pyramid::build(&lopsided, a)));
        let out = after(&mesh, &p, prm, &path);
        assert!(
            crate::brush_kit::max_diff(&out, &mesh) > 1e-3,
            "variant {i} moved something"
        );
        for (a, &b) in pair.iter().enumerate() {
            let [x, y, z] = out.positions[a];
            let off = (Vec3::from_array(out.positions[b]) - Vec3::new(-x, y, z))
                .len2()
                .sqrt();
            assert!(
                off < 2e-6,
                "variant {i}: vertex {a} is off its mirror by {off}"
            );
        }
    }
}

#[test]
fn plane_caps_include_their_bound() {
    // One vertex sampled at the dab point makes the normal exactly +z and the center exactly
    // that vertex, so the plane sits exactly 0.2 radii off the grid: right at the cap.
    let mesh = store(&crate::brush_kit::grid(20)).to_plain();
    let plane = |more: &str, imbed: f32| {
        let text = format!(
            "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\ntype = \"plane\"\ngain = 4.0\n{more}\n[depth]\nimbed = {imbed}\n[sampling]\nnormal_radius = 0.01\nposition_radius = 0.01\n"
        );
        ok(BrushPreset::from_toml(&text))
    };
    let s = [at(Vec3::ZERO, 0.5)];
    for (more, imbed) in [
        ("side = \"below\"\ndepth = 0.2", 0.2),
        ("side = \"above\"\nheight = 0.2", -0.2),
    ] {
        let out = after(&mesh, &plane(more, imbed), params(None), &s);
        assert!(
            crate::brush_kit::max_diff(&out, &mesh) > 0.01,
            "a vertex at the cap moves: {more}"
        );
    }
}

#[test]
fn backface_spares_only_what_faces_away_not_what_is_edge_on() {
    // Seen exactly edge-on, the grid faces neither way, so the automask leaves it alone.
    let mesh = store(&crate::brush_kit::grid(20)).to_plain();
    let s = [at(Vec3::ZERO, 0.5)];
    let edge_on = looking(Vec3::new(1.0, 0.0, 0.0));
    let free = after(&mesh, &draw_plus(""), edge_on.clone(), &s);
    let masked = after(&mesh, &draw_plus("[automask]\nbackface = 1.0"), edge_on, &s);
    assert!(
        crate::brush_kit::max_diff(&free, &mesh) > 0.01,
        "the dab moved the grid"
    );
    assert!(
        crate::brush_kit::max_diff(&free, &masked) == 0.0,
        "edge-on is not facing away"
    );
}

/// The cavity intensity is the measure's gain (decision 0056, section 3). Where full intensity
/// holds a point only partly, half spares half as much, the mean of none and full, as a blend
/// would. Where full intensity holds it fully, half spares at least half, and the deepest
/// hollows (the highest peaks at a negative intensity) are still held fully, which a blend
/// would only halve.
#[test]
fn cavity_intensity_is_the_measures_gain() {
    let (mesh, a) = crate::automask_checks::bumpy();
    let s = [at(Vec3::new(0.0, 0.0, a), 0.6)];
    let run_with = |c: &str| {
        after(
            &mesh,
            &draw_plus(&format!("[automask]\ncavity = {c}")),
            params(None),
            &s,
        )
    };
    let zero = after(&mesh, &draw_plus(""), params(None), &s);
    for (half, full) in [("0.5", "1.0"), ("-0.5", "-1.0")] {
        let (half, full) = (run_with(half), run_with(full));
        let (mut partly, mut fully, mut still) = (0, 0, 0);
        for i in 0..mesh.positions.len() {
            let (z, h, f) = (
                moved(&zero, &mesh, i),
                moved(&half, &mesh, i),
                moved(&full, &mesh, i),
            );
            if f > 0.0 {
                assert!(
                    (h - f32::midpoint(z, f)).abs() <= 1e-5 * z + 3e-7,
                    "vertex {i}: {h} not between {z} and {f}"
                );
                partly += usize::from(f < 0.9 * z);
            } else if z > 0.0 {
                assert!(
                    h <= 0.5 * z + 3e-7,
                    "vertex {i}: half spares less than half"
                );
                fully += 1;
                still += usize::from(h == 0.0);
            }
        }
        assert!(
            partly >= 3 && fully >= 3 && still >= 3,
            "{partly} held partly and {fully} fully, {still} of them still at half"
        );
    }
}

#[test]
fn the_topology_automask_also_limits_a_connected_move() {
    let mesh = store(&ico(4)).to_plain();
    let g = Vec3::new(0.3, 0.2, 1.0).normalized();
    let s = [at(g, 0.4), at(g + Vec3::new(0.05, 0.08, 0.0), 0.4)];
    let grab = preset("move-connected").text;
    let plain = after(&mesh, &ok(BrushPreset::from_toml(&grab)), params(None), &s);
    let short = ok(BrushPreset::from_toml(&format!("{grab}topology = 0.5\n")));
    let limited = after(&mesh, &short, params(None), &s);
    let mut spared = 0;
    for (i, p) in mesh.positions.iter().enumerate() {
        let d = (Vec3::from_array(*p) - g).len2().sqrt();
        if d > 0.25 && moved(&plain, &mesh, i) > 0.0 {
            spared += 1;
            assert!(
                moved(&limited, &mesh, i) == 0.0,
                "vertex {i} is past half a radius along the edges"
            );
        }
    }
    assert!(
        spared > 5,
        "the connected move reached past half a radius ({spared})"
    );
}

/// Review 0002 M5: a mask value that is not a number masks fully. Import refuses such values,
/// so this one is written after import, as any later command could.
#[test]
fn a_mask_value_that_is_not_a_number_holds_its_vertex() {
    let mesh = masked(&ico(4));
    let mut m = store(&mesh);
    let mask = gs_core::ColumnSet::attr(m.attr_id("mask").unwrap());
    let g = Vec3::new(0.0, 0.0, 1.0);
    let q = m.query(g, 0.3);
    let id = m.attr_id("mask").unwrap();
    m.write(&q.brushed, mask).each(|n| {
        if let gs_core::AttrVec::F32(v) = n.attr(id) {
            v.fill(f32::NAN);
        }
    });
    let before = m.to_plain();
    run(
        &mut m,
        &preset("draw").preset,
        params(None),
        &[at(g, 0.3), at(g + Vec3::new(0.05, 0.0, 0.0), 0.3)],
    );
    let after = m.to_plain();
    assert_eq!(
        after.positions.len(),
        before.positions.len(),
        "no vertex died"
    );
    assert!(
        after.positions == before.positions,
        "every NaN-masked vertex held still"
    );
}
