//! The node sink (decision 0012): after a stroke, feeding the nodes it left dirty hands the
//! sink each one with its final positions and normals, the same at any thread count, and
//! feeding never changes the mesh.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use gs_core::{
    BrushPreset, ColumnSet, MeshStore, NodeConfig, NodeId, NodeSink, NodeView, PosReader,
    StrokeParams, StrokeSample, Vec3, View, stroke,
};

use crate::brush_kit::preset;
use crate::common::{fingerprint, ok, snap, sphere, store, surface_point};

/// Positions and normals, by node.
type Columns = BTreeMap<NodeId, (Vec<Vec3>, Vec<Vec3>)>;

/// The positions and normals each node was fed with.
#[derive(Debug, Default)]
struct Kept(Mutex<Columns>);

impl NodeSink for Kept {
    fn node(&self, node: &NodeView<'_>, _pos: &PosReader<'_>) {
        let mut kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        kept.insert(node.id, (node.pos.to_vec(), node.nrm.to_vec()));
    }
}

/// A store after one stroke of `brush` across the top of a lumpy sphere, and the nodes the
/// stroke left dirty for positions or normals.
fn stroked(brush: &BrushPreset) -> (MeshStore, Vec<NodeId>) {
    let mut s = store(&sphere(16, 0.6, 3, false), ok(NodeConfig::new(6, 40)));
    s.take_dirty();
    let c = surface_point(&s, Vec3::new(0.2, 0.9, 0.3));
    let side = c.cross(Vec3::new(0.3, 0.1, -0.9)).normalized();
    let samples: Vec<StrokeSample> = (0..8)
        .map(|k| StrokeSample {
            pos: surface_point(&s, c + side * (0.06 * k as f32)),
            pressure: 1.0,
            tilt: [0.0; 2],
            radius: 0.35,
        })
        .collect();
    let view = View {
        dir: -c.normalized(),
        eye: None,
    };
    ok(stroke(&mut s, brush, StrokeParams::new(view), &samples));
    let moved = (s.take_dirty().into_iter())
        .filter(|d| d.1.contains(ColumnSet::POSITION) || d.1.contains(ColumnSet::NORMAL))
        .map(|d| d.0)
        .collect();
    (s, moved)
}

/// What the sink got when the nodes in `nodes` were fed from `s` on `threads` threads.
fn fed(s: &MeshStore, nodes: &[NodeId], threads: usize) -> Columns {
    let kept = Kept::default();
    gs_par::with_threads(threads, || s.feed(nodes, &kept));
    kept.0.into_inner().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn feeding_hands_over_each_node_as_it_is() {
    for name in ["draw", "move"] {
        let (s, moved) = stroked(&preset(name).preset);
        assert!(!moved.is_empty(), "{name}: the stroke changed something");
        let before = fingerprint(&snap(&s));
        let kept = fed(&s, &moved, 4);
        assert_eq!(
            fingerprint(&snap(&s)),
            before,
            "{name}: feeding changes nothing"
        );
        assert_eq!(
            kept.keys().copied().collect::<Vec<_>>(),
            moved,
            "{name}: every node fed, once"
        );
        let bits = |v: &[Vec3]| v.iter().flat_map(|p| p.bits()).collect::<Vec<u32>>();
        for (&n, (pos, nrm)) in &kept {
            let node = s.node(n);
            assert!(
                bits(pos) == bits(node.pos) && bits(nrm) == bits(node.nrm),
                "{name}: node {n} arrived with its final data"
            );
        }
        let one = fed(&s, &moved, 1);
        let same = |a: &Columns, b: &Columns| {
            a.len() == b.len()
                && a.iter().zip(b).all(|(x, y)| {
                    x.0 == y.0 && bits(&x.1.0) == bits(&y.1.0) && bits(&x.1.1) == bits(&y.1.1)
                })
        };
        assert!(same(&one, &kept), "{name}: the same on 1 and 4 threads");
    }
}
