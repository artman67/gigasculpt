//! The one column-carry rule (decision 0037, section 3), checked against division's `blend`
//! and against a plain reference for each kind.

use gs_core::{AttrDef, AttrKind, AttrValue, Domain};

use crate::common::{mix, unit};

const KINDS: [AttrKind; 4] = [
    AttrKind::Scalar,
    AttrKind::Label,
    AttrKind::Rgba8,
    AttrKind::Flag,
];

/// A point column of `kind` that is not the mask, so its values blend unsettled.
fn def(kind: AttrKind) -> AttrDef {
    AttrDef::new("values", Domain::Vertex, kind)
}

fn bits(v: AttrValue) -> u32 {
    match v {
        AttrValue::F32(x) => x.to_bits(),
        AttrValue::U32(x) => x,
    }
}

/// `k` values of `kind` from seed `s`.
fn values(kind: AttrKind, k: usize, s: u64) -> Vec<AttrValue> {
    (0..k as u64)
        .map(|i| match kind {
            AttrKind::Scalar => AttrValue::F32(unit(mix(s * 31 + i))),
            AttrKind::Flag => AttrValue::U32((mix(s * 31 + i) & 1) as u32),
            _ => AttrValue::U32(mix(s * 31 + i) as u32),
        })
        .collect()
}

#[test]
fn weights_of_one_give_the_same_bits_as_division_blend() {
    for kind in KINDS {
        for k in 1..=4 {
            for s in 0..500 {
                let vals = values(kind, k, s);
                let plain = AttrValue::blend(&def(kind), &vals);
                let weighted = AttrValue::blend_weighted(&def(kind), &vals, &vec![1.0; k]);
                assert_eq!(bits(weighted), bits(plain), "{kind:?} {vals:?}");
            }
        }
    }
}

#[test]
fn each_kind_carries_by_its_own_rule_under_unequal_weights() {
    for s in 0..500 {
        let w = [mix(s), mix(s + 7), mix(s + 13)].map(|h| unit(h).abs());
        let total = f64::from(w[0]) + f64::from(w[1]) + f64::from(w[2]);
        // A scalar: the weighted mean, to f32 rounding.
        let m = values(AttrKind::Scalar, 3, s);
        let want: f64 = (m.iter().zip(w))
            .map(|(v, w)| f64::from(f32::from_bits(bits(*v))) * f64::from(w))
            .sum::<f64>()
            / total;
        let scalar = def(AttrKind::Scalar);
        let got = f32::from_bits(bits(AttrValue::blend_weighted(&scalar, &m, &w)));
        assert!(
            (f64::from(got) - want).abs() < 1e-6,
            "{m:?} {w:?}: {got} for {want}"
        );
        // A color: each channel its weighted mean, rounded, within one step for f32 sums.
        let c = values(AttrKind::Rgba8, 3, s);
        let got = bits(AttrValue::blend_weighted(&def(AttrKind::Rgba8), &c, &w));
        for shift in [0, 8, 16, 24] {
            let mean: f64 = (c.iter().zip(w))
                .map(|(v, w)| f64::from(bits(*v) >> shift & 255) * f64::from(w))
                .sum::<f64>()
                / total;
            let channel = f64::from(got >> shift & 255);
            assert!(
                (channel - mean).abs() <= 0.5 + 1e-4,
                "{c:?} {w:?}: {channel} for {mean}"
            );
        }
        // A label and a flag: the value with the largest weight.
        let top = (0..3).fold(0, |t, i| if w[i] > w[t] { i } else { t });
        for kind in [AttrKind::Label, AttrKind::Flag] {
            let l = values(kind, 3, s);
            assert_eq!(
                bits(AttrValue::blend_weighted(&def(kind), &l, &w)),
                bits(l[top])
            );
        }
    }
}

#[test]
fn a_label_tie_goes_to_the_first_and_no_weight_at_all_keeps_the_first_value() {
    let l = [AttrValue::U32(4), AttrValue::U32(9), AttrValue::U32(2)];
    let tied = AttrValue::blend_weighted(&def(AttrKind::Label), &l, &[0.25, 0.375, 0.375]);
    assert_eq!(bits(tied), 9);
    for kind in KINDS {
        let vals = values(kind, 3, 1);
        let none = AttrValue::blend_weighted(&def(kind), &vals, &[0.0; 3]);
        assert_eq!(bits(none), bits(vals[0]), "{kind:?}");
    }
}
