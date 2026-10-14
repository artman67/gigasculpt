//! Wireframe overlay: quad edges only, and the two paths agree.

use crate::common::{adapters, diff, fitted, lavapipe, luma, snap, upload};
use crate::scene;
use gs_protocol::NO_VERTEX as TRI;
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_render::UploadOptions;
use gs_render::{
    Camera, Fit, Image, Lens, Preset, Projection, Renderer, Shading, View, Wire, WireStyle,
};

const N: u32 = 8;
const SIZE: u32 = 256;

/// The 8 x 8 grid exactly filling a 256-pixel frame, so quad k spans pixels 32k..32k+32.
fn grid_view<'a>(wire: Wire) -> View<'a> {
    let fov_y = std::f32::consts::FRAC_PI_2;
    View {
        wire,
        ..View::new(
            Camera {
                distance: 1.0,
                lens: Lens::from_fov(fov_y, Fit::Vertical),
                projection: Projection::Orthographic,
                ..Camera::preset(Preset::Front)
            },
            Shading::Clay,
        )
    }
}

/// How many probe pixels are clearly darker with the wireframe than without it.
fn lines_at(wired: &Image, plain: &Image, probes: &[(u32, u32)]) -> usize {
    probes
        .iter()
        .filter(|&&(x, y)| luma(wired.pixel(x, y)) + 60 < luma(plain.pixel(x, y)))
        .count()
}

fn wired_grid(r: &Renderer, faces: Vec<[u32; 4]>, wire: Wire) -> (Image, Image) {
    let mesh = upload(r, &scene::plain(scene::grid(N).positions, faces));
    (
        snap(r, &mesh, &grid_view(wire), SIZE),
        snap(r, &mesh, &grid_view(Wire::Off), SIZE),
    )
}

type Pixels = Vec<(u32, u32)>;

/// Interior quad-edge midpoints and quad centers (where each diagonal crosses the middle).
fn probes() -> (Pixels, Pixels) {
    let cell = SIZE / N;
    let edges = (1..N)
        .flat_map(|i| {
            (0..N).flat_map(move |j| {
                [
                    (i * cell, j * cell + cell / 2),
                    (j * cell + cell / 2, i * cell),
                ]
            })
        })
        .collect();
    let centers = (0..N)
        .flat_map(|i| (0..N).map(move |j| (i * cell + cell / 2, j * cell + cell / 2)))
        .collect();
    (edges, centers)
}

#[test]
fn quad_diagonals_are_hidden_triangle_edges_are_not() {
    let r = lavapipe();
    let quads = scene::grid(N).faces;
    let triangles: Vec<[u32; 4]> = quads
        .iter()
        .flat_map(|&[a, b, c, d]| [[a, b, c, TRI], [a, c, d, TRI]])
        .collect();
    let (edges, centers) = probes();
    let (wired, plain) = wired_grid(r, quads, Wire::VertexPulling);
    assert_eq!(
        lines_at(&wired, &plain, &edges),
        edges.len(),
        "every quad edge is drawn"
    );
    assert_eq!(
        lines_at(&wired, &plain, &centers),
        0,
        "no quad diagonal is drawn"
    );
    let (wired, plain) = wired_grid(r, triangles, Wire::VertexPulling);
    assert_eq!(
        lines_at(&wired, &plain, &edges),
        edges.len(),
        "every triangle edge is drawn"
    );
    assert_eq!(
        lines_at(&wired, &plain, &centers),
        centers.len(),
        "triangles show their diagonals"
    );
}

/// Every pixel more than 3 pixels from every quad edge: a line there is drawn where no edge is.
fn far_from_edges() -> Pixels {
    let cell = SIZE / N;
    let far = |p: u32| (p % cell).min(cell - 1 - p % cell) >= 3;
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|&(x, y)| far(x) && far(y))
        .collect()
}

/// The 8 x 8 grid in two face groups: 5 left of the middle, 9 right of it.
fn grouped_grid() -> PlainMesh {
    let mut mesh = scene::grid(N);
    let ids = (mesh.faces.iter())
        .map(|f| {
            let x: f32 = f.iter().map(|&v| mesh.positions[v as usize][0]).sum();
            if x < 0.0 { 5 } else { 9 }
        })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(ids),
    };
    mesh.attributes.insert("group".into(), column);
    mesh
}

/// Fill off leaves only the edges: the wire changes no pixel away from them. Line off leaves
/// only the group colors: the picture is the face groups' own, byte for byte. Both draw the
/// edges over the group colors (catalog item `canvas.wireframe-view`).
#[test]
fn fill_off_leaves_only_edges_and_line_off_only_group_colors() {
    let r = lavapipe();
    let mesh = upload(r, &grouped_grid());
    let (edges, _) = probes();
    let far = far_from_edges();
    let styled = |line, fill| View {
        wire_style: WireStyle {
            line,
            fill,
            ..WireStyle::default()
        },
        ..grid_view(Wire::Auto)
    };
    let shot = |view: &View| snap(r, &mesh, view, SIZE);
    let plain = shot(&grid_view(Wire::Off));
    let groups = shot(&View {
        face_groups: true,
        ..grid_view(Wire::Off)
    });
    assert!(
        far.iter()
            .all(|&(x, y)| groups.pixel(x, y) != plain.pixel(x, y)),
        "the groups have colors"
    );
    let same_far = |a: &Image, b: &Image| far.iter().all(|&(x, y)| a.pixel(x, y) == b.pixel(x, y));
    let lines = shot(&styled(true, false));
    assert_eq!(
        lines_at(&lines, &plain, &edges),
        edges.len(),
        "Fill off: every edge"
    );
    assert!(same_far(&lines, &plain), "Fill off: nothing but the edges");
    let filled = shot(&styled(false, true));
    assert!(
        filled.rgba == groups.rgba,
        "Line off: the group colors alone"
    );
    let both = shot(&styled(true, true));
    // The group colors are darker than the clay, so an edge is a quarter darker there (the
    // probes sit half a pixel off each edge) rather than 60 levels.
    let darker = (edges.iter())
        .filter(|&&(x, y)| 4 * luma(both.pixel(x, y)) < 3 * luma(groups.pixel(x, y)))
        .count();
    assert_eq!(darker, edges.len(), "both: every edge");
    assert!(
        same_far(&both, &groups),
        "both: the group colors between the edges"
    );
}

/// The density view colors polygons large on screen red and small ones blue: the 4 x 4 grid's
/// 64-pixel quads and the 128 x 128 grid's 2-pixel quads, each filling the frame.
#[test]
fn density_view_colors_large_polygons_red_and_small_ones_blue() {
    let r = lavapipe();
    let channels = |n| {
        let mesh = upload(r, &scene::grid(n));
        let view = View {
            density: true,
            ..grid_view(Wire::Off)
        };
        let image = snap(r, &mesh, &view, SIZE);
        // How many pixels each channel is the largest in.
        let mut most = [0usize; 3];
        for p in image.rgba.chunks(4) {
            let top = (0..3).max_by_key(|&c| p[c]).unwrap_or(0);
            most[top] += 1;
        }
        most
    };
    let pixels = (SIZE * SIZE) as usize;
    let large = channels(4);
    assert!(
        large[0] > pixels * 9 / 10,
        "64-pixel quads are red: {large:?}"
    );
    let small = channels(128);
    assert!(
        small[2] > pixels * 9 / 10,
        "2-pixel quads are blue: {small:?}"
    );
}

/// The wire the window draws (`Auto`) on every adapter, lavapipe included: every quad edge
/// and nothing else. `Auto` is the barycentric wireframe where the device has shader
/// barycentrics and vertex pulling where it has not (lavapipe), and the two agree: the spike
/// measured them byte-identical on the RTX 5090 and 5 pixels off by one level on the Radeon.
/// A machine with no adapter fails instead of passing on an empty loop (research 27 R14).
#[test]
fn every_adapter_draws_quad_edges_and_nothing_else() {
    for (kind, r) in adapters() {
        let barycentric = r.gpu().capabilities().barycentric_wireframe;
        let (edges, _) = probes();
        let far = far_from_edges();
        let (wired, plain) = wired_grid(r, scene::grid(N).faces, Wire::Auto);
        assert_eq!(
            (
                lines_at(&wired, &plain, &edges),
                lines_at(&wired, &plain, &far)
            ),
            (edges.len(), 0),
            "{kind:?}: every quad edge and nothing else"
        );
        let mut sphere = scene::quad_sphere(10);
        scene::dent(&mut sphere, 0.12);
        // Many small chunks, so the barycentric path's chunk offset matters.
        let mesh = r
            .upload(
                &sphere,
                UploadOptions {
                    chunk_triangles: 97,
                },
            )
            .expect("uploads");
        // Without barycentrics `Auto` is vertex pulling itself, byte for byte.
        let (levels, pixels) = if barycentric { (2, 50) } else { (0, 0) };
        for preset in [Preset::Front, Preset::Iso] {
            let view = |wire| fitted(&mesh, preset, Shading::Clay, wire);
            let (max, count) = diff(
                &snap(r, &mesh, &view(Wire::Auto), SIZE),
                &snap(r, &mesh, &view(Wire::VertexPulling), SIZE),
            );
            assert!(
                max <= levels && count <= pixels,
                "{kind:?} {preset:?}: the wireframes agree (max {max} levels, {count} pixels differ)"
            );
        }
    }
}
