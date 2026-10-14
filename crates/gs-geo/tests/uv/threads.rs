//! Determinism (decision 0061 section 3): the bench head gives the same charts and seams at 1
//! and 24 threads.

use gs_geo::uv::{Charts, Steer, Symmetry};
use gs_testkit::bench_shapes::head;

use crate::charts;

#[test]
fn the_head_cuts_the_same_at_one_and_twenty_four_threads() {
    let mesh = head(2000);
    let hint: Vec<f32> = (mesh.positions.iter())
        .map(|p| if p[2] > 0.8 { 0.5 } else { 0.0 })
        .collect();
    let steers = [
        Steer::default(),
        Steer {
            hint: Some(&hint),
            symmetry: Some(Symmetry {
                axis: 0,
                tolerance: 1e-4,
            }),
            ..Steer::default()
        },
    ];
    for steer in &steers {
        let at = |threads| gs_par::with_threads(threads, || charts(&mesh, steer));
        let (one, many): (Charts, Charts) = (at(1), at(24));
        assert_eq!(one.chart, many.chart, "the same chart per face");
        assert_eq!(one.seams, many.seams, "the same seams");
        assert_eq!(one.added_seams, many.added_seams, "the same added seams");
        assert!(one.count > 1, "the head is cut: {} charts", one.count);
    }
}
