//! Whole faces (decision 0061 section 2, wrong result 2): every face is in exactly one chart, a
//! chart border is a face border marked on both faces, and charts are numbered by their lowest
//! face (section 3).

use gs_geo::uv::Steer;
use gs_protocol::arity;

use crate::{bit, charts, edges, shapes, surface};

#[test]
fn every_face_is_in_one_chart_numbered_by_its_lowest_face() {
    for (name, mesh) in shapes() {
        let c = charts(&mesh, &Steer::default());
        assert_eq!(c.chart.len(), mesh.faces.len(), "{name}: a chart per face");
        let mut lowest = vec![None; c.count as usize];
        for (f, &k) in c.chart.iter().enumerate() {
            assert!(k < c.count, "{name}: face {f}'s chart {k} is numbered");
            lowest[k as usize].get_or_insert(f);
        }
        let lowest: Vec<usize> = lowest.into_iter().flatten().collect();
        assert_eq!(lowest.len(), c.count as usize, "{name}: no chart is empty");
        assert!(
            lowest.windows(2).all(|w| w[0] < w[1]),
            "{name}: charts numbered in the order of their lowest faces"
        );
    }
}

#[test]
fn every_chart_border_is_a_seam_on_both_its_faces() {
    for (name, mesh) in shapes() {
        let c = charts(&mesh, &Steer::default());
        for (edge, sides) in edges(&mesh.faces) {
            let &[(f, j), (g, k)] = sides.as_slice() else {
                continue;
            };
            let (here, there) = (bit(&c.seams, f, j), bit(&c.seams, g, k));
            assert_eq!(
                here, there,
                "{name}: edge {edge:?} marked alike on both faces"
            );
            if c.chart[f] != c.chart[g] {
                assert!(here, "{name}: edge {edge:?} between two charts is a seam");
            }
        }
        for (f, face) in mesh.faces.iter().enumerate() {
            assert_eq!(
                c.seams[f] >> arity(*face),
                0,
                "{name}: face {f} marks only its edges"
            );
        }
    }
}

#[test]
fn each_chart_mesh_holds_its_faces_corners() {
    for (name, mesh) in shapes() {
        let c = charts(&mesh, &Steer::default());
        let pieces = c.meshes(surface(&mesh));
        assert_eq!(pieces.len(), c.count as usize, "{name}: a mesh per chart");
        let mut seen = vec![false; mesh.faces.len()];
        for (number, chart) in pieces.iter().enumerate() {
            for (i, &f) in chart.faces.iter().enumerate() {
                let f = f as usize;
                assert_eq!(
                    c.chart[f] as usize, number,
                    "{name}: face {f} in its chart's mesh"
                );
                assert!(!seen[f], "{name}: face {f} in one chart mesh only");
                seen[f] = true;
                for k in 0..arity(mesh.faces[f]) {
                    let point = chart.points[chart.corners[i][k] as usize];
                    assert_eq!(
                        point, mesh.faces[f][k],
                        "{name}: face {f} corner {k}'s point"
                    );
                }
            }
        }
        assert!(
            seen.iter().all(|&s| s),
            "{name}: every face in a chart mesh"
        );
    }
}
