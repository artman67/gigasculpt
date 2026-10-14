//! A stroke begun inside a history step its caller already opened (an engine batch that edits
//! and then strokes) joins that step: it goes back to its own start, not the step's, and undo
//! takes back the edit and the stroke together.

use gs_core::{
    AttrVec, BrushPreset, ColumnSet, MeshStore, NodeId, Stroke, StrokeParams, StrokeSample, Vec3,
    View,
};
use gs_testkit::compare::canonical;
use gs_testkit::fixtures::icosphere;

use crate::{ok, store};

fn preset(text: &str) -> BrushPreset {
    ok(BrushPreset::from_toml(text))
}

const MOVE: &str = include_str!("../../../../assets/brushes/move.toml");
const MASK_PEN: &str = include_str!("../../../../assets/brushes/mask-pen.toml");
const DRAW: &str = include_str!("../../../../assets/brushes/draw.toml");

fn sample(pos: Vec3) -> StrokeSample {
    StrokeSample {
        pos,
        pressure: 1.0,
        tilt: [0.0; 2],
        radius: 0.4,
    }
}

fn params() -> StrokeParams {
    StrokeParams::new(View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: None,
    })
}

/// The store's mesh as canonical bytes: equal bytes, bit-identical meshes.
fn bytes(s: &MeshStore) -> Vec<u8> {
    canonical(&s.to_plain())
}

fn all(s: &MeshStore) -> Vec<NodeId> {
    (0..s.node_count() as NodeId).collect()
}

/// Opens a step and moves every vertex along x: the edit a batch makes before its stroke.
fn edited() -> (MeshStore, Vec<u8>, Vec<u8>) {
    let mut s = store(&icosphere(3));
    let original = bytes(&s);
    s.begin_step();
    let nodes = all(&s);
    s.write(&nodes, ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            *p += Vec3::new(0.1, 0.0, 0.0);
        }
    });
    let after_edit = bytes(&s);
    (s, original, after_edit)
}

#[test]
fn a_drag_inside_an_open_step_returns_to_its_own_start() {
    let (mut s, original, after_edit) = edited();
    let grab = Vec3::new(0.1, 0.0, 1.0);
    let mut k = ok(Stroke::begin(&mut s, &preset(MOVE), params()));
    ok(k.add(
        &mut s,
        &[sample(grab), sample(grab + Vec3::new(0.0, 0.5, 0.0))],
    ));
    assert_ne!(bytes(&s), after_edit, "the drag moved the surface");
    ok(k.add(&mut s, &[sample(grab)]));
    assert_eq!(
        bytes(&s),
        after_edit,
        "dragging back returns to where the stroke began, keeping the edit before it"
    );
    ok(k.add(&mut s, &[sample(grab + Vec3::new(0.0, -0.3, 0.0))]));
    let dragged = bytes(&s);
    let report = k.finish(&mut s);
    assert!(
        report.step.is_none(),
        "the stroke closed no step of its own"
    );
    assert!(s.step_open(), "the caller's step is still open");
    assert!(
        !report.nodes.is_empty(),
        "the report names the nodes written"
    );
    s.end_step();
    assert_eq!(s.undo_len(), 1, "the edit and the stroke are one step");
    s.undo();
    assert_eq!(bytes(&s), original, "undo takes back both");
    s.redo();
    assert_eq!(bytes(&s), dragged, "redo brings both back");
}

#[test]
fn cancelling_inside_an_open_step_takes_back_only_the_stroke() {
    let (mut s, original, after_edit) = edited();
    let mut k = ok(Stroke::begin(&mut s, &preset(DRAW), params()));
    let path = [Vec3::new(0.1, 0.0, 1.0), Vec3::new(0.1, 0.4, 0.95)];
    ok(k.add(&mut s, &path.map(sample)));
    assert_ne!(bytes(&s), after_edit, "the dabs raised the surface");
    k.cancel(&mut s);
    assert_eq!(bytes(&s), after_edit, "the edit before the stroke stays");
    assert!(s.step_open(), "the caller's step is still open");
    s.revert_step();
    assert_eq!(
        bytes(&s),
        original,
        "with the stroke gone, the step reverts to its own start"
    );
}

/// Node values of the mask at the stroke or step start, as plain numbers (zeros where none).
fn start_mask(s: &MeshStore, n: NodeId) -> Vec<f32> {
    let mask = s
        .attr_id("mask")
        .unwrap_or_else(|| panic!("no mask column"));
    match s.start_attr(mask, n) {
        Some(AttrVec::F32(v)) => v.clone(),
        Some(other) => panic!("the mask holds {other:?}"),
        None => vec![0.0; s.node(n).pos.len()],
    }
}

/// A node's mask as it is now, as plain numbers.
fn mask_now(s: &MeshStore, n: NodeId) -> Vec<f32> {
    let mask = s
        .attr_id("mask")
        .unwrap_or_else(|| panic!("no mask column"));
    match s.node(n).attr(mask) {
        Some(AttrVec::F32(v)) => v.clone(),
        _ => vec![0.0; s.node(n).pos.len()],
    }
}

#[test]
fn the_attribute_start_is_the_stroke_start_inside_a_step() {
    let mut s = store(&icosphere(3));
    s.begin_step();
    let pen = preset(MASK_PEN);
    // The first stroke in the step paints the front.
    let mut k = ok(Stroke::begin(&mut s, &pen, params()));
    let front = [Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.3, 0.95)];
    ok(k.add(&mut s, &front.map(sample)));
    let painted: Vec<NodeId> = k.report().nodes.clone();
    assert!(!painted.is_empty(), "the pen painted");
    assert_eq!(
        k.finish(&mut s).step,
        None,
        "inside a step a stroke closes nothing"
    );
    for &n in &painted {
        let start = start_mask(&s, n);
        assert!(
            start.iter().all(|&x| x == 0.0),
            "between strokes the start is the step's"
        );
        assert_ne!(start, mask_now(&s, n), "node {n} was painted since");
    }
    // The second stroke starts where the first ended.
    let k = ok(Stroke::begin(&mut s, &pen, params()));
    for &n in &painted {
        assert_eq!(start_mask(&s, n), mask_now(&s, n), "node {n}");
    }
    k.cancel(&mut s);
    s.end_step();
    for &n in &painted {
        assert_eq!(
            start_mask(&s, n),
            mask_now(&s, n),
            "with no step open the start is now"
        );
    }
}

#[test]
fn closing_or_cancelling_a_step_forgets_a_stroke_left_open_in_it() {
    // A stroke dropped without finishing (the engine rolls a failed call back this way) must
    // not leave its start behind for the next step.
    for close in [false, true] {
        let (mut s, original, _) = edited();
        let mut k = ok(Stroke::begin(&mut s, &preset(DRAW), params()));
        let path = [Vec3::new(0.1, 0.0, 1.0), Vec3::new(0.1, 0.4, 0.95)];
        ok(k.add(&mut s, &path.map(sample)));
        drop(k);
        if close {
            s.end_step();
        } else {
            s.cancel_step();
            assert_eq!(
                bytes(&s),
                original,
                "cancelling takes back the edit and the dabs"
            );
        }
        let now = bytes(&s);
        s.begin_step();
        let start = canonical(&plain_with(&s, s.start_pos()));
        assert_eq!(
            start, now,
            "the next step starts where the mesh is (closed: {close})"
        );
        s.cancel_step();
    }
}

/// The store's mesh with the positions `pos` reads.
fn plain_with(s: &MeshStore, pos: gs_core::PosReader<'_>) -> gs_protocol::PlainMesh {
    use gs_core::PosRead;
    let mut m = s.to_plain();
    let live = (0..s.node_count() as NodeId).flat_map(|n| {
        let node = s.node(n);
        (node.base..)
            .zip(node.pos.iter())
            .filter(|(_, p)| !p.is_dead())
            .map(|(v, _)| v)
    });
    for (p, v) in m.positions.iter_mut().zip(live) {
        *p = pos.get(v).to_array();
    }
    m
}
