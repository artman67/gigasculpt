//! Check and split (decision 0061 section 1, amended for UV2: read on the chart's projection
//! before UV3): no chart's box is more than 8 times longer than wide (research 28's wrong result
//! 5, LSCM drifting on thin charts).

use std::collections::BTreeMap;

use gs_geo::uv::Steer;
use gs_protocol::PlainMesh;
use gs_testkit::bench_shapes::ring;

use crate::{charts, groups, shapes, surface};

/// The longest box side over the shortest, of each chart's projection along its u and v.
fn boxes(mesh: &PlainMesh, steer: &Steer) -> Vec<f64> {
    let c = charts(mesh, steer);
    (c.meshes(surface(mesh)).iter())
        .map(|chart| {
            let uv = chart.project(surface(mesh));
            let extent = |k: usize| {
                let (lo, hi) = (uv.iter()).fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                    (lo.min(f64::from(p[k])), hi.max(f64::from(p[k])))
                });
                hi - lo
            };
            let (u, v) = (extent(0), extent(1));
            u.max(v) / u.min(v)
        })
        .collect()
}

fn assert_boxes(name: &str, mesh: &PlainMesh, steer: &Steer) -> usize {
    let boxes = boxes(mesh, steer);
    for (k, &ratio) in boxes.iter().enumerate() {
        assert!(
            ratio <= 8.0,
            "{name}: chart {k}'s box is {ratio} times longer than wide"
        );
    }
    boxes.len()
}

#[test]
fn no_charts_box_is_more_than_eight_times_longer_than_wide() {
    for (name, mesh) in shapes() {
        assert_boxes(name, &mesh, &Steer::default());
    }
    let ring = ring(2000);
    let groups = groups(&ring);
    let steer = Steer {
        groups: Some(&groups),
        ..Steer::default()
    };
    assert_boxes("ring with groups", &ring, &steer);
}

#[test]
fn a_long_flat_strip_is_split_into_short_charts() {
    let n = 40;
    let positions = (0..=n)
        .flat_map(|i| (0..2).map(move |j| [i as f32, 0.0, j as f32]))
        .collect();
    let faces = (0..n)
        .map(|i| [2 * i, 2 * i + 1, 2 * i + 3, 2 * i + 2])
        .collect();
    let strip = PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    };
    let count = assert_boxes("strip", &strip, &Steer::default());
    assert!(
        count >= 5,
        "40 squares in a row need 5 charts at least: {count}"
    );
}
