//! The mask's whole-part edits (research 13, 1.5; decision 0015): each neighbour filter against
//! a slow reference written here from the rule, the promises each filter makes, the sharpen
//! check of the catalog, invert twice giving back the same bits, clearing releasing columns,
//! and an edit that changes nothing adding none.

use std::collections::{BTreeMap, BTreeSet};

use gs_core::mask::{MaskFilter, Scope, settle};
use gs_core::select::{FaceRule, Selector};
use gs_core::{MeshStore, NO, Vec3, VisibilityEdit};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::brush_kit::{arc, at, floats, grid, hidden_faces, params, preset, run, store};
use crate::common::{mix, sphere, unit};

/// Exactly `v`, bit for bit.
fn is(x: f32, v: f32) -> bool {
    x.to_bits() == v.to_bits()
}

const FILTERS: [MaskFilter; 4] = [
    MaskFilter::Blur,
    MaskFilter::Sharpen,
    MaskFilter::Grow,
    MaskFilter::Shrink,
];

/// `mesh` with a mask from `f` (by vertex) and the faces `hide` picks (by face) hidden.
fn with(
    mesh: &PlainMesh,
    f: impl Fn(usize, [f32; 3]) -> f32,
    hide: impl Fn(usize) -> bool,
) -> PlainMesh {
    let mut m = mesh.clone();
    let mask = (m.positions.iter().enumerate())
        .map(|(i, p)| f(i, *p))
        .collect();
    let flags = (0..m.faces.len()).map(|i| u32::from(hide(i))).collect();
    let at = |domain, values| Attribute { domain, values };
    m.attributes
        .insert("mask".into(), at(AttrDomain::Vertex, AttrValues::F32(mask)));
    m.attributes
        .insert("hide".into(), at(AttrDomain::Face, AttrValues::U32(flags)));
    m
}

/// One filter pass the slow way, in `f64`: for every visible vertex, the mean of its distinct
/// neighbours across visible faces, the rule's target, a step toward it (half for blur), then
/// clamped and rounded to the 2^-24 grid. Hidden vertices keep their value.
fn reference(m: &PlainMesh, filter: MaskFilter, strength: f64) -> Vec<f32> {
    let mask = floats(m, "mask");
    let hide = hidden_faces(m);
    let mut nb: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); m.positions.len()];
    let mut faces = vec![(0, 0); m.positions.len()];
    for (f, &h) in m.faces.iter().zip(&hide) {
        let c: Vec<usize> = f
            .iter()
            .filter(|&&v| v != NO)
            .map(|&v| v as usize)
            .collect();
        for (k, &v) in c.iter().enumerate() {
            faces[v] = (faces[v].0 + 1, faces[v].1 + usize::from(h));
            if !h {
                nb[v].insert(c[(k + 1) % c.len()]);
                nb[v].insert(c[(k + c.len() - 1) % c.len()]);
            }
        }
    }
    (0..mask.len())
        .map(|v| {
            let m0 = f64::from(mask[v]);
            let hidden = faces[v].0 > 0 && faces[v].0 == faces[v].1;
            if hidden || nb[v].is_empty() {
                return mask[v];
            }
            let mean = nb[v].iter().map(|&u| f64::from(mask[u])).sum::<f64>() / nb[v].len() as f64;
            let (target, step) = match filter {
                MaskFilter::Blur => (mean, 0.5),
                MaskFilter::Sharpen => ((2.0 * m0 - mean).clamp(0.0, 1.0), 1.0),
                MaskFilter::Grow => (m0.max(mean), 1.0),
                MaskFilter::Shrink => (m0.min(mean), 1.0),
            };
            let x = (m0 + (target - m0) * step * strength).clamp(0.0, 1.0);
            ((x * 16_777_216.0).round() / 16_777_216.0) as f32
        })
        .collect()
}

/// A lumpy sphere with a random mask (a quarter of it 0, a quarter 1, the rest between) and
/// the faces right of x = 0.3 hidden.
fn masked_sphere() -> PlainMesh {
    let mesh = sphere(14, 0.5, 31, false);
    let right: Vec<bool> = (mesh.faces.iter())
        .map(|f| {
            let xs: Vec<f32> = f
                .iter()
                .filter(|&&v| v != NO)
                .map(|&v| mesh.positions[v as usize][0])
                .collect();
            xs.iter().sum::<f32>() / xs.len() as f32 > 0.3
        })
        .collect();
    with(
        &mesh,
        |i, _| match mix(i as u64) % 4 {
            0 => 0.0,
            1 => 1.0,
            _ => settle(unit(mix(i as u64 ^ 9)).abs()),
        },
        |i| right[i],
    )
}

#[test]
fn each_filter_pass_matches_the_slow_reference() {
    let mesh = masked_sphere();
    for filter in FILTERS {
        for strength in [1.0, 0.4] {
            let mut m = store(&mesh);
            let before = m.to_plain();
            m.begin_step();
            m.mask_filter(filter, strength);
            m.end_step();
            let got = floats(&m.to_plain(), "mask");
            let want = reference(&before, filter, f64::from(strength));
            let worst = (got.iter().zip(&want))
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(
                worst <= 2.5e-7,
                "{filter:?} at {strength}: {worst} from the reference"
            );
            assert!(
                got != floats(&before, "mask"),
                "{filter:?} changed something"
            );
        }
    }
}

#[test]
fn filters_keep_their_promises_and_the_mask_in_range() {
    let mesh = masked_sphere();
    let hidden_points = |m: &PlainMesh| {
        let hide = hidden_faces(m);
        let mut faces = vec![(0, 0); m.positions.len()];
        for (f, &h) in m.faces.iter().zip(&hide) {
            for &v in f.iter().filter(|&&v| v != NO) {
                let e = &mut faces[v as usize];
                *e = (e.0 + 1, e.1 + usize::from(h));
            }
        }
        faces
            .iter()
            .map(|&(n, h)| n > 0 && n == h)
            .collect::<Vec<bool>>()
    };
    let mut m = store(&mesh);
    for step in 0..24u64 {
        let filter = FILTERS[(mix(step) % 4) as usize];
        let before = m.to_plain();
        let hidden = hidden_points(&before);
        m.begin_step();
        m.mask_filter(filter, 1.0);
        m.end_step();
        let (b, a) = (floats(&before, "mask"), floats(&m.to_plain(), "mask"));
        for i in 0..a.len() {
            assert!(
                a[i].is_finite() && (0.0..=1.0).contains(&a[i]),
                "{filter:?}: {}",
                a[i]
            );
            if hidden[i] {
                assert_eq!(
                    a[i].to_bits(),
                    b[i].to_bits(),
                    "{filter:?}: a hidden point changed"
                );
            }
            match filter {
                MaskFilter::Grow => assert!(a[i] >= b[i], "grow lowered a point"),
                MaskFilter::Shrink => assert!(a[i] <= b[i], "shrink raised a point"),
                _ => {}
            }
            if is(b[i], 1.0) && filter == MaskFilter::Grow
                || is(b[i], 0.0) && filter == MaskFilter::Shrink
            {
                assert_eq!(a[i], b[i], "{filter:?}: a fully masked or free point stays");
            }
        }
    }
    // A uniform mask over every visible point does not change under blur or sharpen.
    let mut m = store(&with(&mesh, |_, _| 0.375, |_| false));
    let before = m.to_plain();
    for filter in [MaskFilter::Blur, MaskFilter::Sharpen] {
        m.begin_step();
        assert!(
            m.mask_filter(filter, 1.0).is_empty(),
            "{filter:?} wrote nothing"
        );
        m.end_step();
    }
    assert_eq!(floats(&m.to_plain(), "mask"), floats(&before, "mask"));
}

#[test]
fn sharpen_makes_a_blurred_edge_hard_again_where_it_was() {
    // A dense plane masked left of x = -0.0125, between two columns of vertices, blurred four
    // times (the catalog's masking.sharpen check).
    let edge = -0.0125;
    let mesh = with(
        &grid(80),
        |_, p| if p[0] < edge { 1.0 } else { 0.0 },
        |_| false,
    );
    let mut m = store(&mesh);
    for _ in 0..4 {
        m.mask_filter(MaskFilter::Blur, 1.0);
    }
    let band = |m: &MeshStore| {
        let v = floats(&m.to_plain(), "mask");
        v.iter().filter(|&&x| x > 0.1 && x < 0.9).count()
    };
    let mut last = band(&m);
    assert!(last > 100, "the blur made a soft band ({last} points)");
    let mut passes = 0;
    while band(&m) > 0
        || !floats(&m.to_plain(), "mask")
            .iter()
            .all(|&x| is(x, 0.0) || is(x, 1.0))
    {
        m.mask_filter(MaskFilter::Sharpen, 1.0);
        passes += 1;
        let now = band(&m);
        assert!(
            now <= last,
            "pass {passes} widened the band ({last} to {now})"
        );
        last = now;
        assert!(passes <= 12, "the edge is hard within 12 passes");
    }
    let after = m.to_plain();
    let ring = 2.0 / 80.0;
    for (p, x) in after.positions.iter().zip(floats(&after, "mask")) {
        if (p[0] - edge).abs() > ring {
            assert_eq!(
                is(x, 1.0),
                p[0] < edge,
                "the edge stays within one ring of {edge}"
            );
        }
    }
}

/// Every vertex's mask, keyed by its position.
fn by_position(m: &PlainMesh) -> BTreeMap<[u32; 3], u32> {
    gs_testkit::compare::bits_by_position(m, "mask")
}

/// Review 0003 m3: an imported mask is settled as every writer stores it, clamped to 0..1 on
/// its grid, so it keeps the mask's promises: values in 0..1, and invert twice the same bits.
#[test]
fn an_imported_mask_is_settled_like_every_written_one() {
    let mut mesh = sphere(6, 0.5, 7, false);
    let wild = [5.0, -3.0, 0.1, f32::MIN_POSITIVE];
    let mask = (0..mesh.positions.len())
        .map(|i| wild[i % wild.len()])
        .collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), column);
    let mut m = store(&mesh);
    let held = by_position(&m.to_plain());
    for &x in held.values() {
        let x = f32::from_bits(x);
        assert!(
            is(x, settle(x)) && (0.0..=1.0).contains(&x),
            "{x} is settled"
        );
    }
    m.mask_invert(Scope::All);
    m.mask_invert(Scope::All);
    assert_eq!(
        by_position(&m.to_plain()),
        held,
        "invert twice is the same bits"
    );
}

#[test]
fn invert_twice_gives_back_the_same_bits_and_set_follows_its_scope() {
    let mesh = masked_sphere();
    let mut m = store(&mesh);
    let pen = preset("mask-pen").preset;
    let from = gs_core::Vec3::new(-0.3, 0.2, 1.0).normalized();
    let to = gs_core::Vec3::new(0.4, -0.1, 1.0).normalized();
    run(&mut m, &pen, params(None), &arc(from, to, 6, 0.3));
    m.mask_filter(MaskFilter::Blur, 0.7);
    for scope in [Scope::Visible, Scope::All] {
        let before = by_position(&m.to_plain());
        m.mask_invert(scope);
        assert_ne!(
            by_position(&m.to_plain()),
            before,
            "{scope:?}: invert changed the mask"
        );
        m.mask_invert(scope);
        assert_eq!(
            by_position(&m.to_plain()),
            before,
            "{scope:?}: twice is the same bits"
        );
    }
    // Mask All on a partly hidden part masks the visible points only.
    let hidden: BTreeSet<[u32; 3]> = {
        let p = m.to_plain();
        let hide = hidden_faces(&p);
        let mut faces = vec![(0, 0); p.positions.len()];
        for (f, &h) in p.faces.iter().zip(&hide) {
            for &v in f.iter().filter(|&&v| v != NO) {
                let e = &mut faces[v as usize];
                *e = (e.0 + 1, e.1 + usize::from(h));
            }
        }
        (p.positions.iter().zip(faces))
            .filter(|(_, (n, h))| *n > 0 && n == h)
            .map(|(q, _)| q.map(f32::to_bits))
            .collect()
    };
    let before = by_position(&m.to_plain());
    m.mask_set(1.0, Scope::Visible);
    for (k, x) in by_position(&m.to_plain()) {
        let want = if hidden.contains(&k) {
            before[&k]
        } else {
            1.0f32.to_bits()
        };
        assert_eq!(x, want, "mask all");
    }
    // Clearing every point releases every node's column.
    m.mask_set(0.0, Scope::All);
    let mask = m
        .attr_id("mask")
        .unwrap_or_else(|| panic!("the mask column exists"));
    for n in 0..m.node_count() as u32 {
        assert!(
            m.node(n).attr(mask).is_none(),
            "node {n} still holds a mask"
        );
    }
}

/// Review 0003 M3: an edit that changes nothing adds no column, so the store is exactly as it
/// was (a job running over the part can still land). Hiding no face, showing faces already
/// shown, clearing a mask the part does not have, and a Mask Pen stroke that touches nothing
/// add none; the first value written adds the column.
#[test]
fn an_edit_that_changes_nothing_adds_no_column() {
    let mut m = store(&sphere(14, 1.0, 5, false));
    let before = m.attrs().to_vec();
    let none = m.select(&Selector::Groups(vec![99]));
    let every = m.select(&Selector::Unmasked {
        threshold: 0.5,
        rule: FaceRule::Mean,
    });
    for edit in [
        VisibilityEdit::Hide(&none),
        VisibilityEdit::Only(&every),
        VisibilityEdit::Show(&every),
        VisibilityEdit::ShowAll,
    ] {
        assert!(
            m.edit_visibility(edit).is_empty(),
            "{edit:?} changed nothing"
        );
    }
    assert!(m.mask_set(0.0, Scope::All).is_empty(), "no mask to clear");
    let far = at(Vec3::from_array([5.0, 5.0, 5.0]), 0.1);
    run(&mut m, &preset("mask-pen").preset, params(None), &[far]);
    assert_eq!(m.attrs(), &before[..], "no column added");
    m.mask_set(0.5, Scope::All);
    assert_eq!(m.attrs().len(), before.len() + 1, "a value adds the mask");
}
