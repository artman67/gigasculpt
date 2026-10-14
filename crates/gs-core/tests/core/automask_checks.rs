//! Catalog `brush-settings.auto-masking`, its Check clause by clause: the backface automask
//! with the Mask Pen on a thin sheet and with Move Connected, the Move family shipping with it
//! off, Live Detail refusing a brush whose automask limits where it reaches (decision 0021,
//! section 3, amended 2026-10-12), and the cavity automask holding a bumpy plane's hollows on
//! the measure the cavity mask maker reads too (decision 0056).

use gs_core::brush::Family;
use gs_core::live::{self, Automasked, LiveDetail, NotRefined, Target};
use gs_core::mask::{MaskMaker, NewMask};
use gs_core::{BrushPreset, Stroke, StrokeParams, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::fixtures;

use crate::brush_kit::{after, at, floats, grid, ico, params, preset, run, shipped, store};
use crate::common::ok;
use crate::stroke_masks::moved;

/// The shipped preset `stem` with `extra` appended (it has no `[automask]` table of its own).
pub(crate) fn plus(stem: &str, extra: &str) -> BrushPreset {
    ok(BrushPreset::from_toml(&format!(
        "{}\n{extra}\n",
        preset(stem).text
    )))
}

/// A sheet 16 units wide and 1 thick, its front facing +z at z = 0.5: the cube surface
/// stretched, so the back is a closed surface joined to the front by its rims.
fn sheet() -> PlainMesh {
    let mut m = fixtures::cube(32);
    for p in &mut m.positions {
        *p = [p[0] * 8.0, p[1] * 8.0, p[2] * 0.5];
    }
    store(&m).to_plain()
}

/// "Backface Mask with Mask Pen on a 1-unit sheet and a 5-unit brush masks only the front": a
/// dab of radius 5 on the front reaches through to the back, which it masks too unless the
/// backface automask is on.
#[test]
fn backface_with_the_mask_pen_masks_only_the_front_of_a_thin_sheet() {
    let mesh = sheet();
    let dab = [at(Vec3::new(0.0, 0.0, 0.5), 5.0)];
    let pen = preset("mask-pen").text;
    let masks = |text: &str| {
        floats(
            &after(&mesh, &ok(BrushPreset::from_toml(text)), params(None), &dab),
            "mask",
        )
    };
    let leaks = masks(&pen);
    let front_only = masks(&format!("{pen}\n[automask]\nbackface = 1.0\n"));
    let back = |i: usize| mesh.positions[i][2] < 0.0;
    let reached = (0..mesh.positions.len()).filter(|&i| back(i) && leaks[i] > 0.0);
    assert!(
        reached.count() > 50,
        "without the automask the dab masks the back too"
    );
    let mut front = 0;
    for (i, p) in mesh.positions.iter().enumerate() {
        if p[2] < 0.5 {
            assert_eq!(front_only[i], 0.0, "point {i} at {p:?} is not on the front");
        } else if front_only[i] > 0.0 {
            front += 1;
            assert_eq!(
                front_only[i].to_bits(),
                leaks[i].to_bits(),
                "the front as without it"
            );
        }
    }
    assert!(front > 50, "the front is masked ({front} points)");
}

/// "Move Connected with Backface on cannot grab points facing away": a drag at the sphere's
/// side, whose connected reach runs round to the back.
#[test]
fn move_connected_with_backface_on_cannot_grab_points_facing_away() {
    let mesh = store(&ico(4)).to_plain();
    let g = Vec3::new(0.95, 0.0, 0.3).normalized();
    let s = [at(g, 0.8), at(g + Vec3::new(0.0, 0.15, 0.05), 0.8)];
    let grab = preset("move-connected").text;
    assert!(
        grab.contains("backface = 0.0"),
        "Move Connected ships with it off"
    );
    let on = grab.replace("backface = 0.0", "backface = 1.0");
    let free = after(&mesh, &ok(BrushPreset::from_toml(&grab)), params(None), &s);
    let held = after(&mesh, &ok(BrushPreset::from_toml(&on)), params(None), &s);
    // The test sphere's normals point out from its centre; the camera looks down -z.
    let mut away = 0;
    for (i, p) in mesh.positions.iter().enumerate() {
        if p[2] < -0.02 && moved(&mesh, &free, i) > 0.0 {
            away += 1;
            assert_eq!(
                moved(&mesh, &held, i),
                0.0,
                "point {i} faces away and moved"
            );
        }
        if p[2] > 0.02 {
            assert_eq!(
                held.positions[i], free.positions[i],
                "point {i} faces the camera"
            );
        }
    }
    assert!(away > 5, "the drag reached round to the back ({away})");
}

/// The Move family ships with the backface automask off, which would stop a drag from
/// grabbing what faces away (the spec's "ships off for the Move family").
#[test]
fn the_move_family_ships_with_backface_off() {
    let family: Vec<_> = (shipped().into_iter())
        .filter(|s| s.preset.family == Family::Move)
        .collect();
    assert!(
        family.len() >= 3,
        "Move, Move Connected, Move Elastic and kin"
    );
    for s in family {
        assert_eq!(s.preset.automask.backface, 0.0, "{}", s.stem);
    }
}

/// Params refining to edges of 0.02.
fn refining() -> StrokeParams {
    let mut p = params(None);
    p.live_detail = Some(LiveDetail {
        target: Target::Constant(0.02),
        ..LiveDetail::default()
    });
    p
}

/// "A brush with Backface on adds no Live Detail triangles", nor one with the face group or
/// topology automask on: over coarse quads Live Detail would split everywhere under the brush,
/// the stroke keeps the topology and still sculpts. The cavity automask refines.
#[test]
fn a_brush_with_backface_face_group_or_topology_on_adds_no_live_detail_triangles() {
    let samples: Vec<_> = (0..=8)
        .map(|k| at(Vec3::new(-0.4 + 0.1 * k as f32, 0.05, 0.0), 0.3))
        .collect();
    for (extra, named) in [
        ("backface = 0.5", Some(Automasked::Backface)),
        ("face_group = 1.0", Some(Automasked::FaceGroup)),
        ("topology = 1.0", Some(Automasked::Topology)),
        ("cavity = 1.0", None),
    ] {
        let p = plus("draw", &format!("[automask]\n{extra}"));
        assert_eq!(
            live::refuses(&p),
            named.map(NotRefined::Automask),
            "{extra}"
        );
        let refines = named.is_none();
        let mut s = store(&grid(6));
        let before = (s.vertex_count(), s.face_count());
        let start = s.to_plain();
        let mut st = ok(Stroke::begin(&mut s, &p, refining()));
        ok(st.add(&mut s, &samples));
        let out = st.finish(&mut s);
        let after = (s.vertex_count(), s.face_count());
        if refines {
            assert!(out.live.is_some(), "{extra}: refines");
            assert_ne!(after, before, "{extra}");
            continue;
        }
        assert!(out.live.is_none(), "{extra}: {:?}", out.live);
        assert_eq!(after, before, "{extra}: no triangle added");
        let sculpted = (0..start.positions.len()).any(|i| moved(&start, &s.to_plain(), i) > 0.0);
        assert!(sculpted, "{extra}: the stroke still sculpts");
    }
    // The stroke runs with its automasks on: the same bits as without Live Detail.
    let p = plus("draw", "[automask]\nbackface = 1.0\ntopology = 1.0");
    let mut with = store(&grid(6));
    run(&mut with, &p, refining(), &samples);
    let without = after(&grid(6), &p, params(None), &samples);
    assert_eq!(with.to_plain().positions, without.positions);
}

/// Bumps 0.04 high and 0.5 apart on a plane of 64 by 64 quads: hollows at the points where the
/// height is lowest, peaks where it is highest. Also the bumps' height.
pub(crate) fn bumpy() -> (PlainMesh, f32) {
    let a = 0.04_f64;
    let mut m = grid(64);
    for p in &mut m.positions {
        let wave = |c: f32| libm::cos(std::f64::consts::TAU * f64::from(c) / 0.5);
        p[2] = (a * wave(p[0]) * wave(p[1])) as f32;
    }
    (store(&m).to_plain(), a as f32)
}

/// "Cavity Mask 100 on a bumpy plane leaves hollows unchanged": the bottom of every hollow
/// under a Draw dab keeps its place, while the peaks move as they would without the mask.
#[test]
fn cavity_mask_100_leaves_the_hollows_of_a_bumpy_plane_unchanged() {
    let (mesh, a) = bumpy();
    let dab = [at(Vec3::new(0.0, 0.0, a), 0.6)];
    let plain = after(&mesh, &preset("draw").preset, params(None), &dab);
    let masked = after(
        &mesh,
        &plus("draw", "[automask]\ncavity = 1.0"),
        params(None),
        &dab,
    );
    let (mut hollows, mut peaks) = (0, 0);
    for (i, p) in mesh.positions.iter().enumerate() {
        let (free, held) = (moved(&mesh, &plain, i), moved(&mesh, &masked, i));
        if free == 0.0 {
            continue;
        }
        if p[2] < -0.999 * a {
            hollows += 1;
            assert_eq!(
                held,
                0.0,
                "the hollow at {p:?} moved {} of {free}",
                held / free
            );
        }
        if p[2] > 0.999 * a {
            peaks += 1;
            assert_eq!(held.to_bits(), free.to_bits(), "the peak at {p:?}");
        }
    }
    assert!(
        hollows >= 4 && peaks >= 4,
        "{hollows} hollows and {peaks} peaks under the dab"
    );
}

/// The cavity automask reads the one measure the cavity mask maker writes (decision 0056): a
/// Draw dab with Cavity `c` moves every point of the bumpy plane as the same dab does over the
/// mask `mask_make` writes with contrast `100 c` and one ring, at an intensity where the
/// hollows are held fully and at one where the mask is partial nearly everywhere. The mask
/// column keeps its values on a 2^-24 grid (`mask::settle`), which the automask's need not be
/// on, so the two moves agree to a millionth of the unmasked move, and the rounding of
/// positions below 1.
#[test]
fn the_cavity_automask_reads_what_the_cavity_mask_maker_writes() {
    let (mesh, a) = bumpy();
    let dab = [at(Vec3::new(0.1, -0.05, a), 0.6)];
    let draw = preset("draw").preset;
    let plain = after(&mesh, &draw, params(None), &dab);
    for c in [1.0_f32, 0.15] {
        let mut s = store(&mesh);
        let maker = MaskMaker::Cavity {
            contrast: 100.0 * c,
            rings: 1,
        };
        s.mask_make(&maker, NewMask::default());
        let made = s.to_plain();
        let partial = (floats(&made, "mask").into_iter()).filter(|&m| m > 0.0 && m < 1.0);
        let partial = partial.count();
        assert!(
            partial > 100,
            "at {c} the mask is partial at {partial} points"
        );
        let over_mask = after(&made, &draw, params(None), &dab);
        let cavity = plus("draw", &format!("[automask]\ncavity = {c}"));
        let automasked = after(&mesh, &cavity, params(None), &dab);
        let mut held = 0;
        for i in 0..mesh.positions.len() {
            let (free, off) = (moved(&mesh, &plain, i), moved(&over_mask, &automasked, i));
            assert!(
                off <= 1e-6 * free + 1e-7,
                "point {i} at {c}: {off} of {free}"
            );
            held += usize::from(moved(&mesh, &automasked, i) < 0.9 * free);
        }
        assert!(held > 100, "the automask held {held} points at {c}");
    }
}
