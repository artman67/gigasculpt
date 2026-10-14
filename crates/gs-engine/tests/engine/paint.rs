//! Surface paint's color column through the engine (decision 0039): a point nobody painted
//! reads white after every operation that fills a column from nothing, and Symmetrize blends
//! colors where it cuts a face and keeps each face's UVs on its own corners.

use std::collections::BTreeMap;

use gs_io::mesh::{Uvs, column};
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};
use serde_json::json;

use gs_testkit::Session;
use gs_testkit::fixtures::cylinder;

const WHITE: u32 = u32::MAX;
const RED: u32 = 0xff00_00ff;

fn colors(mesh: &PlainMesh) -> &[u32] {
    column(mesh, "color", AttrDomain::Vertex).unwrap_or_else(|| panic!("no color column"))
}

fn byte(c: u32, channel: u32) -> u32 {
    c >> (8 * channel) & 255
}

/// `mesh` with a `color` column read from each point's position.
fn painted(mut mesh: PlainMesh, color: impl Fn([f32; 3]) -> u32) -> PlainMesh {
    let values = mesh.positions.iter().map(|&p| color(p)).collect();
    mesh.attributes.insert(
        "color".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(values),
        },
    );
    mesh
}

/// A sheet of `n` by `m` points, point `(i, j)` at `at(i, j)`, of quads with every third split
/// into two triangles.
fn sheet(n: u32, m: u32, at: impl Fn(u32, u32) -> [f32; 3]) -> PlainMesh {
    let positions = (0..n * m).map(|v| at(v % n, v / n)).collect();
    let mut faces = Vec::new();
    for j in 0..m - 1 {
        for i in 0..n - 1 {
            let v = i + j * n;
            let q = [v, v + 1, v + n + 1, v + n];
            if (i + j) % 3 == 0 {
                faces.push([q[0], q[1], q[2], NO_VERTEX]);
                faces.push([q[0], q[2], q[3], NO_VERTEX]);
            } else {
                faces.push(q);
            }
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::default(),
    }
}

/// An open tube along y of radius 1 from y = -1 to 1, `m` rings of the test cylinder's twelve
/// points, with no caps.
fn tube(m: u32) -> PlainMesh {
    let ring = &cylinder().positions[..12];
    let n = ring.len() as u32;
    let at = |i: u32, j: u32| {
        let [x, _, z] = ring[i as usize];
        [x, 2.0 * j as f32 / (m - 1) as f32 - 1.0, z]
    };
    let positions = (0..n * m).map(|v| at(v % n, v / n)).collect();
    let faces = (0..m - 1)
        .flat_map(|j| (0..n).map(move |i| (i, j)))
        .map(|(i, j)| {
            let (a, b) = (i + j * n, (i + 1) % n + j * n);
            [a, b, b + n, a + n]
        })
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::default(),
    }
}

/// The UV corner columns of `mesh`, read from each corner's point by `uv`.
fn mapped(mut mesh: PlainMesh, uv: impl Fn([f32; 3]) -> [f32; 2]) -> PlainMesh {
    for (k, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate() {
        let (corner, channel) = (k / 2, k % 2);
        let values = (mesh.faces.iter())
            .map(|f| match f[corner] {
                NO_VERTEX => 0.0,
                v => uv(mesh.positions[v as usize])[channel],
            })
            .collect();
        mesh.attributes.insert(
            (*name).to_string(),
            Attribute {
                domain: AttrDomain::Face,
                values: AttrValues::F32(values),
            },
        );
    }
    mesh
}

/// The worst difference between each face corner's UV and `uv` of its point.
fn worst_uv(mesh: &PlainMesh, uv: impl Fn([f32; 3]) -> [f32; 2]) -> f32 {
    let Some(uvs) = Uvs::of(mesh) else {
        panic!("no UV columns")
    };
    let mut worst = 0.0_f32;
    for (f, face) in mesh.faces.iter().enumerate() {
        let got = uvs.face(f);
        for (k, &v) in face.iter().enumerate() {
            if v != NO_VERTEX {
                let want = uv(mesh.positions[v as usize]);
                worst = (worst.max((got[k][0] - want[0]).abs())).max((got[k][1] - want[1]).abs());
            }
        }
    }
    worst
}

#[test]
fn voxel_remesh_caps_an_open_part_in_white() {
    let mut s = Session::new();
    s.add_part("tube", &painted(tube(8), |_| RED));
    s.ok("mesh.voxel_remesh", json!({ "resolution": 48 }));
    let mesh = s.mesh("tube");
    let colors = colors(&mesh);
    // Red on the tube, white on the caps, a blend of the two between: never transparent.
    for (i, &c) in colors.iter().enumerate() {
        assert!(
            byte(c, 0) == 255 && byte(c, 3) == 255,
            "point {i}: {c:08x} is not between red and white"
        );
    }
    let white = colors.iter().filter(|&&c| c == WHITE).count();
    assert!(white > 10, "the caps are white ({white} points)");
}

#[test]
fn a_part_without_color_merges_in_white() {
    let setup = || {
        let mut s = Session::new();
        s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
        s.add_part("painted", &painted(tube(4), |_| RED));
        s
    };
    let sphere = setup().mesh("sphere").positions.len();
    // Each operation that went wrong, so one run shows all of them.
    let mut wrong = Vec::new();
    let mut s = setup();
    s.ok("part.merge_down", json!({ "part": "sphere" }));
    let mesh = s.mesh("sphere");
    if !colors(&mesh)[..sphere].iter().all(|&c| c == WHITE) {
        wrong.push("part.merge_down");
    }
    assert!(colors(&mesh)[sphere..].iter().all(|&c| c == RED));
    let mut s = setup();
    s.ok("part.merge_visible", json!({ "name": "both" }));
    let mesh = s.mesh("both");
    if !colors(&mesh)[..sphere].iter().all(|&c| c == WHITE) {
        wrong.push("part.merge_visible");
    }
    assert!(colors(&mesh)[sphere..].iter().all(|&c| c == RED));
    // Remeshed together, the sphere's surface comes out white, the tube's red, and the points
    // between a blend: none transparent.
    let mut s = setup();
    s.ok(
        "part.remesh",
        json!({ "parts": ["sphere", "painted"], "resolution": 48, "name": "one" }),
    );
    let mesh = s.mesh("one");
    let colors = colors(&mesh);
    let blend = |c: u32| byte(c, 0) == 255 && byte(c, 3) == 255;
    if !colors.iter().all(|&c| blend(c)) || !colors.contains(&WHITE) {
        wrong.push("part.remesh");
    }
    assert!(
        wrong.is_empty(),
        "the part without color is not white: {wrong:?}"
    );
}

#[test]
fn symmetrize_blends_a_color_ramp_where_it_cuts() {
    // Points at x = -0.93, -0.73, ... 0.07, ...: none on the plane, so every face across it is
    // cut, 0.35 of the way from the kept point at 0.07 to the dropped one at -0.13.
    let grid = sheet(10, 6, |i, j| [-0.93 + 0.2 * i as f32, 0.2 * j as f32, 0.0]);
    let ramp = |p: [f32; 3]| {
        let v = (255.0 * (p[0] + 1.0) / 2.0).round() as u32;
        v | v << 8 | v << 16 | 255 << 24
    };
    let mut s = Session::new();
    s.add_part("ramp", &painted(grid, ramp));
    s.ok("mesh.symmetrize", json!({ "axis": "x" }));
    let mesh = s.mesh("ramp");
    let colors = colors(&mesh);
    let mut cuts = 0;
    for (p, &c) in mesh.positions.iter().zip(colors) {
        if p[0] == 0.0 {
            cuts += 1;
            for channel in 0..3 {
                let got = f64::from(byte(c, channel));
                assert!(
                    (got - 127.5).abs() <= 1.0,
                    "the cut at y = {} reads {c:08x}, not the ramp's 127.5",
                    p[1]
                );
            }
            assert_eq!(byte(c, 3), 255, "opaque");
        }
    }
    assert_eq!(
        cuts, 7,
        "a cut point on each of the 6 rows, and one on the diagonal of the split cell at i = 4, j = 2"
    );
}

#[test]
fn symmetrize_keeps_each_face_corner_on_its_uv() {
    // A sheared sheet, so the plane cuts quads into triangles, quads and fanned pentagons; the
    // UV of every corner is its point's x + 1 and y, so the mirrored side shows the kept side's.
    let grid = sheet(10, 6, |i, j| {
        let y = 0.2 * j as f32;
        [-0.93 + 0.2 * i as f32 + 0.15 * y, y, 0.0]
    });
    let before = |p: [f32; 3]| [p[0] + 1.0, p[1]];
    let after = |p: [f32; 3]| [p[0].abs() + 1.0, p[1]];
    let mut s = Session::new();
    s.add_part("sheet", &mapped(grid, before));
    assert!(worst_uv(&s.mesh("sheet"), before) < 1e-6, "as made");
    s.ok("mesh.symmetrize", json!({ "axis": "x" }));
    let mesh = s.mesh("sheet");
    let worst = worst_uv(&mesh, after);
    assert!(worst < 1e-5, "a corner is {worst} off its UV");
}

#[test]
fn reorder_keeps_each_face_corner_on_its_uv() {
    // Made in rows, which the store's spatial order does not keep, each face starting at
    // another corner, so the store turns faces to start at their lowest new point.
    let mut grid = sheet(10, 6, |i, j| [0.2 * i as f32, 0.2 * j as f32, 0.0]);
    for (k, f) in grid.faces.iter_mut().enumerate() {
        let n = if f[3] == NO_VERTEX { 3 } else { 4 };
        f[..n].rotate_left(k % n);
    }
    let uv = |p: [f32; 3]| [p[0] + 1.0, p[1]];
    let mut s = Session::new();
    s.add_part("sheet", &mapped(grid, uv));
    let given = s.mesh("sheet");
    s.ok("mesh.reorder", json!({}));
    let mesh = s.mesh("sheet");
    assert_ne!(mesh.positions, given.positions, "a new numbering");
    let worst = worst_uv(&mesh, uv);
    assert!(
        worst < 1e-6,
        "from the order given: a corner is {worst} off its UV"
    );
    // Points raised off the sheet sort differently: a part in the store's order reordered.
    let samples: Vec<_> = (0..4)
        .map(|i| json!({ "position": [0.6 + 0.1 * i as f32, 0.5, 0.0] }))
        .collect();
    let stroke = json!({ "brush": "Draw", "part": "sheet", "samples": samples, "size": 0.6 });
    s.ok("brush.stroke", stroke);
    let raised = s.mesh("sheet");
    // The stroke moved the points a little, not the UVs: each point keeps the UV it holds.
    let Some(uvs) = Uvs::of(&raised) else {
        panic!("no UV columns")
    };
    let mut held = BTreeMap::new();
    for (f, face) in raised.faces.iter().enumerate() {
        for (&v, uv) in face.iter().zip(uvs.face(f)) {
            if v != NO_VERTEX {
                held.insert(raised.positions[v as usize].map(f32::to_bits), uv);
            }
        }
    }
    s.ok("mesh.reorder", json!({}));
    let mesh = s.mesh("sheet");
    assert_ne!(mesh.positions, raised.positions, "a new numbering again");
    let worst = worst_uv(&mesh, |p| held[&p.map(f32::to_bits)]);
    assert!(
        worst < 1e-6,
        "from the store's order: a corner is {worst} off its UV"
    );
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    assert_eq!(json!(s.mesh("sheet")), json!(given), "undo is exact");
}

#[test]
fn a_live_detail_stroke_on_a_part_with_uvs_keeps_each_face_corner_on_its_uv() {
    // A part with UVs does not refine (decision 0021, item 11): the stroke moves points and
    // keeps the faces, their corners and the UV on each.
    let uv = |p: [f32; 3]| [p[0] + 2.0 * p[2], p[1]];
    let mut s = Session::new();
    s.add_part("tube", &mapped(tube(16), uv));
    let before = s.mesh("tube");
    let samples: Vec<_> = (0..4)
        .map(|i| json!({ "position": [0.0, -0.3 + 0.2 * i as f32, 1.0] }))
        .collect();
    let done = s.query(
        "brush.stroke",
        json!({
            "brush": "Clay",
            "part": "tube",
            "samples": samples,
            "size": "20%",
            "live_detail": { "mode": "constant", "size": 0.02 },
        }),
    );
    let after = s.mesh("tube");
    assert_ne!(after.positions, before.positions, "the stroke moved points");
    assert_eq!(after.faces, before.faces, "and kept the faces: {done}");
    for name in gs_io::mesh::UV_COLUMNS {
        assert_eq!(
            json!(after.attributes.get(name)),
            json!(before.attributes.get(name)),
            "{name}"
        );
    }
    assert!(
        gs_testkit::text(&done["live_detail"]["why_not"]).contains("UV"),
        "{done}"
    );
}

/// The color at `at` on a flat mesh in the plane z = 0, each channel interpolated across the
/// triangle it lies in (in bytes, as stored).
fn color_at(mesh: &PlainMesh, at: [f32; 2]) -> Option<[f32; 4]> {
    let colors = colors(mesh);
    for f in &mesh.faces {
        let p = |k: usize| mesh.positions[f[k] as usize];
        let (a, b, c) = (p(0), p(1), p(2));
        let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        let u = ((b[0] - at[0]) * (c[1] - at[1]) - (c[0] - at[0]) * (b[1] - at[1])) / det;
        let v = ((c[0] - at[0]) * (a[1] - at[1]) - (a[0] - at[0]) * (c[1] - at[1])) / det;
        let w = 1.0 - u - v;
        if [u, v, w].iter().all(|&t| t >= -1e-5) {
            let c = [f[0], f[1], f[2]].map(|i| colors[i as usize]);
            let channel =
                |k| u * byte(c[0], k) as f32 + v * byte(c[1], k) as f32 + w * byte(c[2], k) as f32;
            return Some([0, 1, 2, 3].map(channel));
        }
    }
    None
}

/// A red line one point wide drawn across a flat sheet, read along its length and one and a
/// half point spacings to each side: the samples on it that read red (green under half), and
/// the samples beside it that read white (green over half).
fn line_read(mesh: &PlainMesh, x: f32, spacing: f32) -> (usize, usize) {
    let samples = 36;
    let along = |k: usize| -0.9 + 1.8 * (k as f32 + 0.5) / samples as f32;
    let green = |at| color_at(mesh, at).map_or(f32::NAN, |c| c[1]);
    let on = (0..samples)
        .filter(|&k| green([x, along(k)]) < 127.5)
        .count();
    let beside = (0..samples)
        .flat_map(|k| [-1.5, 1.5].map(|d| [x + d * spacing, along(k)]))
        .filter(|&at| green(at) > 127.5)
        .count();
    (on, beside)
}

#[test]
fn decimate_keeps_a_painted_line_on_a_flat_sheet() {
    // A flat sheet 41 points square, 0.05 apart, with red lines on the points at x = 0.15 and
    // x = -0.15: flat, so only the paint holds any face in place. A line survives when all 36
    // samples along it read red and all 72 beside it white, so its borders stay within one and
    // a half spacings of where they were. Off, the paint holds nothing and the lines go.
    let grid = sheet(41, 41, |i, j| {
        [-1.0 + 0.05 * i as f32, -1.0 + 0.05 * j as f32, 0.0]
    });
    let line = |p: [f32; 3]| {
        if (p[0].abs() - 0.15).abs() < 0.01 {
            RED
        } else {
            WHITE
        }
    };
    let reads = |args: serde_json::Value| {
        let mut s = Session::new();
        s.add_part("sheet", &painted(grid.clone(), line));
        let reply = s.query("mesh.decimate", args);
        assert_eq!(reply["reached"], json!(true), "{reply}");
        let mesh = s.mesh("sheet");
        [0.15, -0.15].map(|x| line_read(&mesh, x, 0.05))
    };
    let drawn = painted(grid.clone(), line);
    let as_drawn = [0.15, -0.15].map(|x| line_read(&drawn, x, 0.05));
    assert_eq!(as_drawn, [(36, 72); 2], "as drawn");
    assert_eq!(reads(json!({ "percent": 10 })), as_drawn, "decimated");
    let mirrored = reads(json!({ "percent": 10, "symmetry": "x" }));
    assert_eq!(mirrored, as_drawn, "decimated with symmetry");
    let off = reads(json!({ "percent": 10, "keep_paint": false }));
    assert!(
        off.iter().all(|&read| read != (36, 72)),
        "kept paint off: {off:?}"
    );
}

#[test]
fn decimate_keeps_uvs_across_the_plane_with_no_edge_loop_on_it() {
    // Symmetric across x = 0 with no point on the plane; the UVs are not symmetric (u is x + 1),
    // so the mirrored half shows the kept half's.
    let grid = sheet(10, 8, |i, j| {
        let (x, y) = (-0.9 + 0.2 * i as f32, j as f32 / 7.0);
        [x, y, 0.3 * (1.0 - x * x) * y * (1.0 - y)]
    });
    let after = |p: [f32; 3]| [p[0].abs() + 1.0, p[1]];
    let mut s = Session::new();
    s.add_part("sheet", &mapped(grid, |p| [p[0] + 1.0, p[1]]));
    let reply = s.query("mesh.decimate", json!({ "percent": 60, "symmetry": "x" }));
    assert_eq!(reply["reached"], json!(true), "{reply}");
    let mesh = s.mesh("sheet");
    let worst = worst_uv(&mesh, after);
    assert!(worst < 1e-5, "a corner is {worst} off its UV");
}
