//! Carrying edits up (decision 0014, section 5): rigid moves of a coarse level move the top
//! rigidly, top detail survives a coarse edit, points the edit cannot reach keep their bits,
//! attributes travel as deltas and face values reach every descendant (columns first made on a
//! lower level included), the result does not depend on the thread count, and deleting levels
//! keeps what is left.

use std::sync::Arc;

use gs_core::select::{FaceRule, Selector};
use gs_core::{
    AttrDef, AttrValue, AttrVec, ColumnSet, Levels, Numbering, PosRead, Settings, Vec3,
    VisibilityEdit,
};

use super::{bits, positions, stack, top_positions};
use crate::brush_kit::{arc, hidden_faces, params, preset, run};
use crate::common::{check, fingerprint, ok, snap, some, sphere};
use crate::hidden::same_state;

/// A sphere with `up` levels above its base, at the top, with no mask: the geometric relations
/// below hold for points the top lets move freely.
fn free(up: usize) -> Levels {
    built(up, false)
}

/// The same sphere carrying a mask (0 to 0.8 at random), face groups and colors.
pub(super) fn levels(up: usize) -> Levels {
    built(up, true)
}

fn built(up: usize, attrs: bool) -> Levels {
    let mut l = stack(&sphere(6, 0.3, 31, attrs), &Settings::default());
    for _ in 0..up {
        ok(l.divide(true));
    }
    l
}

/// Moves every point of the active level with `f(canonical index, position)`.
pub(super) fn edit(l: &mut Levels, f: impl Fn(u32, Vec3) -> Vec3 + Sync) {
    let num: Arc<Numbering> = Arc::clone(l.numbering());
    let store = l.store_mut();
    let nodes: Vec<u32> = (0..store.node_count() as u32).collect();
    store.write(&nodes, ColumnSet::POSITION).each(|node| {
        let id = node.id();
        for (local, p) in node.positions().iter_mut().enumerate() {
            *p = f(num.index(id, local as u32), *p);
        }
    });
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).len2().sqrt()
}

#[test]
fn a_rigid_move_of_a_coarse_level_moves_the_top_rigidly() {
    for (name, motion) in [
        (
            "translation",
            (|p: Vec3| p + Vec3::new(0.3, -0.2, 0.1)) as fn(Vec3) -> Vec3,
        ),
        ("rotation", |p: Vec3| {
            Vec3::new(0.6 * p.x - 0.8 * p.y, 0.8 * p.x + 0.6 * p.y, p.z)
        }),
    ] {
        let mut l = free(3);
        let before = top_positions(&l);
        ok(l.set(1));
        edit(&mut l, |_, p| motion(p));
        let carried = ok(l.set(3));
        assert!(carried.points > 0);
        let after = top_positions(&l);
        let worst = (before.iter().zip(&after))
            .map(|(&b, &a)| dist(motion(b), a))
            .fold(0.0, f32::max);
        assert!(worst < 2e-5, "{name}: the top moved rigidly within {worst}");
        assert_eq!(check(&snap(l.top())), Ok(()), "{name}: top invariants");
    }
}

/// Writes the active level's mask as `f(position)` at every point, registering the column.
fn mask_by(l: &mut Levels, f: impl Fn(Vec3) -> f32 + Sync) {
    let store = l.store_mut();
    let a = ok(store.add_attr(AttrDef::mask()));
    let nodes: Vec<u32> = (0..store.node_count() as u32).collect();
    let values: Vec<Vec<f32>> = (nodes.iter())
        .map(|&n| store.node(n).pos.iter().map(|&p| f(p)).collect())
        .collect();
    store.write(&nodes, ColumnSet::attr(a)).each(|node| {
        let v = &values[node.id() as usize];
        if let AttrVec::F32(m) = node.attr(a) {
            m.copy_from_slice(v);
        }
    });
}

#[test]
fn top_points_masked_or_hidden_on_the_top_keep_their_place_when_a_lower_level_moves() {
    // Protection made on the top (review 0004, M1): faces near +y hidden, a fully masked cap
    // near +z, a half-masked band near +x. Then level 1 moves rigidly.
    let mut l = free(3);
    l.store_mut().begin_step();
    mask_by(&mut l, |p| if p.y > 0.5 { 1.0 } else { 0.0 });
    let s = l.store_mut();
    let near_y = s.select(&Selector::Masked {
        threshold: 1.0,
        rule: FaceRule::All,
    });
    s.edit_visibility(VisibilityEdit::Hide(&near_y));
    mask_by(&mut l, |p| match () {
        () if p.z > 0.5 => 1.0,
        () if p.x > 0.5 => 0.5,
        () => 0.0,
    });
    l.store_mut().end_step();
    let (top, num) = (l.top(), Arc::clone(l.level_numbering(l.top_level())));
    let cfg = top.config();
    let mask = |c: u32| match attr(top, &num, "mask", c) {
        AttrValue::F32(m) => m,
        AttrValue::U32(_) => -1.0,
    };
    let hidden = |c: u32| {
        let v = num.id(c);
        top.node(cfg.node_of(v)).hidden(cfg.local_of(v) as usize)
    };
    let kinds: Vec<(bool, f32)> = (0..num.vertex_count() as u32)
        .map(|c| (hidden(c), mask(c)))
        .collect();
    let before = top_positions(&l);
    let t = Vec3::new(0.3, -0.2, 0.1);
    ok(l.set(1));
    edit(&mut l, |_, p| p + t);
    ok(l.set(3));
    let after = top_positions(&l);
    let mut seen = [0usize; 4];
    for (c, &(hide, m)) in kinds.iter().enumerate() {
        let (b, a) = (before[c], after[c]);
        let (kind, want) = match () {
            () if hide => (0, None),
            () if m >= 1.0 => (1, None),
            () if m > 0.0 => (2, Some(b + t * 0.5)),
            () => (3, Some(b + t)),
        };
        seen[kind] += 1;
        match want {
            None => assert_eq!(
                a.bits(),
                b.bits(),
                "point {c} (hidden {hide}, mask {m}) moved"
            ),
            Some(w) => assert!(
                dist(a, w) < 2e-5,
                "point {c} (mask {m}) is {} off",
                dist(a, w)
            ),
        }
    }
    assert!(
        seen.iter().all(|&n| n > 20),
        "every kind of point is there: {seen:?}"
    );
    assert_eq!(check(&snap(l.top())), Ok(()), "top invariants");
}

/// The height of the bump at canonical point `c`: its offset from the mean of the points
/// between `r0` and `r1` from it, along their mean normal.
fn bump(l: &Levels, c: u32, r0: f32, r1: f32) -> f32 {
    let p = top_positions(l);
    let num = l.level_numbering(l.top_level());
    let n = l.top().nrm();
    let ring: Vec<u32> = (0..p.len() as u32)
        .filter(|&u| (r0..r1).contains(&dist(p[u as usize], p[c as usize])))
        .collect();
    let mean = ring.iter().fold(Vec3::ZERO, |s, &u| s + p[u as usize]) * (1.0 / ring.len() as f32);
    let normal = ring
        .iter()
        .fold(Vec3::ZERO, |s, &u| s + n.get(num.id(u)))
        .normalized();
    (p[c as usize] - mean).dot(normal)
}

#[test]
fn top_detail_survives_a_coarse_move() {
    // Two stacks, one with a small bump at the top (about four top edges wide), get the same
    // coarse edit; the bump is what the one has over the other, measured before and after.
    let (mut with, mut without) = (free(3), free(3));
    let top = top_positions(&with);
    let c = (0..top.len() as u32).max_by(|&a, &b| top[a as usize].z.total_cmp(&top[b as usize].z));
    let c = some(c);
    let apex = top[c as usize];
    edit(&mut with, |_, p| {
        let d = dist(p, apex);
        if d < 0.12 {
            let w = 1.0 - d / 0.12;
            p + p.normalized() * (0.04 * w * w)
        } else {
            p
        }
    });
    let height = |a: &Levels, b: &Levels| bump(a, c, 0.15, 0.2) - bump(b, c, 0.15, 0.2);
    let before = height(&with, &without);
    // At level 1, push the whole cap up and sideways, smoothly: a stretch and a shear.
    for l in [&mut with, &mut without] {
        ok(l.set(1));
        edit(l, |_, p| {
            p + Vec3::new(0.2, 0.0, 0.3) * (p.z - 0.3).max(0.0)
        });
        ok(l.set(3));
    }
    let after = height(&with, &without);
    let ratio = after / before;
    assert!(
        (0.9..1.1).contains(&ratio),
        "the bump kept {ratio} of its height ({before} to {after})"
    );
}

#[test]
fn points_the_edit_cannot_reach_keep_their_bits() {
    let mut l = free(3);
    let before = top_positions(&l);
    ok(l.set(1));
    let p1 = positions(&l);
    let at = p1[17];
    edit(&mut l, |c, p| {
        if c == 17 {
            p + Vec3::new(0.0, 0.0, 0.05)
        } else {
            p
        }
    });
    ok(l.set(3));
    let after = top_positions(&l);
    let changed: Vec<usize> = (0..after.len())
        .filter(|&i| after[i].bits() != before[i].bits())
        .collect();
    assert_ne!(changed.len(), 0, "the edit reached the top");
    let far = changed
        .iter()
        .map(|&i| dist(before[i], at))
        .fold(0.0, f32::max);
    // Level 1 of this sphere has edges about 0.13 long: the reach is about two of its rings.
    assert!(far < 0.45, "a point {far} from the edit changed");
    assert!(
        changed.len() * 20 < after.len(),
        "{} of {} points changed",
        changed.len(),
        after.len()
    );
}

#[test]
fn the_same_carry_gives_the_same_bits_on_one_and_many_threads() {
    let run = |threads: usize| {
        gs_par::with_threads(threads, || {
            let mut l = levels(2);
            ok(l.set(0));
            edit(&mut l, |c, p| p * (1.0 + 0.01 * (c % 7) as f32));
            ok(l.set(2));
            ok(l.divide(true));
            fingerprint(&snap(l.top()))
        })
    };
    assert_eq!(run(1), run(24));
}

/// Writes `value` into face attribute `name` of the active store at canonical face `f`, or
/// into point attribute `name` at canonical point `f`.
pub(super) fn set_attr(l: &mut Levels, name: &str, f: u32, value: AttrValue) {
    let num: Arc<Numbering> = Arc::clone(l.numbering());
    let store = l.store_mut();
    let a = some(store.attr_id(name));
    let cfg = store.config();
    let point = store.attrs()[a.index()].domain == gs_core::Domain::Vertex;
    let (node, slot) = if point {
        let v = num.id(f);
        (cfg.node_of(v), cfg.local_of(v))
    } else {
        let id = num.face(f).0;
        (cfg.face_node(id), cfg.face_local(id))
    };
    store
        .write(&[node], ColumnSet::attr(a))
        .each(|n| match (n.attr(a), value) {
            (AttrVec::F32(v), AttrValue::F32(x)) => v[slot as usize] = x,
            (AttrVec::U32(v), AttrValue::U32(x)) => v[slot as usize] = x,
            (AttrVec::Bits(v), AttrValue::U32(x)) => v.set(slot as usize, x != 0),
            _ => panic!("value of the wrong type"),
        });
}

/// Value `i` of a node's column.
fn value(col: &AttrVec, i: usize) -> AttrValue {
    match col {
        AttrVec::F32(v) => AttrValue::F32(v[i]),
        AttrVec::U32(v) => AttrValue::U32(v[i]),
        AttrVec::Bits(v) => AttrValue::U32(u32::from(v.get(i))),
    }
}

fn attr(store: &gs_core::MeshStore, num: &Numbering, name: &str, c: u32) -> AttrValue {
    let a = some(store.attr_id(name));
    let v = num.id(c);
    let cfg = store.config();
    let col: Option<&AttrVec> = store.node(cfg.node_of(v)).attr(a);
    value(some(col), cfg.local_of(v) as usize)
}

#[test]
fn a_mask_painted_low_carries_up_as_a_delta() {
    let mut l = levels(2);
    ok(l.set(1));
    set_attr(&mut l, "mask", 40, AttrValue::F32(1.0));
    ok(l.set(2));
    let top = l.level_numbering(2);
    assert_eq!(
        attr(l.top(), top, "mask", 40),
        AttrValue::F32(1.0),
        "the edited point takes its value"
    );
    let all: Vec<f32> = (0..top.vertex_count() as u32)
        .map(|c| match attr(l.top(), top, "mask", c) {
            AttrValue::F32(x) => x,
            AttrValue::U32(_) => -1.0,
        })
        .collect();
    assert!(
        all.iter().all(|x| (0.0..=1.0).contains(x)),
        "clamped to 0..1"
    );
}

#[test]
fn a_group_set_low_reaches_every_descendant_and_shows_by_majority() {
    let mut l = levels(2);
    ok(l.set(1));
    set_attr(&mut l, "group", 9, AttrValue::U32(77));
    ok(l.set(2));
    let top = l.level_numbering(2);
    let store = l.top();
    let a = some(store.attr_id("group"));
    let cfg = store.config();
    for f in 9 * 4..9 * 4 + 4 {
        let id = top.face(f).0;
        let col = some(store.node(cfg.face_node(id)).attr(a));
        assert_eq!(
            value(col, cfg.face_local(id) as usize),
            AttrValue::U32(77),
            "face {f}"
        );
    }
    ok(l.set(1));
    let num = Arc::clone(l.numbering());
    let id = num.face(9).0;
    let col = some(l.store().node(cfg.face_node(id)).attr(a));
    assert_eq!(
        value(col, cfg.face_local(id) as usize),
        AttrValue::U32(77),
        "shown again at level 1"
    );
}

#[test]
fn deleting_higher_keeps_the_active_level_and_its_edits() {
    let mut l = free(3);
    ok(l.set(1));
    edit(&mut l, |c, p| if c == 5 { p * 1.1 } else { p });
    let live = positions(&l);
    assert!(l.delete_higher());
    assert_eq!((l.count(), l.active()), (2, 1));
    assert_eq!(
        bits(&top_positions(&l)),
        bits(&live),
        "the live store became the top"
    );
    ok(l.set(0));
    assert_eq!(bits(&positions(&l)), bits(&live[..positions(&l).len()]));
    assert!(!l.delete_lower() || l.count() == 1);
}

#[test]
fn deleting_lower_makes_the_active_level_the_base() {
    let mut l = free(3);
    let each: Vec<Vec<[u32; 3]>> = (1..4)
        .map(|j| {
            ok(l.set(j));
            bits(&positions(&l))
        })
        .collect();
    ok(l.set(1));
    assert!(l.delete_lower());
    assert_eq!((l.count(), l.active()), (3, 0));
    for (j, want) in each.iter().enumerate() {
        ok(l.set(j));
        assert_eq!(
            &bits(&positions(&l)),
            want,
            "level {j} after deleting below"
        );
    }
    ok(l.divide(true));
    assert_eq!(l.count(), 4, "a level can be added on the new base's stack");
}

#[test]
fn hidden_faces_and_a_mask_first_made_on_a_lower_level_reach_the_top_and_come_back() {
    let mut l = stack(&sphere(6, 0.3, 31, false), &Settings::default());
    for _ in 0..2 {
        ok(l.divide(true));
    }
    ok(l.set(1));
    let s = l.store_mut();
    let a = Vec3::new(-0.3, 0.1, 1.0).normalized();
    let b = Vec3::new(0.3, -0.1, 1.0).normalized();
    let pen = preset("mask-pen").preset;
    run(s, &pen, params(None), &arc(a, b, 3, 0.3));
    s.begin_step();
    let unmasked = s.select(&Selector::Unmasked {
        threshold: 0.5,
        rule: FaceRule::Mean,
    });
    s.edit_visibility(VisibilityEdit::Hide(&unmasked));
    s.end_step();
    let edited = l.store().to_plain();
    let hidden = hidden_faces(&edited);
    assert!(
        hidden.iter().any(|&h| h) && !hidden.iter().all(|&h| h),
        "some faces hidden"
    );
    same_state(l.store(), "the edited level");
    ok(l.set(2));
    assert!(l.store().any_hidden(), "the top hides the faces too");
    same_state(l.store(), "the top");
    ok(l.set(0));
    same_state(l.store(), "the base");
    ok(l.set(1));
    same_state(l.store(), "the edited level again");
    let back = l.store().to_plain();
    assert_eq!(hidden_faces(&back), hidden, "the same faces hidden");
    let mask = |m: &gs_protocol::PlainMesh| match m.attributes.get("mask").map(|a| &a.values) {
        Some(gs_protocol::AttrValues::F32(v)) => v.iter().map(|x| x.to_bits()).collect(),
        _ => Vec::new(),
    };
    let made = mask(&edited);
    assert!(made.iter().any(|&x| x != 0), "the pen masked some points");
    assert_eq!(mask(&back), made, "the same mask");
}
