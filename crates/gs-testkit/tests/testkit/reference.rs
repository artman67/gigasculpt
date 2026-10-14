//! The effect reference reports exactly which parts and vertices changed.

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::Scene;
use gs_testkit::reference::{Effect, effect};

use crate::data::tetra;

/// The tetrahedron with a vertex column and a face column.
fn tagged() -> PlainMesh {
    let mut mesh = tetra();
    let column = |domain, values| Attribute { domain, values };
    let mask = column(
        AttrDomain::Vertex,
        AttrValues::F32(vec![0.0, 0.25, 0.5, 1.0]),
    );
    let group = column(AttrDomain::Face, AttrValues::U32(vec![1, 2, 3, 4]));
    mesh.attributes.insert("mask".into(), mask);
    mesh.attributes.insert("group".into(), group);
    mesh
}

/// The effect of changing part `a` of a one-part scene from `tagged()` with `change`:
/// (parts, topology changed, vertices touched).
fn changed(change: impl Fn(&mut PlainMesh)) -> (Vec<String>, bool, u64) {
    let before = scene(&[("a", tagged())]);
    let mut after = before.clone();
    if let Some(a) = after.get_mut("a") {
        change(a);
    }
    let got = effect(&before, &after);
    (got.parts, got.topology_changed, got.verts_touched)
}

/// A change that sets value `i` of column `name` to the bits `x`.
fn set(name: &'static str, i: usize, x: u32) -> impl Fn(&mut PlainMesh) {
    move |m| match m.attributes.get_mut(name).map(|c| &mut c.values) {
        Some(AttrValues::F32(v)) => v[i] = f32::from_bits(x),
        Some(AttrValues::U32(v)) => v[i] = x,
        None => {}
    }
}

#[test]
fn attribute_changes_alone_touch_vertices() {
    let a = || vec!["a".to_string()];
    assert_eq!(
        changed(set("mask", 2, 7)),
        (a(), false, 1),
        "one vertex value"
    );
    let corners = tetra().faces[1].iter().filter(|&&v| v != u32::MAX).count() as u64;
    assert_eq!(
        changed(set("group", 1, 9)),
        (a(), false, corners),
        "one face value: its corners"
    );
    let negative_zero = set("mask", 0, (-0.0f32).to_bits());
    assert_eq!(
        changed(negative_zero),
        (a(), false, 1),
        "0.0 to -0.0 is a change"
    );
    let dropped = |m: &mut PlainMesh| {
        m.attributes.remove("group");
    };
    assert_eq!(
        changed(dropped),
        (a(), false, 4),
        "a column removed: every vertex"
    );
    let retyped = |m: &mut PlainMesh| {
        if let Some(c) = m.attributes.get_mut("mask") {
            c.values = AttrValues::U32(vec![
                0,
                0.25f32.to_bits(),
                0.5f32.to_bits(),
                1.0f32.to_bits(),
            ]);
        }
    };
    assert_eq!(
        changed(retyped),
        (a(), false, 4),
        "same bits, new type: every vertex"
    );
    assert_eq!(changed(|_| {}), (Vec::new(), false, 0), "nothing changed");
}

fn scene(parts: &[(&str, gs_protocol::PlainMesh)]) -> Scene {
    parts
        .iter()
        .map(|(n, m)| ((*n).to_string(), m.clone()))
        .collect()
}

#[test]
fn nothing_changed_means_an_empty_effect() {
    let before = scene(&[("a", tetra()), ("b", tetra())]);
    let none = Effect {
        parts: Vec::new(),
        topology_changed: false,
        verts_touched: 0,
        bbox: None,
    };
    assert_eq!(effect(&before, &before.clone()), none, "same scene");
}

#[test]
fn moved_vertices_are_counted_and_bounded() {
    let before = scene(&[("a", tetra()), ("b", tetra())]);
    let mut after = before.clone();
    if let Some(b) = after.get_mut("b") {
        b.positions[1] = [7.0, 8.0, 9.0];
        b.positions[3][2] = -0.0 - b.positions[3][2];
    }
    let got = effect(&before, &after);
    let want = Effect {
        parts: vec!["b".into()],
        topology_changed: false,
        verts_touched: 2,
        bbox: Some([[0.5, -1.25, -5.0], [7.0, 8.0, 9.0]]),
    };
    assert_eq!(got, want, "two vertices of b moved");
    let mut signed = before.clone();
    if let Some(a) = signed.get_mut("a") {
        a.positions[2][0] = -0.0;
    }
    let mut zero = before.clone();
    if let Some(a) = zero.get_mut("a") {
        a.positions[2][0] = 0.0;
    }
    assert_eq!(
        effect(&zero, &signed).verts_touched,
        1,
        "0.0 to -0.0 is a change"
    );
}

#[test]
fn added_removed_and_rewired_parts_change_topology() {
    let before = scene(&[("a", tetra())]);
    let added = effect(&before, &scene(&[("a", tetra()), ("b", tetra())]));
    assert_eq!(
        (
            added.parts.clone(),
            added.topology_changed,
            added.verts_touched
        ),
        (vec!["b".to_string()], true, 4),
        "a new part: all its vertices"
    );
    let removed = effect(&before, &scene(&[]));
    assert_eq!(
        (
            removed.parts,
            removed.topology_changed,
            removed.verts_touched
        ),
        (vec!["a".to_string()], true, 4),
        "a removed part: all its old vertices"
    );
    assert!(removed.bbox.is_some(), "bounded by the old vertices");
    let mut rewired = tetra();
    rewired.faces.swap(0, 1);
    let got = effect(&before, &scene(&[("a", rewired)]));
    assert_eq!(
        (got.topology_changed, got.verts_touched),
        (true, 4),
        "faces rewired"
    );
    let mut grown = tetra();
    grown.positions.push([0.0, 0.0, 0.0]);
    let got = effect(&before, &scene(&[("a", grown)]));
    assert_eq!(
        (got.topology_changed, got.verts_touched),
        (true, 5),
        "a vertex added"
    );
}
