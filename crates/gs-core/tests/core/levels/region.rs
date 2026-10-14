//! The carry's marked region (decision 0014, section 5) against a carry over every point of
//! each level (review 0004, T1): the same edits on random lower levels, with masks and face
//! groups painted between, give the same top, bit for bit, positions and attributes.

use gs_core::{AttrValue, Settings};
use gs_protocol::PlainMesh;

use super::carry::{edit, set_attr};
use super::{bits, positions, stack, top_positions};
use crate::common::{dir, fan, fingerprint, mix, ok, snap, sphere, unit};

/// Creases on the first edges of a few base faces, at sharpness 1, 2, always and 1.
fn creases(mesh: &PlainMesh) -> Vec<(u32, u32, u8)> {
    let probe = stack(mesh, &Settings::default());
    let faces = probe.numbering().canonical_faces(probe.store());
    [(0usize, 1u8), (3, 2), (7, 255), (11, 1)]
        .iter()
        .filter(|(f, _)| *f < faces.len())
        .map(|&(f, s)| (faces[f][0], faces[f][1], s))
        .collect()
}

/// One scripted session on `mesh`, carrying over every point when `everything`: three levels,
/// detail on the top, then six rounds of a smooth push on a random lower level (sometimes the
/// level above it too) with mask and face group writes. The top's position bits and the
/// fingerprint of its whole store.
fn session(mesh: &PlainMesh, seed: u64, creased: bool, everything: bool) -> (Vec<[u32; 3]>, u64) {
    let settings = Settings {
        creases: if creased { creases(mesh) } else { Vec::new() },
        ..Settings::default()
    };
    let mut l = stack(mesh, &settings);
    l.carry_everything(everything);
    for _ in 0..3 {
        ok(l.divide(true));
    }
    edit(&mut l, |c, p| {
        p + dir((seed * 100_003 + u64::from(c)) % 1_000_000) * 0.002
    });
    let top = l.top_level();
    let attrs = l.store().attr_id("mask").is_some();
    for round in 0..6u64 {
        let h = mix(seed.wrapping_mul(31).wrapping_add(round));
        let j = (h % top as u64) as usize;
        ok(l.set(j));
        let p = positions(&l);
        let center = p[(mix(h) % p.len() as u64) as usize];
        let r = 0.15 + 0.4 * unit(mix(h ^ 0x7)).abs();
        let push = dir(mix(h ^ 0x9) % 1_000_000) * (0.05 * unit(mix(h ^ 0xb)).abs() + 0.01);
        edit(&mut l, |_, q| {
            let d = (q - center).len2().sqrt();
            if d < r {
                let w = 1.0 - d / r;
                q + push * (w * w)
            } else {
                q
            }
        });
        if attrs {
            let n = l.numbering().vertex_count() as u64;
            let faces = l.numbering().face_count() as u64;
            for k in 0..3u64 {
                let c = (mix(h ^ (100 + k)) % n) as u32;
                let m = unit(mix(h ^ (200 + k))).abs();
                set_attr(&mut l, "mask", c, AttrValue::F32(m));
                let f = (mix(h ^ (300 + k)) % faces) as u32;
                set_attr(&mut l, "group", f, AttrValue::U32(5 + k as u32));
            }
        }
        if h.is_multiple_of(3) && j + 1 < top {
            ok(l.set(j + 1));
            edit(&mut l, |c, q| if c % 17 == 0 { q + push * 0.5 } else { q });
        }
    }
    ok(l.set(top));
    (bits(&top_positions(&l)), fingerprint(&snap(l.top())))
}

/// The meshes: closed quads and triangles with a mask, groups and colors; closed quads; an open
/// fan of triangles with a boundary.
fn meshes() -> Vec<(&'static str, PlainMesh)> {
    vec![
        ("quads and triangles", sphere(6, 0.3, 31, true)),
        ("quads", sphere(5, 0.0, 7, true)),
        ("open fan", fan(7)),
    ]
}

/// Runs each case (mesh, creased, seed) both ways; panics listing the ones that differ.
fn agree(cases: &[(usize, bool, u64)]) {
    let meshes = meshes();
    let mut differ = Vec::new();
    for &(m, creased, seed) in cases {
        let (name, mesh) = &meshes[m];
        let marked = session(mesh, seed, creased, false);
        let all = session(mesh, seed, creased, true);
        let points = (marked.0.iter().zip(&all.0))
            .filter(|(a, b)| a != b)
            .count();
        if points > 0 || marked.1 != all.1 {
            differ.push(format!(
                "{name}, creased {creased}, seed {seed}: {points} of {} points differ, \
                 fingerprints {} and {}",
                marked.0.len(),
                marked.1,
                all.1
            ));
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

#[test]
fn the_marked_region_carries_what_a_carry_over_every_point_does() {
    // One seed on the mesh with attributes and on the open fan, creased and not. The other 32
    // of the 36 cases run at tier 2.
    agree(&[(0, false, 0), (0, true, 0), (2, false, 0), (2, true, 0)]);
}

mod tier2 {
    #[test]
    fn the_marked_region_carries_what_a_carry_over_every_point_does_in_every_case() {
        let tier1 = [(0, false, 0), (0, true, 0), (2, false, 0), (2, true, 0)];
        let cases: Vec<(usize, bool, u64)> = (0..3)
            .flat_map(|m| [false, true].map(|c| (m, c)))
            .flat_map(|(m, c)| (0..6).map(move |s| (m, c, s)))
            .filter(|x| !tier1.contains(x))
            .collect();
        assert_eq!(cases.len(), 32);
        super::agree(&cases);
    }
}
