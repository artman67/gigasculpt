//! Where each new vertex and face came from (the sources the engine carries columns by,
//! decision 0037 section 3), and projection with blur against a slow reference built from the
//! unprojected and projected results.

use gs_geo::voxel::{CellSize, Remeshed, Settings, SignMethod, remesh};
use gs_protocol::{AttrDomain, AttrValues, PlainMesh, face_triangles};

use crate::common::*;

fn with(mesh: &PlainMesh, resolution: u32, project: bool, blur: f32) -> Remeshed {
    let settings = Settings {
        cell: CellSize::Resolution(resolution),
        project,
        blur,
    };
    remesh(mesh, &settings).unwrap_or_else(|e| panic!("remesh failed: {e}"))
}

/// A sphere with one vertex column and one face column, so the remesh finds sources.
fn painted(n: u32) -> PlainMesh {
    let mut mesh = sphere([0.1, -0.2, 0.05], 1.0, n);
    let (verts, faces) = (mesh.positions.len(), mesh.faces.len());
    let columns = [
        (
            "mask",
            column(AttrDomain::Vertex, AttrValues::F32(vec![0.5; verts])),
        ),
        (
            "group",
            column(AttrDomain::Face, AttrValues::U32(vec![1; faces])),
        ),
    ];
    for (name, c) in columns {
        mesh.attributes.insert(name.into(), c);
    }
    mesh
}

/// The point a source's weights make of its input vertices.
fn rebuilt(mesh: &PlainMesh, (corners, weights): ([u32; 3], [f32; 3])) -> P {
    let p = corners.map(|v| wide(mesh.positions[v as usize]));
    std::array::from_fn(|a| (0..3).map(|k| f64::from(weights[k]) * p[k][a]).sum())
}

#[test]
fn each_vertex_source_is_an_input_triangle_and_weights_that_rebuild_its_point() {
    let mesh = painted(10);
    let tris: Vec<_> = mesh.triangles().collect();
    // Projected, each vertex sits on its source point; unprojected, within about half a voxel.
    for (project, within) in [(true, 1e-3), (false, 0.6)] {
        let out = with(&mesh, 16, project, 0.0);
        let points = &out.sources.points;
        assert_eq!(points.len(), out.mesh.positions.len(), "one per vertex");
        for (p, source) in out.mesh.positions.iter().zip(points) {
            let Some((corners, weights)) = *source else {
                panic!("a closed sphere has no hole, so {p:?} has a source");
            };
            assert!(tris.contains(&corners), "{corners:?} is an input triangle");
            let sum: f32 = weights.iter().sum();
            assert!(
                weights.iter().all(|&w| w >= 0.0) && (sum - 1.0).abs() < 1e-5,
                "{weights:?}"
            );
            let off = norm(sub(rebuilt(&mesh, (corners, weights)), wide(*p)));
            assert!(
                off <= within * out.grid.voxel,
                "{project}: {p:?} is {off} from its source"
            );
        }
    }
}

#[test]
fn each_face_source_is_the_nearest_old_face() {
    let mut mesh = painted(10);
    // Remeshed from a mesh with only a face column, so face sources alone are asked for too.
    mesh.attributes.remove("mask");
    let out = with(&mesh, 16, false, 0.0);
    assert_eq!(
        out.sources.faces.len(),
        out.mesh.faces.len(),
        "one per face"
    );
    let all = Nearest::new(triangles(&mesh), out.grid.voxel);
    for (f, source) in out.mesh.faces.iter().zip(&out.sources.faces) {
        let Some(g) = *source else {
            panic!("face {f:?} has no source on a closed sphere");
        };
        let c = corners(&out.mesh, f);
        let centre: P = std::array::from_fn(|a| c.iter().map(|p| p[a]).sum::<f64>() / 4.0);
        let old = mesh.faces[g as usize];
        let at = |k: usize| mesh.positions[old[k] as usize];
        let near_old = (face_triangles(old, at).iter())
            .map(|t| triangle_distance(centre, &t.map(|k| wide(at(k)))))
            .fold(f64::INFINITY, f64::min);
        assert!(
            near_old <= all.distance(centre) + 0.3 * out.grid.voxel,
            "face {f:?} came from face {g}, which is not near it"
        );
    }
}

#[test]
fn surface_made_to_close_a_hole_has_no_source() {
    let mut open = without(&sphere([0.0; 3], 1.0, 12), |c| {
        c.iter().all(|p| p[1] >= 0.55)
    });
    let n = open.positions.len();
    let column = column(AttrDomain::Vertex, AttrValues::F32(vec![1.0; n]));
    open.attributes = [("mask".to_string(), column)].into();
    let out = with(&open, 24, false, 0.0);
    assert_eq!(out.sign, SignMethod::CappedWinding, "the patch is a cap's");
    let mut film = 0;
    for (p, source) in out.mesh.positions.iter().zip(&out.sources.points) {
        let middle = f64::from(p[0] * p[0] + p[2] * p[2]) <= 0.25 && p[1] >= 0.5;
        if middle {
            film += 1;
            assert!(source.is_none(), "film vertex {p:?} has no source");
        } else if p[1] <= 0.3 {
            assert!(source.is_some(), "surface vertex {p:?} has one");
        }
    }
    assert!(film >= 10, "non-trivial: {film} film vertices");
    let faces = &out.sources.faces;
    assert!(
        faces.contains(&None) && faces.iter().any(Option::is_some),
        "film and surface faces"
    );
}

#[test]
fn a_mesh_with_no_columns_asks_for_no_sources() {
    let mut mesh = painted(8);
    mesh.attributes.clear();
    for project in [false, true] {
        let out = with(&mesh, 12, project, 0.0);
        assert!(
            out.sources.points.is_empty() && out.sources.faces.is_empty(),
            "{project}"
        );
        assert!(
            out.mesh.attributes.is_empty(),
            "a remesh carries no columns itself"
        );
    }
    let out = with(&painted(8), 12, false, 0.0);
    assert!(
        out.mesh.attributes.is_empty(),
        "a remesh carries no columns itself"
    );
}

#[test]
fn blur_smooths_the_projection_moves_exactly_as_documented() {
    let mesh = sphere([0.1, -0.2, 0.05], 1.0, 24);
    let base = with(&mesh, 14, false, 0.0).mesh;
    let target = with(&mesh, 14, true, 0.0).mesh;
    let moves: Vec<P> = base
        .positions
        .iter()
        .zip(&target.positions)
        .map(|(b, t)| sub(wide(*t), wide(*b)))
        .collect();
    // Each vertex's neighbours, once per face it shares an edge with them in.
    let mut around = vec![Vec::new(); base.positions.len()];
    for f in &base.faces {
        for k in 0..4 {
            around[f[k] as usize].push(f[(k + 1) % 4] as usize);
            around[f[k] as usize].push(f[(k + 3) % 4] as usize);
        }
    }
    for (blur, passes) in [(10.0, 3), (100.0, 25)] {
        let mut m = moves.clone();
        for _ in 0..passes {
            m = (0..m.len())
                .map(|v| {
                    let n = around[v].len() as f64;
                    std::array::from_fn(|a| {
                        0.5 * m[v][a] + 0.5 * around[v].iter().map(|&u| m[u][a]).sum::<f64>() / n
                    })
                })
                .collect();
        }
        let out = with(&mesh, 14, true, blur);
        assert!(out.mesh.faces == base.faces, "blur {blur}: same topology");
        let voxel = out.grid.voxel;
        for (v, p) in out.mesh.positions.iter().enumerate() {
            let want: P = std::array::from_fn(|a| f64::from(base.positions[v][a]) + m[v][a]);
            let off = norm(sub(wide(*p), want));
            assert!(
                off <= 1e-4 * voxel,
                "blur {blur}, vertex {v}: {off} from the reference"
            );
        }
    }
    let most = with(&mesh, 14, true, 100.0).mesh;
    let beyond = with(&mesh, 14, true, 1000.0).mesh;
    assert!(most.positions == beyond.positions, "blur is capped at 100");
}
