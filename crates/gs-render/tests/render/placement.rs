//! A placed mesh (decision 0029) draws where its transform puts it: the same picture as the
//! mesh with its points moved there on the CPU, culled from the part's own view, and a mesh
//! that is not shown draws nothing.

use crate::common::{covered, flip, lavapipe, snap};
use crate::scene;
use gs_core::transform::Similarity;
use gs_core::{Quat, Vec3};
use gs_render::{Camera, Preset, Shading, Sides, UploadOptions, View};

#[test]
fn a_placed_mesh_draws_where_its_points_moved_there_would() {
    let r = lavapipe();
    let mut mesh = scene::icosphere(3);
    scene::dent(&mut mesh, 0.12);
    let t = Similarity {
        translation: Vec3::new(3.0, -1.0, 2.0),
        rotation: Quat::about(Vec3::new(0.3, 1.0, -0.2).normalized(), 0.9),
        scale: 1.5,
    };
    let mut baked = mesh.clone();
    for p in &mut baked.positions {
        *p = t.point(Vec3::from_array(*p)).to_array();
    }
    // Small chunks, front faces only: the culling pass tests chunks by place and facing.
    let options = UploadOptions {
        chunk_triangles: 64,
    };
    let upload = |m| {
        let mut gpu = r.upload(m, options).expect("uploads");
        gpu.set_sides(Sides::Front);
        gpu
    };
    let (moved, baked, still) = (upload(&mesh).placed(t, true), upload(&baked), upload(&mesh));
    // Close enough that part of the mesh is outside the frame, so the side planes cull; the
    // move is far enough that culling the part's chunks from the scene's view would drop them.
    let fit = Camera::fitted(Preset::Iso, &baked.fit_points(), 1.0);
    let camera = Camera {
        distance: fit.distance * 0.6,
        ..fit
    };
    // Normals as color: shows that normals turn with the part. No occlusion, whose radius
    // follows the chunk boxes, which a turn makes larger than the baked mesh's.
    let view = View::new(camera, Shading::Normals);
    let (drawn, expected) = (snap(r, &moved, &view, 128), snap(r, &baked, &view, 128));
    let (_, off) = flip(&expected, &drawn);
    assert!(covered(&expected) > 4000, "the mesh fills the frame");
    assert!(off <= covered(&expected) / 200, "{off} pixels differ");
    let (_, apart) = flip(&expected, &snap(r, &still, &view, 128));
    assert!(
        apart > covered(&expected) / 4,
        "the transform moves the picture"
    );
    let hidden = moved.placed(t, false);
    assert_eq!(
        covered(&snap(r, &hidden, &view, 128)),
        0,
        "a hidden mesh draws nothing"
    );
}
