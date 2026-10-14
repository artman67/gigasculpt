//! Every mask value a part stores is on the 2^-24 grid (decision 0015, amended 2026-10-13), seen
//! through the engine: a copy made through `part.create` of a top whose mask was painted below it
//! holds the same mask bits, and Voxel Remesh, Quad Remesh and Decimate of a soft-masked part
//! store their carried masks on the grid.

use gs_core::mask::settle;
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::Session;
use gs_testkit::compare::bits_by_position;
use serde_json::json;

use crate::support::agent;

/// The mask of `m`.
fn mask(m: &PlainMesh) -> &[f32] {
    match m.attributes.get("mask") {
        Some(Attribute {
            values: AttrValues::F32(v),
            ..
        }) => v,
        _ => panic!("no mask"),
    }
}

/// How many of `m`'s mask values are off the grid, after checking some are soft.
fn off_grid(m: &PlainMesh, what: &str) -> usize {
    let values = mask(m);
    let soft = values.iter().any(|&x| x > 0.0 && x < 1.0);
    assert!(soft, "{what}: the mask holds soft values");
    (values.iter())
        .filter(|x| settle(**x).to_bits() != x.to_bits())
        .count()
}

#[test]
fn a_copy_through_part_create_keeps_a_mask_painted_below_the_top_to_the_bit() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    s.ok("level.set", json!({ "level": 1 }));
    let samples = [[-0.2, 0.0, 1.0], [0.2, 0.0, 1.0]].map(|p| json!({ "position": p }));
    let pen = json!({ "part": "sphere", "brush": "Mask Pen", "size": 0.8, "samples": samples });
    s.ok("brush.stroke", pen);
    s.ok("level.set", json!({ "level": 3 }));
    let top = s.mesh("sphere");
    assert_eq!(
        off_grid(&top, "the top"),
        0,
        "the carried mask is on the grid"
    );
    s.add_part("copy", &top);
    assert!(
        bits_by_position(&s.mesh("copy"), "mask") == bits_by_position(&top, "mask"),
        "the copy holds the top's mask bits"
    );
}

/// A sphere part named `soft` whose mask rises from 0 to 1 with y.
fn soft_sphere(s: &mut Session) {
    s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let mut mesh = s.mesh("sphere");
    let ramp = (mesh.positions.iter())
        .map(|p| f32::midpoint(p[1], 1.0))
        .collect();
    let (domain, values) = (AttrDomain::Vertex, AttrValues::F32(ramp));
    mesh.attributes
        .insert("mask".into(), Attribute { domain, values });
    s.add_part("soft", &mesh);
}

#[test]
fn voxel_remesh_quad_remesh_and_decimate_store_a_soft_mask_on_the_grid() {
    for (command, args) in [
        (
            "mesh.voxel_remesh",
            json!({ "part": "soft", "resolution": 40 }),
        ),
        ("mesh.quad_remesh", json!({ "part": "soft", "faces": 300 })),
        ("mesh.decimate", json!({ "part": "soft", "percent": 25 })),
    ] {
        let mut s = agent();
        soft_sphere(&mut s);
        s.ok(command, args);
        assert_eq!(off_grid(&s.mesh("soft"), command), 0, "{command}");
    }
}
