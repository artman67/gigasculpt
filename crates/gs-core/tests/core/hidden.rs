//! Hidden faces (decision 0015): the derived hidden-point bits and node summaries against a
//! brute-force rebuild after every edit and undo (differential), hidden and fully masked points
//! never move under any brush (property), hidden faces do not pull on visible points at the
//! border (metamorphic: the same stroke with the hidden faces deleted), a stroke keeping the
//! bits of every point it does not move, and the node skips held to a work counter.

use std::collections::BTreeMap;

use gs_core::mask::{MaskFilter, Scope};
use gs_core::select::{FaceRule, Selector};
use gs_core::{Cover, MeshStore, NO, NodeState, StrokeParams, Vec3, View, VisibilityEdit, stroke};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::brush_kit::{
    at, grid, hidden_faces, mirror_x, params, preset, run, samples, shipped, store,
};
use crate::common::{mix, ok, refine, sphere, surface_point, unit};

/// `mesh` with the faces `hide` picks (by their corners) hidden.
fn hiding(mesh: &PlainMesh, hide: impl Fn(&[Vec3]) -> bool) -> PlainMesh {
    let mut m = mesh.clone();
    let flags = (m.faces.iter())
        .map(|f| {
            let c: Vec<Vec3> = (f.iter().filter(|&&v| v != NO))
                .map(|&v| Vec3::from_array(m.positions[v as usize]))
                .collect();
            u32::from(hide(&c))
        })
        .collect();
    let values = AttrValues::U32(flags);
    let column = Attribute {
        domain: AttrDomain::Face,
        values,
    };
    m.attributes.insert("hide".into(), column);
    m
}

fn center(c: &[Vec3]) -> Vec3 {
    c.iter().fold(Vec3::ZERO, |s, &p| s + p) * (1.0 / c.len() as f32)
}

// ---------------------------------------------------------------- derived state

fn cover(count: usize, of: usize) -> Cover {
    if count == 0 {
        Cover::None
    } else if count < of {
        Cover::Some
    } else {
        Cover::All
    }
}

/// Hidden points and node summaries rebuilt from the stored faces, hide bits and mask, by
/// brute force: every live face of every node, counted against every corner.
fn brute(m: &MeshStore) -> (Vec<Vec<bool>>, Vec<NodeState>, bool) {
    let hide = m.attr_id("hide").map(|h| m.attr_reader(h));
    let mask = m.attr_id("mask").map(|a| m.attr_reader(a));
    let mut around: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut face_counts = Vec::new();
    for n in 0..m.node_count() as u32 {
        let node = m.node(n);
        let (mut live, mut hidden) = (0, 0);
        for (j, f) in node.faces.iter().enumerate().filter(|f| f.1[0] != NO) {
            let h = hide.is_some_and(|r| r.label(node.first_face + j as u32) != 0);
            (live, hidden) = (live + 1, hidden + usize::from(h));
            for &v in f.iter().filter(|&&v| v != NO) {
                let e = around.entry(v).or_default();
                *e = (e.0 + 1, e.1 + u32::from(h));
            }
        }
        face_counts.push(cover(hidden, live));
    }
    let mut points = Vec::new();
    let mut states = Vec::new();
    for n in 0..m.node_count() as u32 {
        let node = m.node(n);
        let bits: Vec<bool> = (0..node.pos.len() as u32)
            .map(|l| {
                around
                    .get(&(node.base + l))
                    .is_some_and(|&(k, h)| k > 0 && k == h)
            })
            .collect();
        let live: Vec<usize> = (0..node.pos.len())
            .filter(|&l| !node.pos[l].is_dead())
            .collect();
        let hidden = live.iter().filter(|&&l| bits[l]).count();
        let ms: Vec<f32> = (live.iter())
            .map(|&l| mask.map_or(0.0, |r| r.scalar(node.base + l as u32)))
            .collect();
        let mask_cover = if ms.iter().all(|&x| x <= 0.0) {
            Cover::None
        } else if ms.iter().all(|&x| x >= 1.0) {
            Cover::All
        } else {
            Cover::Some
        };
        states.push(NodeState {
            faces: face_counts[n as usize],
            points: cover(hidden, live.len()),
            mask: mask_cover,
        });
        points.push(bits);
    }
    let any = states.iter().any(|s| s.faces != Cover::None);
    (points, states, any)
}

/// The store's derived hidden points, node summaries and any-hidden flag, against `brute`.
pub(crate) fn same_state(m: &MeshStore, what: &str) {
    let (points, states, any) = brute(m);
    assert_eq!(m.any_hidden(), any, "{what}: any hidden");
    for n in 0..m.node_count() as u32 {
        let node = m.node(n);
        let got: Vec<bool> = (0..node.pos.len()).map(|l| node.hidden(l)).collect();
        assert_eq!(got, points[n as usize], "{what}: hidden points of node {n}");
        assert_eq!(
            m.node_state(n),
            states[n as usize],
            "{what}: summary of node {n}"
        );
    }
}

/// A random rectangle on the plane z = 3 seen along -z, for Select Rect: its press and release.
fn random_rect(h: u64) -> Vec<gs_core::StrokeSample> {
    let (x, y) = (unit(h) * 0.6, unit(mix(h)) * 0.6);
    let (w, t) = (
        0.2 + unit(mix(h ^ 1)).abs() * 0.6,
        0.2 + unit(mix(h ^ 2)).abs() * 0.6,
    );
    [(x, y), (x + w, y + t)]
        .map(|(a, b)| at(Vec3::new(a, b, 3.0), 0.1))
        .to_vec()
}

#[test]
fn derived_hidden_state_matches_a_brute_force_rebuild_after_every_edit_and_undo() {
    let mut m = store(&sphere(14, 1.0, 5, true));
    same_state(&m, "imported");
    let select = preset("select-rect").preset;
    let pen = preset("mask-pen").preset;
    for step in 0..60u64 {
        let h = mix(step * 977 + 13);
        let what = format!("step {step} (op {})", h % 9);
        match h % 9 {
            0 => {
                let mut prm = params((h & 16 != 0).then(mirror_x));
                prm.invert = h & 32 != 0;
                run(&mut m, &select, prm, &random_rect(h));
            }
            1 | 2 => {
                let op = [VisibilityEdit::Invert, VisibilityEdit::ShowAll][(h % 9) as usize - 1];
                m.begin_step();
                m.edit_visibility(op);
                m.end_step();
            }
            3 => {
                let rule = [FaceRule::Mean, FaceRule::Any, FaceRule::All][(h >> 8) as usize % 3];
                let sel = Selector::Masked {
                    threshold: 0.3 + unit(h).abs() * 0.4,
                    rule,
                };
                m.begin_step();
                let set = m.select(&sel);
                m.edit_visibility(match (h >> 12) % 3 {
                    0 => VisibilityEdit::Hide(&set),
                    1 => VisibilityEdit::Show(&set),
                    _ => VisibilityEdit::Only(&set),
                });
                m.end_step();
            }
            4 => {
                m.begin_step();
                m.mask_set(
                    unit(h).abs(),
                    [Scope::Visible, Scope::All][(h >> 4) as usize % 2],
                );
                m.mask_filter(MaskFilter::Blur, 1.0);
                m.end_step();
            }
            5 => {
                let c = surface_point(&m, Vec3::new(unit(h), unit(mix(h)), 0.7));
                m.begin_step();
                refine(&mut m, c, 0.3, 0.05, 2);
                m.end_step();
            }
            6 => drop(m.undo()),
            7 => drop(m.redo()),
            _ => {
                let a = Vec3::new(unit(h) * 0.5, unit(mix(h)) * 0.5, 1.0).normalized();
                let b = Vec3::new(unit(mix(h ^ 3)) * 0.5, 0.2, 1.0).normalized();
                let mut prm = params(None);
                prm.invert = h & 64 != 0;
                run(&mut m, &pen, prm, &crate::brush_kit::arc(a, b, 3, 0.25));
            }
        }
        same_state(&m, &what);
    }
}

/// Review 0003 M6: Invert swaps hidden and visible faces on any part, so on a fully visible
/// part it hides every face, whether the part never had a `hide` column or an undone hide left
/// one that holds nothing (decision 0015).
#[test]
fn invert_hides_every_face_of_a_fully_visible_part_whatever_columns_it_has() {
    let mesh = sphere(14, 1.0, 5, false);
    let invert = |m: &mut MeshStore| {
        m.edit_visibility(VisibilityEdit::Invert);
        hidden_faces(&m.to_plain())
    };
    let mut fresh = store(&mesh);
    let mut undone = store(&mesh);
    undone.begin_step();
    let every = undone.select(&Selector::Unmasked {
        threshold: 0.5,
        rule: FaceRule::Mean,
    });
    undone.edit_visibility(VisibilityEdit::Hide(&every));
    undone.end_step();
    drop(undone.undo());
    assert_eq!(
        undone.attrs().len(),
        fresh.attrs().len() + 1,
        "the undone hide left its column"
    );
    let hidden = invert(&mut fresh);
    assert!(
        !hidden.is_empty() && hidden.iter().all(|&h| h),
        "every face hidden"
    );
    assert_eq!(invert(&mut undone), hidden, "the same faces either way");
    assert!(
        invert(&mut fresh).iter().all(|&h| !h),
        "and inverting again shows them all"
    );
}

// ---------------------------------------------------------------- what never moves

/// The vertices whose faces are all hidden, and those fully masked, in export order.
fn held(m: &PlainMesh) -> (Vec<bool>, Vec<bool>) {
    let hide: Vec<bool> = match m.attributes.get("hide").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&x| x != 0).collect(),
        _ => vec![false; m.faces.len()],
    };
    let mut only_hidden = vec![None; m.positions.len()];
    for (f, &h) in m.faces.iter().zip(&hide) {
        for &v in f.iter().filter(|&&v| v != NO) {
            let e: &mut Option<bool> = &mut only_hidden[v as usize];
            *e = Some(e.unwrap_or(true) && h);
        }
    }
    let masked = match m.attributes.get("mask").map(|a| &a.values) {
        Some(AttrValues::F32(v)) => v.iter().map(|&x| x >= 1.0).collect(),
        _ => vec![false; m.positions.len()],
    };
    (
        only_hidden.into_iter().map(|h| h == Some(true)).collect(),
        masked,
    )
}

#[test]
fn hidden_points_and_fully_masked_points_never_move_under_any_brush() {
    // A third of the points fully masked, and the faces right of x = 0.15 hidden.
    let mut mesh = sphere(16, 0.5, 9, false);
    let mask = (0..mesh.positions.len() as u64)
        .map(|i| {
            if mix(i * 5 + 1).is_multiple_of(3) {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), column);
    let mesh = hiding(&mesh, |c| center(c).x > 0.15);
    for s in shipped() {
        for sym in [None, Some(mirror_x())] {
            let mut m = store(&mesh);
            let before = m.to_plain();
            let (hidden, masked) = held(&before);
            run(&mut m, &s.preset, params(sym), &samples(&s.preset, 0.3));
            let after = m.to_plain();
            let paints_mask = s.stem == "mask-pen";
            let (mut hid, mut fixed) = (0, 0);
            for i in 0..before.positions.len() {
                let still =
                    before.positions[i].map(f32::to_bits) == after.positions[i].map(f32::to_bits);
                if hidden[i] {
                    hid += 1;
                    assert!(still, "{}: hidden vertex {i} moved", s.stem);
                }
                if masked[i] && !paints_mask {
                    fixed += 1;
                    assert!(still, "{}: fully masked vertex {i} moved", s.stem);
                }
            }
            assert!(
                hid > 100 && (fixed > 100 || paints_mask),
                "{}: the test holds points back",
                s.stem
            );
            if paints_mask {
                let (b, a) = (
                    crate::brush_kit::floats(&before, "mask"),
                    crate::brush_kit::floats(&after, "mask"),
                );
                for i in (0..b.len()).filter(|&i| hidden[i]) {
                    assert_eq!(
                        a[i].to_bits(),
                        b[i].to_bits(),
                        "a hidden point's mask changed"
                    );
                }
            }
        }
    }
}

/// Review 0003 m4: a stroke leaves every point it does not move as it was, bit for bit, `-0.0`
/// included (OBJ writers often print it), hidden points and the rest of a written node alike.
#[test]
fn a_stroke_keeps_the_bits_of_every_point_it_does_not_move() {
    let mut mesh = grid(60);
    for p in &mut mesh.positions {
        p[2] = -0.0;
    }
    let mesh = hiding(&mesh, |c| center(c).x < 0.0);
    let mut m = store(&mesh);
    let before = m.to_plain();
    let dab = [at(Vec3::new(0.2, 0.0, 0.0), 0.3)];
    run(&mut m, &preset("draw").preset, params(None), &dab);
    let after = m.to_plain();
    let (mut moved, mut kept) = (0, 0);
    for (b, a) in before.positions.iter().zip(&after.positions) {
        if (Vec3::from_array(*a) - Vec3::from_array(*b)).len2() > 0.0 {
            moved += 1;
        } else {
            kept += 1;
            assert_eq!(
                a.map(f32::to_bits),
                b.map(f32::to_bits),
                "{b:?} kept its bits"
            );
        }
    }
    assert!(moved > 0 && kept > moved, "{moved} moved, {kept} kept");
}

// ---------------------------------------------------------------- the border

/// `mesh` without its hidden faces and the vertices only they used, renumbered.
fn without_hidden(mesh: &PlainMesh) -> PlainMesh {
    let hide = match mesh.attributes.get("hide").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.clone(),
        _ => vec![0; mesh.faces.len()],
    };
    let faces: Vec<[u32; 4]> = (mesh.faces.iter().zip(&hide))
        .filter(|f| *f.1 == 0)
        .map(|f| *f.0)
        .collect();
    let mut used = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    for f in &faces {
        for &v in f.iter().filter(|&&v| v != NO) {
            if used[v as usize] == u32::MAX {
                used[v as usize] = positions.len() as u32;
                positions.push(mesh.positions[v as usize]);
            }
        }
    }
    PlainMesh {
        positions,
        faces: faces
            .iter()
            .map(|f| f.map(|v| if v == NO { NO } else { used[v as usize] }))
            .collect(),
        attributes: BTreeMap::new(),
    }
}

/// Each vertex's position after, keyed by its position before.
fn moves(before: &PlainMesh, after: &PlainMesh) -> BTreeMap<[u32; 3], [f32; 3]> {
    (before.positions.iter().zip(&after.positions))
        .map(|(b, a)| (b.map(f32::to_bits), *a))
        .collect()
}

#[test]
fn hidden_faces_do_not_pull_visible_points_at_the_border() {
    let mesh = hiding(&sphere(16, 0.5, 21, false), |c| center(c).x > 0.1);
    let smooth = preset("smooth").preset;
    let stroke_on = |m: &PlainMesh| {
        let mut s = store(m);
        let g = surface_point(&s, Vec3::new(0.15, 0.2, 0.9));
        let before = s.to_plain();
        let view = View {
            dir: (Vec3::ZERO - g).normalized(),
            eye: None,
        };
        ok(stroke(
            &mut s,
            &smooth,
            StrokeParams::new(view),
            &[at(g, 0.35)],
        ));
        moves(&before, &s.to_plain())
    };
    let hidden = stroke_on(&mesh);
    let cut = stroke_on(&without_hidden(&mesh));
    let mut shown = mesh.clone();
    shown.attributes.clear();
    let open = stroke_on(&shown);
    let (mut matched, mut pulled) = (0, 0);
    for (key, after) in &cut {
        let got = hidden[key];
        let d = (0..3)
            .map(|k| (got[k] - after[k]).abs())
            .fold(0.0, f32::max);
        assert!(
            d < 1e-6,
            "a visible point moved {d} off the cut mesh's result"
        );
        matched += 1;
        let o = open[key];
        pulled += usize::from((0..3).any(|k| (o[k] - after[k]).abs() > 1e-4));
    }
    assert!(matched > 500, "{matched} visible points compared");
    assert!(
        pulled > 5,
        "with every face shown the border moves differently ({pulled})"
    );
}

// ---------------------------------------------------------------- node skips

/// One Draw dab on the surface toward `toward`, seen from outside: the vertex slots it
/// examined, the nodes it skipped, and whether any point moved.
fn dab(m: &mut MeshStore, toward: Vec3) -> (u64, u64, bool) {
    let g = surface_point(m, toward);
    let view = View {
        dir: (Vec3::ZERO - g).normalized(),
        eye: None,
    };
    let w = m.work();
    let before = m.to_plain();
    run(
        m,
        &preset("draw").preset,
        StrokeParams::new(view),
        &[at(g, 0.3)],
    );
    let moved = before.positions != m.to_plain().positions;
    (
        m.work().examined - w.examined,
        m.work().skipped - w.skipped,
        moved,
    )
}

#[test]
fn a_stroke_over_hidden_nodes_examines_none_of_their_points() {
    let mesh = sphere(48, 0.5, 42, false);
    let back = hiding(&mesh, |c| center(c).z < 0.2);
    // Wholly inside the hidden back: every node it reaches is skipped.
    let mut m = store(&back);
    let (examined, skipped, moved) = dab(&mut m, Vec3::new(0.1, 0.2, -1.0));
    assert!(
        !moved && examined == 0 && skipped > 0,
        "{examined} slots examined, {skipped} nodes skipped"
    );
    // Across the border: the hidden nodes' slots are not examined, the visible ones are.
    let toward = Vec3::new(0.0, 1.0, 0.0);
    let (open, _, _) = dab(&mut store(&mesh), toward);
    let (part, skipped, moved) = dab(&mut store(&back), toward);
    assert!(
        moved && skipped > 0 && part < open,
        "{part} of {open} slots examined"
    );
}

/// Review 0003 T1: a node under mask 1 at every point is skipped as a hidden one is: the
/// stroke reaches it, examines none of its points, and counts it skipped.
#[test]
fn a_stroke_over_fully_masked_nodes_examines_none_of_their_points() {
    let mut mesh = sphere(48, 0.5, 42, false);
    let mask = (mesh.positions.iter())
        .map(|p| if p[2] < 0.2 { 1.0 } else { 0.0 })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), column);
    let (examined, skipped, moved) = dab(&mut store(&mesh), Vec3::new(0.1, 0.2, -1.0));
    assert!(
        !moved && examined == 0 && skipped > 0,
        "{examined} slots examined, {skipped} nodes skipped"
    );
}
