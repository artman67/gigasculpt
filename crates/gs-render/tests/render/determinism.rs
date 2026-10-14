//! Same input, same bytes: renders on lavapipe (ARCHITECTURE sections 8, 10.3). MatCap bakes
//! are checked at two thread counts in `matcap.rs`.

use crate::common::{fitted, lavapipe, shipped, upload};
use crate::scene;
use gs_render::matcap::bake;
use gs_render::{Preset, RenderSize, Shading, Wire};

#[test]
fn same_render_twice_is_byte_identical() {
    let r = lavapipe();
    let mut sphere = scene::quad_sphere(12);
    scene::dent(&mut sphere, 0.12);
    // Two uploads of the same mesh: preparing the pools must be deterministic too.
    let (first, second) = (upload(r, &sphere), upload(r, &sphere));
    let matcap = r.upload_bake(&bake(&shipped("red-wax")).expect("bakes"));
    let shadings = [
        Shading::Clay,
        Shading::MatCap(&matcap),
        Shading::Zebra,
        Shading::Normals,
    ];
    let mut seen = Vec::new();
    for (shading, wire, samples) in shadings
        .iter()
        .flat_map(|&s| [(s, Wire::Off, 1), (s, Wire::VertexPulling, 4)])
    {
        let size = RenderSize {
            width: 96,
            height: 80,
            samples,
        };
        let a = r
            .snapshot(&first, &fitted(&first, Preset::Iso, shading, wire), size)
            .expect("renders");
        let b = r
            .snapshot(&second, &fitted(&second, Preset::Iso, shading, wire), size)
            .expect("renders");
        assert!(
            a == b,
            "{wire:?} at {samples} samples renders the same bytes twice"
        );
        assert!(
            !seen.contains(&a),
            "every shading, wire and sample setting looks different"
        );
        seen.push(a);
    }
}
