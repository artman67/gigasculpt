//! Every stroke class runs end to end, from pen samples to the GPU (catalog
//! `core.stroke-pipeline`): a dab stroke (Draw), a drag (Move), an anchored stroke (Draw with
//! the anchored class) and a curve (Clay Line, stamping its square) take one pen sample at a
//! time through gs-core's stroke; the store's dirty nodes go to the GPU after every sample and
//! at the stroke's end, as the window sends them. Every update sends exactly the nodes that
//! moved, never the whole mesh, and leaves the GPU holding what a fresh upload holds.

use std::sync::Arc;

use gs_core::alpha::{Pyramid, Recipe};
use gs_core::brush::{Smoothing, StrokeSpec};
use gs_core::{BrushPreset, MeshStore, Stroke, Vec3};
use gs_render::Sides;

use crate::common::lavapipe;
use crate::store::{holds_fresh, moved, path, preset, sphere_store};

/// The pyramid of the shipped alpha `p` names, the way the engine resolves it.
fn alpha(p: &BrushPreset) -> Option<Arc<Pyramid>> {
    let spec = p.tip.alpha.as_ref()?;
    let file = format!(
        "{}/../../assets/alphas/{}.toml",
        env!("CARGO_MANIFEST_DIR"),
        spec.image
    );
    let text = std::fs::read_to_string(&file).expect("the alpha recipe reads");
    let recipe = Recipe::from_toml(&text).expect("the recipe parses");
    Some(Arc::new(Pyramid::build(
        &recipe.draw().expect("draws"),
        spec,
    )))
}

#[test]
fn every_stroke_class_sends_only_the_nodes_it_moved_from_pen_to_gpu() {
    let r = lavapipe();
    let mut anchored = preset("draw");
    anchored.stroke = StrokeSpec::Anchored {
        follow: false,
        tap: 0.5,
        smoothing: Smoothing::default(),
    };
    let classes = [
        ("dab", preset("draw")),
        ("drag", preset("move")),
        ("anchored", anchored),
        ("curve", preset("clay-line")),
    ];
    for (class, brush) in classes {
        let mut store = sphere_store(40, 8, 200);
        store.take_dirty();
        let mut mesh = r.upload_store(&store);
        mesh.set_sides(Sides::Front);
        let start = r.read_pools(&mesh).expect("reads back");
        let mut sent = 0;
        let mut send = |store: &mut MeshStore, what: &str| {
            let dirty = store.take_dirty();
            let nodes = moved(&dirty).len();
            let upload = r.update_store(&mut mesh, store, &dirty);
            let what = format!("{class}, {what}");
            assert!(!upload.full, "{what}: not everything");
            assert_eq!(
                upload.nodes, nodes,
                "{what}: the moved nodes, and only those"
            );
            assert!(
                2 * nodes < store.node_count(),
                "{what}: {nodes} of {} nodes",
                store.node_count()
            );
            holds_fresh(r, &mesh, store, &what);
            sent += nodes;
        };
        let (samples, mut params) = path(Vec3::new(0.2, 0.9, 0.4), 0.3);
        params.alpha = alpha(&brush);
        let mut stroke = Stroke::begin(&mut store, &brush, params).expect("begins");
        for (k, sample) in samples.iter().enumerate() {
            stroke
                .add(&mut store, std::slice::from_ref(sample))
                .expect("takes the sample");
            send(&mut store, &format!("sample {k}"));
        }
        stroke.finish(&mut store);
        send(&mut store, "the stroke's end");
        assert!(sent > 0, "{class}: the stroke sent nodes");
        assert!(
            r.read_pools(&mesh).expect("reads back") != start,
            "{class}: the stroke changed the pools, so the comparison can fail"
        );
    }
}
