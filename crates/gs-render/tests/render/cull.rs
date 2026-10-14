//! GPU culling: never drops a chunk that could reach the screen (culled frames render the same
//! bytes as unculled ones), and does skip what cannot be seen (work counters).

use crate::common::{fitted, lavapipe, snap};
use crate::scene;
use crate::store::sphere_store;
use gs_protocol::{NO_VERTEX as TRI, PlainMesh};
use gs_render::{
    Camera, Drawn, Fit, GpuMesh, Image, Lens, Preset, Projection, RenderSize, Renderer, Shading,
    Sides, UploadOptions, View, Wire,
};
use proptest::prelude::*;
use std::f32::consts::PI;

fn chunked(r: &Renderer, mesh: &PlainMesh, chunk_triangles: u32) -> GpuMesh {
    r.upload(mesh, UploadOptions { chunk_triangles })
        .expect("uploads")
}

/// The same frame with culling on and off.
fn both_ways(r: &Renderer, mesh: &mut GpuMesh, view: &View) -> (Image, Image) {
    mesh.set_culling(true);
    let culled = snap(r, &*mesh, view, 64);
    mesh.set_culling(false);
    let full = snap(r, &*mesh, view, 64);
    mesh.set_culling(true);
    (culled, full)
}

/// A camera anywhere: around, close to, inside or far from the mesh, either projection.
fn any_camera(fit: Camera) -> impl Strategy<Value = Camera> {
    (
        -PI..PI,
        -1.5f32..1.5,
        0.05f32..2.0,
        prop::array::uniform3(-0.8f32..0.8),
        0.3f32..1.6,
        any::<bool>(),
    )
        .prop_map(
            move |(azimuth, elevation, zoom, shift, fov_y, ortho)| Camera {
                orientation: Camera::turned(azimuth, elevation),
                distance: fit.distance * zoom,
                target: shift,
                lens: Lens::from_fov(fov_y, Fit::Vertical),
                projection: if ortho {
                    Projection::Orthographic
                } else {
                    Projection::Perspective
                },
            },
        )
}

#[test]
fn culling_never_drops_a_visible_chunk() {
    let r = lavapipe();
    let mut grid = scene::grid(12);
    // Bend the open grid so some of its chunks face away from any camera.
    for p in &mut grid.positions {
        p[2] = 0.6 * p[0] * p[0] - 0.3;
    }
    let meshes = std::cell::RefCell::new([
        r.upload_store(&sphere_store(16, 6, 40)),
        chunked(r, &grid, 16),
    ]);
    let fit = Camera::fitted(Preset::Front, &meshes.borrow()[0].fit_points(), 1.0);
    proptest!(ProptestConfig::with_cases(24), |(camera in any_camera(fit))| {
        for mesh in meshes.borrow_mut().iter_mut() {
            for sides in [Sides::Both, Sides::Front] {
                mesh.set_sides(sides);
                for shading in [Shading::Normals, Shading::Zebra] {
                    let view = View::new(camera, shading);
                    let (culled, full) = both_ways(r, mesh, &view);
                    prop_assert!(culled == full, "{:?}: culling changed the picture", sides);
                }
            }
        }
    });
}

#[test]
fn culling_skips_what_cannot_be_seen() {
    let r = lavapipe();
    assert!(
        r.gpu().capabilities().multi_draw_indirect_count,
        "lavapipe can draw with a GPU-written count"
    );
    let mut mesh = r.upload_store(&sphere_store(24, 7, 60));
    let total = mesh.chunks().len() as u32;
    let front = fitted(&mesh, Preset::Front, Shading::Normals, Wire::Off);
    let count = |mesh: &GpuMesh, view: &View| drawn(r, mesh, view).chunks;
    assert_eq!(
        count(&mesh, &front),
        total,
        "a fitted view sees every chunk"
    );
    // Aimed past the sphere's side: about half of it is out of frame. (Aimed straight at it,
    // even close up, the frustum still holds most of the hidden far side.)
    let aside = View {
        camera: Camera {
            distance: front.camera.distance * 0.5,
            target: [1.5, 0.0, 0.0],
            ..front.camera
        },
        ..front
    };
    let framed = count(&mesh, &aside);
    assert!(
        framed > 0 && framed * 3 < total * 2,
        "looking past the side, the frustum keeps {framed} of {total} chunks"
    );
    mesh.set_sides(Sides::Front);
    let facing = count(&mesh, &front);
    assert!(
        facing > 0 && facing * 10 < total * 9,
        "drawing front faces, {facing} of {total} chunks may face the camera"
    );
    assert!(
        count(&mesh, &aside) < framed,
        "facing and frustum tests add up"
    );
    let ortho = View {
        camera: Camera {
            projection: Projection::Orthographic,
            ..front.camera
        },
        ..front
    };
    let facing_ortho = count(&mesh, &ortho);
    assert!(
        facing_ortho > 0 && facing_ortho * 10 < total * 9,
        "orthographic: {facing_ortho} of {total} chunks may face the camera"
    );
    // (That the pictures stay the same is `culling_never_drops_a_visible_chunk`.)
    // The frame really draws only the kept chunks: the clipper sees fewer triangles.
    let kept = drawn(r, &mesh, &aside);
    let total_triangles = u64::from(mesh.triangle_count());
    assert!(
        kept.triangles.is_some_and(|t| t > 0 && t < total_triangles),
        "a culled frame draws fewer triangles: {kept:?} of {total_triangles}"
    );
    mesh.set_culling(false);
    let all = drawn(r, &mesh, &aside);
    assert_eq!(
        all,
        Drawn {
            chunks: total,
            triangles: Some(u64::from(mesh.triangle_count()))
        },
        "culling off draws everything"
    );
    mesh.set_culling(true);
    let pulled = View {
        wire: Wire::VertexPulling,
        ..aside
    };
    assert_eq!(
        drawn(r, &mesh, &pulled),
        all,
        "the vertex-pulling wireframe draws everything"
    );
}

/// What a 64 x 64 frame of `view` draws.
fn drawn(r: &Renderer, mesh: &GpuMesh, view: &View) -> Drawn {
    r.count_drawn(mesh, view, RenderSize::square(64))
        .expect("counts")
}

/// Small two-triangle squares facing +Z, each its own chunk, centered at `centers` (z = 0).
fn squares(r: &Renderer, centers: &[[f32; 2]]) -> GpuMesh {
    let (mut positions, mut faces) = (Vec::new(), Vec::new());
    for &[x, y] in centers {
        let base = positions.len() as u32;
        let h = 0.1;
        positions.extend([
            [x - h, y - h, 0.0],
            [x + h, y - h, 0.0],
            [x + h, y + h, 0.0],
            [x - h, y + h, 0.0],
        ]);
        faces.push([base, base + 1, base + 2, base + 3]);
    }
    chunked(r, &scene::plain(positions, faces), 2)
}

/// A camera on +Z looking at `target` that frames x and y within 2 of it at z = 0.
fn framing(target: [f32; 3], projection: Projection) -> View<'static> {
    View::new(
        Camera {
            distance: 2.0,
            lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
            target,
            projection,
            ..Camera::preset(Preset::Front)
        },
        Shading::Clay,
    )
}

/// Each side plane culls exactly the chunks wholly beyond it, and keeps a chunk that pokes in
/// even a little. The clipper sees exactly the kept chunks' triangles.
#[test]
fn each_frustum_side_culls_exactly_what_is_beyond_it() {
    let r = lavapipe();
    // One square in the middle; on each side, one straddling the frame's edge (its center 0.05
    // outside, so a sliver shows) and one wholly outside.
    let mut centers = vec![[0.0, 0.0]];
    for d in [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
        centers.push([2.05 * d[0], 2.05 * d[1]]);
        centers.push([2.5 * d[0], 2.5 * d[1]]);
    }
    let mesh = squares(r, &centers);
    for projection in [Projection::Orthographic, Projection::Perspective] {
        let centered = drawn(r, &mesh, &framing([0.0; 3], projection));
        assert_eq!(
            centered,
            Drawn {
                chunks: 5,
                triangles: Some(10)
            },
            "{projection:?}: the middle and the four straddling squares"
        );
        // Moving the frame by 0.45 brings one outside square in on that side and leaves the
        // straddling one on the far side wholly out: the count stays 5 only if both planes
        // of the axis work.
        for (shift, axis) in [([0.45, 0.0, 0.0], "x"), ([0.0, 0.45, 0.0], "y")] {
            for sign in [1.0f32, -1.0] {
                let target = shift.map(|c: f32| c * sign);
                let moved = drawn(r, &mesh, &framing(target, projection));
                assert_eq!(
                    moved.chunks,
                    5,
                    "{projection:?}: frame moved along {axis} by {}",
                    0.45 * sign
                );
            }
        }
    }
}

/// The normal-cone test against the exact geometry: a chunk whose normals spread 30 degrees
/// around -Z, seen orthographically from +Z rotated by `angle`, faces wholly away until the
/// angle plus 30 degrees nears 90 (less the facing margin of about 0.9 degrees).
#[test]
fn cones_cull_up_to_their_half_angle() {
    let r = lavapipe();
    let (mut positions, mut faces) = (Vec::new(), Vec::new());
    // Chunk 0: four triangles facing -Z tilted +30 degrees about Y, four tilted -30.
    let (s, c) = (0.5f32, 0.866_025_4f32);
    for (k, side) in [1.0f32, -1.0].into_iter().flat_map(|t| [t; 4]).enumerate() {
        let base = positions.len() as u32;
        let y = k as f32 * 0.2;
        // Edges along Y, then along (c, 0, side * s): the normal is (side * s, 0, -c), 30
        // degrees off -Z toward +X or -X, away from a camera on +Z.
        positions.extend([
            [0.0, y, 0.0],
            [0.0, y + 0.1, 0.0],
            [0.1 * c, y, 0.1 * side * s],
        ]);
        faces.push([base, base + 1, base + 2, TRI]);
    }
    let mut mesh = chunked(r, &scene::plain(positions, faces), 8);
    mesh.set_sides(Sides::Front);
    let seen = |degrees: f32| {
        let view = View::new(
            Camera {
                orientation: Camera::turned(degrees.to_radians(), 0.0),
                projection: Projection::Orthographic,
                ..Camera::fitted(Preset::Front, &mesh.fit_points(), 1.0)
            },
            Shading::Clay,
        );
        drawn(r, &mesh, &view).chunks
    };
    for (degrees, expected) in [
        (0.0, 0),
        (50.0, 0),
        (-50.0, 0),
        (58.0, 0),
        (62.0, 1),
        (70.0, 1),
    ] {
        assert_eq!(
            seen(degrees),
            expected,
            "seen from {degrees} degrees off the axis"
        );
    }
}

/// One chunk of 8 triangles at `x`: `away` of them face -Z (away from a camera on +Z), the
/// rest +Z, each a small right triangle; `extra` replaces the last ones.
fn chunk(
    positions: &mut Vec<[f32; 3]>,
    faces: &mut Vec<[u32; 4]>,
    x: f32,
    away: usize,
    extra: &[[[f32; 3]; 3]],
) {
    for k in 0..8 - extra.len() {
        let (x0, y0) = (x + 0.1 * (k % 4) as f32, 0.1 * (k / 4) as f32);
        let mut t = [[x0, y0, 0.0], [x0 + 0.08, y0, 0.0], [x0, y0 + 0.08, 0.0]];
        if k < away {
            t.swap(1, 2);
        }
        let base = positions.len() as u32;
        positions.extend(t);
        faces.push([base, base + 1, base + 2, TRI]);
    }
    for t in extra {
        let base = positions.len() as u32;
        positions.extend(t.map(|p| [p[0] + x, p[1], p[2]]));
        faces.push([base, base + 1, base + 2, TRI]);
    }
}

/// Chunks whose triangles all face away are skipped when only front faces are drawn, unless a
/// triangle is too thin to trust its facing, or the chunk's normals spread past 87 degrees
/// from their axis (where the cone test stops being safe).
#[test]
fn only_trusted_cones_cull() {
    let r = lavapipe();
    let (mut positions, mut faces) = (Vec::new(), Vec::new());
    // Chunk 0 faces the camera; chunk 1 faces away.
    chunk(&mut positions, &mut faces, 0.0, 0, &[]);
    chunk(&mut positions, &mut faces, 1.0, 8, &[]);
    // Chunk 2 faces away, with a sliver whose area is about 1e-7 of its edges' product.
    let sliver = [[0.1, 0.3, 0.0], [0.4, 0.3, 0.0], [0.8, 0.3 + 1e-7, 0.0]];
    chunk(&mut positions, &mut faces, 2.0, 8, &[sliver]);
    // Chunk 3 faces the camera, but two triangles tilt 120 degrees off, so the cone's half
    // angle is past 90 degrees. Trusting it would skip a chunk that faces the camera.
    let (c, s) = (-0.5f32, 0.866_025_4f32);
    let tilted = |side: f32| {
        // Normal (side * s, 0, c): the triangle spans y and the direction (-c, 0, side * s).
        [
            [0.2, 0.3, 0.0],
            [0.2, 0.4, 0.0],
            [0.2 - 0.1 * c, 0.3, 0.1 * side * s],
        ]
    };
    chunk(
        &mut positions,
        &mut faces,
        3.0,
        0,
        &[tilted(1.0), tilted(-1.0)],
    );
    // Chunk 4 faces away, with a long sliver: 10 units long, its doubled area 1e-6, a
    // hundred-millionth of its long edge squared. Thinness is relative to the triangle's size.
    let long = [[0.0, 0.6, 0.0], [5.0, 0.6 + 1e-7, 0.0], [10.0, 0.6, 0.0]];
    chunk(&mut positions, &mut faces, 4.0, 8, &[long]);
    let plain = scene::plain(positions, faces);
    let mut mesh = chunked(r, &plain, 8);
    assert_eq!(mesh.chunks().len(), 5, "five chunks of eight triangles");
    let front = fitted(&mesh, Preset::Front, Shading::Clay, Wire::Off);
    mesh.set_sides(Sides::Front);
    assert_eq!(
        drawn(r, &mesh, &front).chunks,
        4,
        "only the chunk facing away with a trusted, narrow cone is skipped"
    );
    mesh.set_sides(Sides::Both);
    assert_eq!(
        drawn(r, &mesh, &front).chunks,
        5,
        "both sides drawn: nothing is skipped for facing away"
    );
}

/// Front-only drawing really hides back faces: an open grid seen from behind disappears.
#[test]
fn front_only_hides_the_back_of_an_open_mesh() {
    let r = lavapipe();
    let mut mesh = chunked(r, &scene::grid(4), 8);
    let back = fitted(&mesh, Preset::Back, Shading::Clay, Wire::Off);
    let seen = snap(r, &mesh, &back, 32);
    mesh.set_sides(Sides::Front);
    for culling in [true, false] {
        mesh.set_culling(culling);
        let hidden = snap(r, &mesh, &back, 32);
        assert!(
            hidden.rgba.chunks(4).all(|p| p == gs_render::BACKGROUND),
            "culling {culling}: only background behind a front-only grid"
        );
    }
    assert!(
        seen.rgba.chunks(4).any(|p| p != gs_render::BACKGROUND),
        "both sides drawn, the back shows"
    );
}
