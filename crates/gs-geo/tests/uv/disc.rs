//! Cut to a disc (decision 0061 section 1): after the cut every chart is a disc, one boundary
//! loop and Euler characteristic 1, above all on the ring (genus 1, its bands annuli and tubes
//! with groups on) and the sleeve (a tube).

use std::collections::BTreeMap;

use gs_geo::uv::{Chart, Corners, Steer};
use gs_protocol::{PlainMesh, corners};
use gs_testkit::bench_shapes::sphere;

use crate::{charts, groups, shapes, surface};

/// The root of `i`'s group.
fn find(root: &mut [usize], i: usize) -> usize {
    if root[i] != i {
        root[i] = find(root, root[i]);
    }
    root[i]
}

/// A chart's Euler characteristic `V - E + F` and its boundary loops, counted on its own mesh.
fn topology(chart: &Chart) -> (i64, usize) {
    let mut uses: BTreeMap<[u32; 2], u32> = BTreeMap::new();
    for face in &chart.corners {
        let c = corners(face);
        for k in 0..c.len() {
            let (a, b) = (c[k], c[(k + 1) % c.len()]);
            *uses.entry([a.min(b), a.max(b)]).or_default() += 1;
        }
    }
    let mut root: Vec<usize> = (0..chart.points.len()).collect();
    let mut on_loop = vec![false; chart.points.len()];
    for (&[a, b], &n) in &uses {
        if n == 1 {
            on_loop[a as usize] = true;
            on_loop[b as usize] = true;
            let (a, b) = (find(&mut root, a as usize), find(&mut root, b as usize));
            root[a.max(b)] = a.min(b);
        }
    }
    let loops = (0..root.len())
        .filter(|&p| on_loop[p] && find(&mut root, p) == p)
        .count();
    let count = |n: usize| i64::try_from(n).unwrap_or_else(|_| panic!("{n} fits an i64"));
    let euler = count(chart.points.len()) - count(uses.len()) + count(chart.faces.len());
    (euler, loops)
}

fn assert_discs(name: &str, mesh: &PlainMesh, steer: &Steer) -> u32 {
    let c = charts(mesh, steer);
    for chart in c.meshes(surface(mesh)) {
        assert_eq!(
            topology(&chart),
            (1, 1),
            "{name}: the chart from face {} is a disc",
            chart.faces[0]
        );
    }
    c.count
}

#[test]
fn every_chart_is_a_disc_after_the_cut() {
    for (name, mesh) in shapes() {
        assert_discs(name, &mesh, &Steer::default());
    }
}

#[test]
fn the_rings_bands_are_cut_open_with_groups_on() {
    let ring = gs_testkit::bench_shapes::ring(2000);
    let groups = groups(&ring);
    let steer = Steer {
        groups: Some(&groups),
        ..Steer::default()
    };
    let count = assert_discs("ring with groups", &ring, &steer);
    assert!(count >= 8, "at least one chart per band: {count}");
}

#[test]
fn a_closed_island_kept_whole_is_cut_into_discs() {
    let mesh = sphere(600);
    // Every corner at one UV: no seams, so the whole sphere is one kept island.
    let keep: Vec<Corners> = vec![[[0.25, 0.5]; 4]; mesh.faces.len()];
    let steer = Steer {
        keep: Some(&keep),
        ..Steer::default()
    };
    let count = assert_discs("sphere kept whole", &mesh, &steer);
    assert!(
        count >= 2,
        "a closed island needs two charts at least: {count}"
    );
}
