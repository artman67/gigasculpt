//! Shared pieces for the brush tests: the shipped presets, a symmetric test mesh, strokes along
//! arcs, and plain-data views of the result. Reads gs-core only through its public API.

use std::collections::BTreeMap;
use std::sync::Arc;

use gs_core::alpha::{Picture, Pyramid, Recipe};
use gs_core::symmetry::Symmetry;
use gs_core::{
    BrushPreset, MeshStore, NodeConfig, StrokeParams, StrokeReport, StrokeSample, Vec3, View,
    stroke,
};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::fixtures;

use crate::common::{mix, ok};

/// One shipped preset: file stem, text and parsed preset.
pub struct Shipped {
    pub stem: String,
    pub text: String,
    pub preset: BrushPreset,
}

/// Every preset in `assets/brushes`, by file name.
pub fn shipped() -> Vec<Shipped> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brushes");
    let mut files: Vec<_> = ok(std::fs::read_dir(&dir)).map(|e| ok(e).path()).collect();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let text = ok(std::fs::read_to_string(&f));
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned());
            let preset =
                BrushPreset::from_toml(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            Shipped {
                stem: stem.unwrap_or_default(),
                text,
                preset,
            }
        })
        .collect()
}

/// The shipped preset in `<stem>.toml`.
pub fn preset(stem: &str) -> Shipped {
    let found = shipped().into_iter().find(|s| s.stem == stem);
    found.unwrap_or_else(|| panic!("no preset {stem}"))
}

/// Select Rect drawing the closed outline through every point (the polygon shape), for the
/// outline rules on outlines that are not rectangles (decision 0051, section 4).
pub fn lasso() -> BrushPreset {
    let text = preset("select-rect").text;
    let rect = "shape = \"rect\"";
    assert!(text.contains(rect), "Select Rect draws a rectangle");
    ok(BrushPreset::from_toml(
        &text.replace(rect, "shape = \"polygon\""),
    ))
}

/// Small nodes, so a test mesh has a dozen or more nodes and seams.
pub fn cfg() -> NodeConfig {
    ok(NodeConfig::new(6, 40))
}

/// The unit icosphere: exactly symmetric across every coordinate plane, with vertices on
/// the planes.
pub fn ico(subdivisions: u32) -> PlainMesh {
    fixtures::icosphere(subdivisions)
}

/// A flat square of `n` by `n` quads on z = 0 from -1 to 1, facing +z, symmetric bit for
/// bit across x = 0 and y = 0.
pub fn grid(n: u32) -> PlainMesh {
    let c = |k: u32| ((f64::from(k) - f64::from(n) / 2.0) * 2.0 / f64::from(n)) as f32;
    let at = |i: u32, j: u32| i * (n + 1) + j;
    let mut m = PlainMesh::default();
    for i in 0..=n {
        for j in 0..=n {
            m.positions.push([c(i), c(j), 0.0]);
        }
    }
    for i in 0..n {
        for j in 0..n {
            m.faces
                .push([at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    m
}

/// `mesh` with a vertex mask: 1 on about a third of the vertices (by a hash), 0 elsewhere.
pub fn masked(mesh: &PlainMesh) -> PlainMesh {
    let mut m = mesh.clone();
    let mask = (0..m.positions.len() as u64)
        .map(|i| {
            if mix(i * 7 + 3).is_multiple_of(3) {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    m.attributes.insert(
        "mask".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(mask),
        },
    );
    m
}

/// `mesh` painted red, blue and white by a hash, so a brush that blends color has color to blend.
pub fn painted(mesh: &PlainMesh) -> PlainMesh {
    let mut m = mesh.clone();
    let color = (0..m.positions.len() as u64)
        .map(|i| [RED, BLUE, u32::MAX][(mix(i * 5 + 1) % 3) as usize])
        .collect();
    m.attributes.insert(
        "color".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(color),
        },
    );
    m
}

/// A mesh's colors: white where it has no color column (the column's empty value).
pub fn colors(m: &PlainMesh) -> Vec<u32> {
    match m.attributes.get("color").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.clone(),
        _ => vec![u32::MAX; m.positions.len()],
    }
}

pub fn store(mesh: &PlainMesh) -> MeshStore {
    ok(MeshStore::from_plain(mesh, cfg()))
}

/// A sample at `p` with full pressure.
pub fn at(p: Vec3, radius: f32) -> StrokeSample {
    StrokeSample {
        pos: p,
        pressure: 1.0,
        tilt: [0.0; 2],
        radius,
    }
}

/// `n + 1` samples along the great-circle arc from `a` to `b` on the unit sphere.
pub fn arc(a: Vec3, b: Vec3, n: usize, radius: f32) -> Vec<StrokeSample> {
    (0..=n)
        .map(|i| at((a + (b - a) * (i as f32 / n as f32)).normalized(), radius))
        .collect()
}

/// Samples along straight segments between `corners` (position, pressure, radius), each
/// segment cut into `per` pieces, with pressure and radius changing linearly along it.
pub fn polyline(corners: &[(Vec3, f32, f32)], per: usize) -> Vec<StrokeSample> {
    let sample = |a: (Vec3, f32, f32), b: (Vec3, f32, f32), t: f32| StrokeSample {
        pressure: a.1 + (b.1 - a.1) * t,
        ..at(a.0 + (b.0 - a.0) * t, a.2 + (b.2 - a.2) * t)
    };
    let mut out = vec![sample(corners[0], corners[0], 0.0)];
    for w in corners.windows(2) {
        out.extend((1..=per).map(|i| sample(w[0], w[1], i as f32 / per as f32)));
    }
    out
}

/// A camera looking at the origin from the +z side.
pub fn front() -> View {
    View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: None,
    }
}

/// Mirror across x = 0, no feather override.
pub fn mirror_x() -> Symmetry {
    Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    }
}

/// The samples a preset's test stroke uses: an arc over the front of the sphere crossing near
/// the x = 0 plane, a drag from a point on it, or for gestures a line or a rectangle's press and
/// release on the plane z = 2.
pub fn samples(p: &BrushPreset, radius: f32) -> Vec<StrokeSample> {
    let (a, b) = (Vec3::new(0.35, 0.2, 1.0), Vec3::new(-0.1, 0.35, 1.0));
    match p.name.as_str() {
        "Clip Curve" => vec![
            at(Vec3::new(-2.0, 0.3, 2.0), radius),
            at(Vec3::new(2.0, 0.3, 2.0), radius),
        ],
        "Select Rect" => [(-0.5, -0.4), (0.6, 0.5)]
            .map(|(x, y)| at(Vec3::new(x, y, 2.0), radius))
            .to_vec(),
        _ if matches!(p.stroke, gs_core::brush::StrokeSpec::Drag { .. }) => {
            let g = a.normalized();
            vec![
                at(g, radius),
                at(g + Vec3::new(-0.1, 0.15, 0.05), radius),
                at(g + Vec3::new(-0.2, 0.3, 0.1), radius),
            ]
        }
        _ => arc(a, b, 8, radius),
    }
}

/// The picture the shipped recipe `assets/alphas/<name>.toml` draws.
pub fn recipe_picture(name: &str) -> Picture {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/alphas");
    let text = ok(std::fs::read_to_string(dir.join(format!("{name}.toml"))));
    ok(ok(Recipe::from_toml(&text)).draw())
}

/// The pyramid a preset's alpha names, from the shipped recipe of that name, the way the
/// engine resolves it.
pub fn shipped_alpha(p: &BrushPreset) -> Option<Arc<Pyramid>> {
    let spec = p.tip.alpha.as_ref()?;
    Some(Arc::new(Pyramid::build(&recipe_picture(&spec.image), spec)))
}

/// A pyramid's levels as plain data, for the slow reference.
pub fn levels_of(p: &Pyramid) -> Vec<gs_testkit::reference::alpha::Level> {
    gs_testkit::reference::alpha::read(p.levels(), |k| p.size(k), |k, x, y| p.texel(k, x, y))
}

/// The mesh after one stroke of `p` along `s` on `mesh`, with `prm`.
pub fn after(
    mesh: &PlainMesh,
    p: &BrushPreset,
    prm: StrokeParams,
    s: &[StrokeSample],
) -> PlainMesh {
    let mut m = store(mesh);
    run(&mut m, p, prm, s);
    m.to_plain()
}

/// Run a whole stroke. A preset naming a shipped alpha stamps it, as through the engine.
pub fn run(
    s: &mut MeshStore,
    p: &BrushPreset,
    mut params: StrokeParams,
    samples: &[StrokeSample],
) -> StrokeReport {
    if params.alpha.is_none() {
        params.alpha = shipped_alpha(p);
    }
    ok(stroke(s, p, params, samples))
}

/// Run a whole stroke of the shipped preset `stem`.
pub fn run_preset(
    s: &mut MeshStore,
    stem: &str,
    params: StrokeParams,
    samples: &[StrokeSample],
) -> StrokeReport {
    run(s, &preset(stem).preset, params, samples)
}

/// Red as the color column holds it (red lowest), and a dark blue.
pub const RED: u32 = 0xff00_00ff;
pub const BLUE: u32 = 0xffcc_4010;

/// Default parameters through the front camera, with optional symmetry, painting red (Alt
/// blue), so a brush that paints shows on an unpainted part, which is white.
pub fn params(sym: Option<Symmetry>) -> StrokeParams {
    let mut p = StrokeParams::new(front());
    p.colors.main = RED;
    p.colors.secondary = BLUE;
    if let Some(sym) = sym {
        p.symmetry = sym;
    }
    p
}

/// Each face's hide flag; all clear when the mesh has no hide column.
pub fn hidden_faces(m: &PlainMesh) -> Vec<bool> {
    match m.attributes.get("hide").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&h| h != 0).collect(),
        _ => vec![false; m.faces.len()],
    }
}

/// A vertex attribute as floats; zeros when the mesh does not have it.
pub fn floats(m: &PlainMesh, name: &str) -> Vec<f32> {
    match m.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::F32(v)) => v.clone(),
        Some(AttrValues::U32(v)) => v.iter().map(|&x| x as f32).collect(),
        None => vec![0.0; m.positions.len()],
    }
}

/// Each vertex's mask and both UV hints (decision 0061, section 6), the scalar columns a brush
/// paints.
pub fn scalars(m: &PlainMesh) -> Vec<[f32; 3]> {
    let [a, b, c] = ["mask", "uv_hint", "uv_density"].map(|name| floats(m, name));
    (0..a.len()).map(|i| [a[i], b[i], c[i]]).collect()
}

/// [`scalars`] in one list.
pub fn flat(m: &PlainMesh) -> Vec<f32> {
    scalars(m).concat()
}

/// For each vertex, the vertex at its mirror image across x = 0 (positions match bit for bit).
pub fn mirror_pairs(m: &PlainMesh) -> Vec<usize> {
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let index: BTreeMap<[u32; 3], usize> = (m.positions.iter().enumerate())
        .map(|(i, &p)| (key(p), i))
        .collect();
    (m.positions.iter())
        .map(|&[x, y, z]| {
            let mirrored = key([-x, y, z]);
            let plus_zero = key([if x == 0.0 { 0.0 } else { -x }, y, z]);
            *index
                .get(&mirrored)
                .or_else(|| index.get(&plus_zero))
                .unwrap_or_else(|| panic!("no mirror of {x} {y} {z}"))
        })
        .collect()
}

/// Largest distance between matching vertices.
pub fn max_diff(a: &PlainMesh, b: &PlainMesh) -> f32 {
    let d =
        |p: &[f32; 3], q: &[f32; 3]| (Vec3::from_array(*p) - Vec3::from_array(*q)).len2().sqrt();
    a.positions
        .iter()
        .zip(&b.positions)
        .map(|(p, q)| d(p, q))
        .fold(0.0, f32::max)
}

/// Largest difference between two value lists.
pub fn max_value_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// Distance from `p` to the segment `a`-`b`.
pub fn to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.len2().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    (p - (a + ab * t)).len2().sqrt()
}
