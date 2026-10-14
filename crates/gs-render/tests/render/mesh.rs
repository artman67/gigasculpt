//! Upload: quads, chunks, normals, validation.

use crate::common::{BACKGROUND, covered, diff, fitted, lavapipe, snap, upload};
use crate::scene;
use gs_protocol::{NO_VERTEX as TRI, PlainMesh, quad_diagonal, quad_triangles};
use gs_render::{
    Bounds, Camera, Fit, Lens, Occlusion, Preset, Projection, RenderError, RenderMesh, Shading,
    UploadOptions, View, Wire, encode_normal, vertex_normals,
};
use proptest::prelude::*;

/// A quad sphere with every vertex pushed in or out a little, so no quad is planar.
fn bumpy_quads() -> PlainMesh {
    let mut mesh = scene::quad_sphere(6);
    for (i, p) in mesh.positions.iter_mut().enumerate() {
        let s = 1.0 + 0.08 * ((i * 7919 % 13) as f32 / 12.0 - 0.5);
        *p = p.map(|c| c * s);
    }
    mesh
}

fn split(mesh: &PlainMesh, halves: impl Fn([u32; 4]) -> [[u32; 4]; 2]) -> PlainMesh {
    scene::plain(
        mesh.positions.clone(),
        mesh.faces.iter().flat_map(|&f| halves(f)).collect(),
    )
}

#[test]
fn quad_renders_like_its_two_triangles() {
    let r = lavapipe();
    let quads = bumpy_quads();
    // Each quad as two triangles along its shorter diagonal (decision 0040, section 2), or
    // along the other one.
    let along = |other: usize| {
        split(&quads, |f| {
            let diagonal = quad_diagonal(f.map(|v| quads.positions[v as usize])) ^ other;
            quad_triangles(diagonal).map(|t| [f[t[0]], f[t[1]], f[t[2]], TRI])
        })
    };
    let (as_drawn, other_diagonal) = (along(0), along(1));
    let meshes = [&quads, &as_drawn, &other_diagonal].map(|m| upload(r, m));
    for shading in [Shading::Clay, Shading::Zebra, Shading::Normals] {
        let [q, t, o] = meshes.each_ref().map(|m| {
            snap(
                r,
                m,
                &fitted(&meshes[0], Preset::Iso, shading, Wire::Off),
                128,
            )
        });
        assert!(
            q == t,
            "a quad renders exactly like its two triangles along its shorter diagonal"
        );
        assert!(
            q != o,
            "the test can tell the diagonals apart (the quads are not planar)"
        );
    }
}

/// A camera-facing grid filling the whole frame: any triangle not drawn leaves a hole.
fn filled_frame_view<'a>() -> View<'a> {
    let fov_y = std::f32::consts::FRAC_PI_2; // tan(fov / 2) = 1: half-height 0.9 at distance 0.9
    View::new(
        Camera {
            distance: 0.9,
            lens: Lens::from_fov(fov_y, Fit::Vertical),
            projection: Projection::Orthographic,
            ..Camera::preset(Preset::Front)
        },
        Shading::Clay,
    )
}

#[test]
fn every_triangle_is_drawn_whatever_the_chunk_size() {
    let r = lavapipe();
    let grid = scene::grid(24);
    let reference = r
        .upload(
            &grid,
            UploadOptions {
                chunk_triangles: 1 << 20,
            },
        )
        .expect("uploads");
    let base = snap(r, &reference, &filled_frame_view(), 96);
    assert_eq!(covered(&base), 96 * 96, "the grid covers the frame");
    for chunk_triangles in [1, 7, 64, 1151] {
        let mesh = r
            .upload(&grid, UploadOptions { chunk_triangles })
            .expect("uploads");
        assert_eq!(
            snap(r, &mesh, &filled_frame_view(), 96),
            base,
            "chunks of {chunk_triangles} draw the same pixels"
        );
        let wire = View {
            wire: Wire::VertexPulling,
            ..filled_frame_view()
        };
        assert_eq!(
            covered(&snap(r, &mesh, &wire, 96)),
            96 * 96,
            "vertex pulling with chunks of {chunk_triangles} draws every triangle"
        );
    }
    assert!(
        matches!(
            r.upload(&grid, UploadOptions { chunk_triangles: 0 }),
            Err(RenderError::Mesh(_))
        ),
        "empty chunks are refused"
    );
}

#[test]
fn chunk_order_does_not_change_a_closed_mesh() {
    let r = lavapipe();
    let mut sphere = scene::icosphere(4);
    scene::dent(&mut sphere, 0.12);
    let one = r
        .upload(
            &sphere,
            UploadOptions {
                chunk_triangles: 1 << 20,
            },
        )
        .expect("uploads");
    let many = r
        .upload(
            &sphere,
            UploadOptions {
                chunk_triangles: 97,
            },
        )
        .expect("uploads");
    assert_eq!(
        many.chunks().len(),
        5120usize.div_ceil(97),
        "97-triangle chunks"
    );
    for wire in [Wire::Off, Wire::VertexPulling] {
        // Depth's precision follows the near and far planes, which the chunks' bounds set, so
        // occlusion (which reads depth) is off, and the view direction Lit mode's highlights
        // read (reconstructed from depth) can move a pixel across a rounding step: at most one
        // level on a few pixels. A gap or a draw-order fight would differ far more.
        let view = View {
            occlusion: Occlusion::OFF,
            ..fitted(&one, Preset::Iso, Shading::Zebra, wire)
        };
        let (max, count) = diff(&snap(r, &one, &view, 128), &snap(r, &many, &view, 128));
        assert!(
            max <= 1 && count <= 8,
            "{wire:?}: cutting into chunks and reordering changes the picture ({count} pixels, \
             up to {max} levels)"
        );
    }
}

fn random_mesh() -> impl Strategy<Value = PlainMesh> {
    prop::collection::vec(prop::array::uniform3(-10.0f32..10.0), 3..60).prop_flat_map(|positions| {
        let n = positions.len() as u32;
        let face =
            (0..n, 0..n, 0..n, prop_oneof![Just(TRI), 0..n]).prop_map(|(a, b, c, d)| [a, b, c, d]);
        prop::collection::vec(face, 0..80)
            .prop_map(move |faces| scene::plain(positions.clone(), faces))
    })
}

/// Chunks cover the triangles in order, all but the last full, each with exact bounds.
#[test]
fn chunk_table_is_complete_and_tight() {
    // Opened before the runner, so a device that fails to open fails once, not per case.
    let r = lavapipe();
    proptest!(|(plain in random_mesh(), size in 1u32..40)| {
        let mesh = r.upload(&plain, UploadOptions { chunk_triangles: size }).expect("valid mesh");
        let (positions, tris) = (&plain.positions, plain.triangles().collect::<Vec<_>>());
        prop_assert_eq!(mesh.triangle_count() as usize, tris.len());
        prop_assert_eq!(mesh.chunks().len(), tris.len().div_ceil(size as usize));
        let mut all = Bounds::EMPTY;
        for (k, (chunk, expected)) in mesh.chunks().iter().zip(tris.chunks(size as usize)).enumerate() {
            prop_assert_eq!(chunk.first_triangle, k as u32 * size);
            prop_assert_eq!(chunk.triangle_count as usize, expected.len());
            let mut b = Bounds::EMPTY;
            expected.iter().flatten().for_each(|&v| b.include(positions[v as usize]));
            prop_assert_eq!(chunk.bounds, b);
            expected.iter().flatten().for_each(|&v| all.include(positions[v as usize]));
        }
        prop_assert_eq!(mesh.bounds(), all);
        prop_assert_eq!(mesh.fit_points().len(), 8 * mesh.chunks().len());
    });
}

/// Area-weighted vertex normals against an f64 reference with the same near-zero rule: a sum
/// no longer than `FOLD_NOISE` times the sum of the longer of `|b - a|` and `|c - a|` squared
/// means +Z. Clearly under the
/// limit (half of it, beyond any f32 rounding) the normal must be exactly +Z; clearly over it
/// (twice) and away from heavy cancellation the directions must agree.
fn normals_match_reference(plain: &PlainMesh) -> Result<(), TestCaseError> {
    let got = vertex_normals(plain).expect("valid mesh");
    let positions = &plain.positions;
    let mut sum = vec![[0.0f64; 3]; positions.len()];
    let (mut total, mut size) = (vec![0.0f64; positions.len()], vec![0.0f64; positions.len()]);
    for [a, b, c] in plain.triangles() {
        let p = |v: u32| positions[v as usize].map(f64::from);
        let (pa, pb, pc) = (p(a), p(b), p(c));
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        for v in [a, b, c].map(|v| v as usize) {
            (0..3).for_each(|k| sum[v][k] += n[k]);
            total[v] += len(n);
            size[v] += len(e1).max(len(e2)) * len(e1).max(len(e2));
        }
    }
    for (v, (s, g)) in sum.iter().zip(&got).enumerate() {
        let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
        let noise = f64::from(gs_render::FOLD_NOISE) * size[v];
        if len <= noise / 2.0 {
            prop_assert!(
                g.map(f32::to_bits) == [0.0f32, 0.0, 1.0].map(f32::to_bits),
                "vertex {} whose faces cancel (or with none) gets +Z, not {:?}",
                v,
                g
            );
        } else if len > 2.0 * noise && len > 1e-2 * total[v] {
            // Away from near-total cancellation, where f32 sums are only roughly right.
            let cos = (0..3).map(|k| s[k] / len * f64::from(g[k])).sum::<f64>();
            prop_assert!(
                cos > 1.0 - 1e-6,
                "vertex {} normal {:?} matches the reference {:?}",
                v,
                g,
                s
            );
        }
    }
    Ok(())
}

/// The case a random seed found (saved by the review-0001 follow-up): a quad `17 0 48 14`,
/// drawn as `0 48 14` and `0 14 17` (its shorter diagonal is 0 to 14), and the triangle
/// `17 14 0` fold back to back, so vertex 17's two face normals cancel. f32 left noise that
/// normalized to a random direction; f64 cancelled exactly. (The seed's triangle was `14 48 17`,
/// folded onto the quad's other split.)
#[test]
fn folded_faces_give_plus_z() {
    #[rustfmt::skip]
    let positions = vec![
        [-2.554031, -2.6426623, -7.7701364], [9.875168, -0.40956262, -9.901319],
        [5.548287, 8.550785, 2.1027052], [2.457682, 5.073803, -1.3322276],
        [-9.87302, -6.016365, 5.379061], [3.390169, -8.715139, -1.3871],
        [8.328247, 2.5215795, -5.1844482], [2.0290103, -1.7123669, -4.0288844],
        [-8.231405, 4.653665, 3.0133312], [-8.138342, 5.042206, -5.0373874],
        [2.913892, 4.1168175, -2.079837], [-7.139388, 5.3726287, 8.58781],
        [7.766192, -7.852255, -7.1668205], [7.5462275, -9.072623, 7.9281073],
        [-2.0438843, -3.130471, -3.1354978], [-2.205552, 3.0062475, -0.59211034],
        [-0.5256007, -5.9568076, -8.285684], [-9.353623, 1.0357882, 2.3265736],
        [6.3569393, 3.7160065, -7.2436585], [-3.7824025, -5.4127045, -2.07325],
        [5.534469, -7.617221, -6.8958545], [-6.828777, 1.715076, -0.51915663],
        [-3.193459, 5.645696, 6.5849557], [2.2936323, 1.2485301, 0.33333305],
        [2.0820513, -9.164909, -9.839365], [-7.60034, -4.8985505, 9.015172],
        [5.3453565, -0.40959772, -2.5799], [4.817519, 0.33329698, 1.6718642],
        [9.822591, 4.4001274, -4.444522], [-0.32668316, -2.9712894, 7.5688896],
        [0.56016153, -3.8633974, -3.0031412], [6.466614, 7.2715087, 6.4884615],
        [-6.937746, -1.1211563, 1.7124493], [-9.749412, -9.135091, 4.0909424],
        [8.381159, 5.133763, 2.101019], [-5.4898276, 6.7205515, -4.078746],
        [-5.7909374, 6.01789, -7.008098], [1.8232831, 2.2280874, 7.6561728],
        [4.0532136, 2.2019548, 1.1137271], [-2.4886851, 2.8255284, 6.037076],
        [3.1814451, 3.8100822, 6.373751], [3.3397217, -3.227535, 2.7236817],
        [-6.3197446, -6.194154, -3.3814104], [-2.8052554, -4.17628, -3.0862594],
        [6.268218, -7.518472, 8.951088], [-1.9085455, -6.134361, 4.896911],
        [-7.868149, 0.20111735, -7.430768], [-2.7366633, -4.881168, 4.158031],
        [6.1706815, -9.108707, -5.7614627], [-1.8217869, -1.263689, -0.6218948],
        [-9.481196, -7.0713477, 7.922256], [9.509279, -4.324302, 0.23479792],
        [-2.3979383, 7.23492, -4.8171797], [-2.3530347, -3.683111, 9.425758],
        [-5.7177696, 2.165294, 9.578728], [8.53785, -0.6893316, 8.896491],
        [-8.804971, 5.957548, 8.05116],
    ];
    let plain = scene::plain(positions, vec![[17, 0, 48, 14], [17, 14, 0, TRI]]);
    normals_match_reference(&plain).expect("the saved case matches the reference");
    let normals = vertex_normals(&plain).expect("valid mesh");
    assert_eq!(normals[17], [0.0, 0.0, 1.0], "the folded vertex gets +Z");
    assert_ne!(
        normals[0],
        [0.0, 0.0, 1.0],
        "a vertex with one real face keeps its own normal"
    );
}

proptest! {
    #[test]
    fn vertex_normals_match_reference(plain in random_mesh()) {
        normals_match_reference(&plain)?;
    }

    /// 16-bit octahedral normals decode (by an independent f64 decoder) to within 0.0002 rad.
    #[test]
    fn octahedral_normals_round_trip(v in prop::array::uniform3(-1.0f32..1.0)) {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        prop_assume!(len > 1e-3);
        let n = v.map(|c| c / len);
        let [x, y] = encode_normal(n).map(|e| (f64::from(e) / 32767.0).max(-1.0));
        // Compare against the f64 direction of v: n itself is only unit length to f32 precision.
        let truth = v.map(f64::from);
        let tl = (truth[0] * truth[0] + truth[1] * truth[1] + truth[2] * truth[2]).sqrt();
        let z = 1.0 - x.abs() - y.abs();
        let t = (-z).max(0.0);
        let d = [x - t * x.signum(), y - t * y.signum(), z];
        let dl = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let cos = (0..3).map(|k| d[k] / dl * truth[k] / tl).sum::<f64>();
        prop_assert!(cos > libm::cos(2e-4), "{:?} decodes to {:?}", n, d);
    }
}

#[test]
fn bounds_have_eight_corners_unless_empty() {
    assert!(
        Bounds::EMPTY.corners().is_empty(),
        "an empty box has no corners"
    );
    let flat = Bounds {
        min: [0.0, 1.0, 2.0],
        max: [0.0, 3.0, 4.0],
    };
    let mut corners = flat.corners();
    corners.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let want = [
        [0.0, 1.0, 2.0],
        [0.0, 1.0, 4.0],
        [0.0, 3.0, 2.0],
        [0.0, 3.0, 4.0],
    ];
    assert_eq!(
        corners,
        want.iter().flat_map(|c| [*c, *c]).collect::<Vec<_>>(),
        "a box flat in x has its 4 corners twice"
    );
    let mut grown = Bounds::EMPTY;
    grown.union(&flat);
    grown.union(&Bounds {
        min: [-1.0, 2.0, 2.0],
        max: [0.5, 2.5, 9.0],
    });
    grown.union(&Bounds::EMPTY);
    assert_eq!(
        grown,
        Bounds {
            min: [-1.0, 1.0, 2.0],
            max: [0.5, 3.0, 9.0]
        },
        "union covers both boxes, and an empty box adds nothing"
    );
}

#[test]
fn zero_normal_encodes_as_plus_z() {
    assert_eq!(
        encode_normal([0.0; 3]),
        [0, 0],
        "no direction encodes as the octahedron's +Z"
    );
    assert_eq!(
        encode_normal([0.0, 0.0, -1.0]),
        [32767, 32767],
        "-Z sits in the folded corner"
    );
}

#[test]
fn malformed_meshes_are_rejected() {
    let bad = |faces: &[[u32; 4]], normals: Option<&[[f32; 3]]>| {
        let plain = scene::plain(vec![[0.0f32; 3]; 3], faces.to_vec());
        let mesh = normals.map_or(RenderMesh::from(&plain), |n| {
            RenderMesh::from(&plain).with_normals(n)
        });
        let refused = matches!(
            lavapipe().upload(mesh, UploadOptions::default()),
            Err(RenderError::Mesh(_))
        );
        assert_eq!(
            refused,
            vertex_normals(&plain).is_err() || normals.is_some_and(|n| n.len() != 3),
            "vertex_normals checks faces too"
        );
        refused
    };
    assert!(
        bad(&[[0, 1, 3, TRI]], None),
        "a triangle corner past the last vertex"
    );
    assert!(
        bad(&[[0, 1, 2, 3]], None),
        "a quad's 4th corner past the last vertex"
    );
    let plain = scene::plain(vec![[0.0; 3]; 3], vec![[0, 1, 3, TRI]]);
    let error = lavapipe()
        .upload(&plain, UploadOptions::default())
        .err()
        .map(|e| e.to_string());
    assert_eq!(
        error.as_deref(),
        Some("bad mesh: face 0 uses vertex 3, but there are 3"),
        "the error names the face"
    );
    assert!(
        bad(&[[0, 1, 2, TRI]], Some(&[[0.0, 0.0, 1.0]; 2])),
        "fewer normals than positions"
    );
    assert!(
        !bad(&[[0, 1, 2, TRI], [2, 1, 0, 0]], Some(&[[0.0, 0.0, 1.0]; 3])),
        "the last vertex and a 4th slot of u32::MAX are fine"
    );
}

#[test]
fn empty_mesh_renders_only_background() {
    let r = lavapipe();
    let mesh = upload(r, &PlainMesh::default());
    assert_eq!(mesh.chunks().len(), 0, "no faces, no chunks");
    let image = snap(r, &mesh, &filled_frame_view(), 32);
    assert!(
        image.rgba.chunks(4).all(|p| p == BACKGROUND),
        "the clear color is exactly BACKGROUND"
    );
}

/// Normals given with the mesh are the ones drawn: all +Z shows one flat color.
#[test]
fn given_normals_are_drawn() {
    let r = lavapipe();
    let sphere = scene::icosphere(3);
    let up = vec![[0.0f32, 0.0, 1.0]; sphere.positions.len()];
    let mesh = r
        .upload(
            RenderMesh::from(&sphere).with_normals(&up),
            UploadOptions::default(),
        )
        .expect("uploads");
    let image = snap(
        r,
        &mesh,
        &fitted(&mesh, Preset::Front, Shading::Normals, Wire::Off),
        64,
    );
    let colors: Vec<&[u8]> = image.rgba.chunks(4).filter(|p| *p != BACKGROUND).collect();
    assert!(
        colors.len() > 1000 && colors.iter().all(|p| *p == colors[0]),
        "one flat color: every pixel has the same normal"
    );
    // +Z shows as (127.5, 127.5, 255), which rounds either way.
    assert!(
        colors[0][0].abs_diff(128) <= 1 && colors[0][1].abs_diff(128) <= 1 && colors[0][2] == 255,
        "the color is +Z: {:?}",
        colors[0]
    );
}
