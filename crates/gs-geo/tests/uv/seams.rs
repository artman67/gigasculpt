//! `uv::seams` (decision 0061 section 8): a seam is an edge whose two faces hold different UVs
//! at its ends, found on a hand-made layout and on a layout made from the cutter's charts, whose
//! seams it must give back exactly.

use std::collections::BTreeMap;

use gs_geo::uv::{Corners, Steer, seams};
use gs_protocol::PlainMesh;
use gs_testkit::bench_shapes::head;

use crate::{charts, surface};

/// Three quads in a row along x, facing up.
fn row() -> PlainMesh {
    let at = |i: u32, j: u32| 2 * i + j;
    let positions = (0..4)
        .flat_map(|i| (0..2).map(move |j| [i as f32, 0.0, j as f32]))
        .collect();
    let faces = (0..3)
        .map(|i| [at(i, 0), at(i, 1), at(i + 1, 1), at(i + 1, 0)])
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

#[test]
fn an_edge_is_a_seam_when_its_faces_differ_at_one_end() {
    let mesh = row();
    let mut uvs: Vec<Corners> = (mesh.faces.iter())
        .map(|face| face.map(|v| [mesh.positions[v as usize][0], mesh.positions[v as usize][2]]))
        .collect();
    assert_eq!(
        seams(&mesh.faces, &uvs),
        vec![0, 0, 0],
        "one island has no seams"
    );
    // Face 2's corner 1 is the point at x = 2, z = 1, which face 1 holds as its corner 2.
    uvs[2][1] = [2.0, 1.5];
    assert_eq!(
        seams(&mesh.faces, &uvs),
        vec![0, 1 << 2, 1],
        "the edge between faces 1 and 2 is a seam on both, the rest are not"
    );
}

#[test]
fn a_layout_made_from_the_charts_has_exactly_their_seams() {
    let mesh = head(2000);
    let c = charts(&mesh, &Steer::default());
    let mut uvs: Vec<Corners> = vec![[[0.0; 2]; 4]; mesh.faces.len()];
    let mut wedge = 0.0_f32;
    for (number, chart) in c.meshes(surface(&mesh)).iter().enumerate() {
        // Every wedge its own UV.
        let own: Vec<[f32; 2]> = (0..chart.points.len())
            .map(|_| {
                wedge += 1.0;
                [wedge, number as f32]
            })
            .collect();
        chart.write(&own, &mut uvs);
    }
    assert_eq!(
        seams(&mesh.faces, &uvs),
        c.seams,
        "the layout's seams are the charts'"
    );
}
