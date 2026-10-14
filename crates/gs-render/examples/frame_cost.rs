//! What a frame costs on a GPU (decision 0019, "Measured"): a quad sphere of about N million
//! triangles at 2560x1440, culled, fitted iso view.
//!
//! ```text
//! gate/heavy.sh cargo run -p gs-render --release --example frame_cost -- ADAPTER MILLIONS
//! ```
//!
//! Prints, for an empty frame, clay, an 8-bit MatCap, a baked look (Gray), an imported
//! flat-plus-crease pair, and Lit mode (decision 0023: Gray lit live, and Skin 4 with live
//! subsurface on): the median milliseconds per frame over 9 runs of 20 frames, each run timed
//! to a full queue wait, with every frame drawn as after a camera move (one fresh sample). Then,
//! for clay, the baked look and the two lit looks, each pass's GPU time from timestamps (median
//! of 9) for a moving frame, an accumulating frame and a settled frame, and the wall time from a
//! camera move until 16 samples are in.
//!
//! With `parts` after MILLIONS (decision 0033, section 5): the same triangles as 5 quad spheres
//! in a row, clay and the baked look, each without and with the active part's outline: the
//! moving frame as above, then each pass's GPU time for a moving frame.
//!
//! With `colors` after MILLIONS (decision 0039, section 9): the sphere without a color column,
//! with a point color on every point, and with a face group on every face shown, in clay, the
//! baked look and Lit gray: the moving frame and each pass's GPU time. Then the GPU bytes
//! of both uploads, and a dab's nodes repainted on a store of the sphere and sent as a
//! color-only update, against the same nodes' positions sent: the median milliseconds until
//! the GPU has them, and the bytes sent.

#[expect(dead_code, reason = "shared with the tests, which use the rest")]
#[path = "../tests/render/scene.rs"]
mod scene;

use gs_core::transform::Similarity;
use gs_core::{AttrDef, AttrVec, ColumnSet, MeshStore, NodeConfig, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_render::matcap::{Library, MatCapImage, bake};
use gs_render::{
    AdapterKind, Camera, FrameTarget, GpuMesh, Preset, RenderSize, Renderer, Scene, Shading,
    Subsurface, SubsurfaceMode, UiLayer, UploadOptions, View,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WIDTH: u32 = 2560;
const HEIGHT: u32 = 1440;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = AdapterKind::from_name(args.first().map_or("nvidia", String::as_str))
        .ok_or("adapter is lavapipe, nvidia or radv")?;
    let millions: f64 = args.get(1).map_or("10", String::as_str).parse()?;
    let n = (millions * 1e6 / 12.0).sqrt() as u32;
    let r = Renderer::new(kind)?;
    match args.get(2).map(String::as_str) {
        Some("parts") => return parts(&r, millions),
        Some("colors") => return colors(&r, n),
        _ => {}
    }
    let mesh = r.upload(&scene::quad_sphere(n), UploadOptions::default())?;
    let pixels: Vec<[u16; 4]> = (0..64 * 64)
        .map(|i| [((i % 64) * 1000) as u16, 30000, 20000, u16::MAX])
        .collect();
    let image = MatCapImage { size: 64, pixels };
    let flat = r.upload_matcap(&image);
    let pair = r.upload_matcap_pair(&image, &image);
    let library = Library::shipped();
    let gray = library.get("gray").ok_or("the gray look")?;
    let baked = r.upload_bake(&*bake(&gray.recipe)?);
    let camera = Camera::fitted(Preset::Iso, &mesh.fit_points(), 16.0 / 9.0);
    eprintln!(
        "{} | {} triangles | {WIDTH}x{HEIGHT}",
        r.gpu().adapter_name(),
        mesh.triangle_count()
    );
    let view = |shading| View::new(camera, shading);
    moving(&r, &[], view(Shading::Clay), "empty frame")?;
    moving(&r, &[&mesh], view(Shading::Clay), "clay")?;
    moving(&r, &[&mesh], view(Shading::MatCap(&flat)), "matcap image")?;
    moving(
        &r,
        &[&mesh],
        view(Shading::MatCap(&baked)),
        "matcap baked gray",
    )?;
    moving(&r, &[&mesh], view(Shading::MatCap(&pair)), "matcap pair")?;
    let skin = library.get("skin-4").ok_or("the skin 4 look")?;
    let lit_gray = view(Shading::Lit(&gray.recipe.material));
    let scattering = View {
        subsurface: Subsurface {
            mode: SubsurfaceMode::On,
            ..Subsurface::DEFAULT
        },
        ..view(Shading::Lit(&skin.recipe.material))
    };
    moving(&r, &[&mesh], lit_gray, "lit gray")?;
    moving(&r, &[&mesh], scattering, "lit skin 4, subsurface on")?;
    eprintln!("clay:");
    passes(&r, &mesh, view(Shading::Clay))?;
    eprintln!("matcap baked gray:");
    passes(&r, &mesh, view(Shading::MatCap(&baked)))?;
    eprintln!("lit gray:");
    passes(&r, &mesh, lit_gray)?;
    eprintln!("lit skin 4, subsurface on:");
    passes(&r, &mesh, scattering)?;
    Ok(())
}

fn target(r: &Renderer, samples: u32) -> Result<FrameTarget> {
    Ok(r.frame_target(RenderSize {
        width: WIDTH,
        height: HEIGHT,
        samples,
    })?)
}

/// Reading back a 1x1 frame waits for the whole queue (as the perf suite syncs).
fn sync(r: &Renderer) -> Result<()> {
    let one = r.frame_target(RenderSize::square(1))?;
    r.read_frame(&one)?;
    Ok(())
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[expect(clippy::disallowed_methods, reason = "a timing tool, not engine code")]
fn moving(r: &Renderer, meshes: &[&GpuMesh], view: View, label: &str) -> Result<()> {
    let t = target(r, 16)?;
    let scene = Scene {
        meshes,
        view,
        viewport: [0, 0, WIDTH, HEIGHT],
    };
    let mut painter = r.ui_painter();
    let mut draw = || {
        t.restart();
        r.draw_frame(&t, &scene, &mut painter, &UiLayer::default())
    };
    for _ in 0..5 {
        draw()?;
    }
    sync(r)?;
    let mut runs = Vec::new();
    for _ in 0..9 {
        let start = std::time::Instant::now();
        for _ in 0..20 {
            draw()?;
        }
        sync(r)?;
        runs.push(start.elapsed().as_secs_f64() * 1e3 / 20.0);
    }
    eprintln!("{label}: median {:.2} ms per moving frame", median(runs));
    Ok(())
}

/// Per-pass GPU times for a moving, an accumulating and a settled frame, and the time to
/// settle.
#[expect(clippy::disallowed_methods, reason = "a timing tool, not engine code")]
fn passes(r: &Renderer, mesh: &GpuMesh, view: View) -> Result<()> {
    let t = target(r, 16)?;
    let scene = Scene {
        meshes: &[mesh],
        view,
        viewport: [0, 0, WIDTH, HEIGHT],
    };
    // Per pass: GPU milliseconds of each run for a moving, an accumulating and a settled frame.
    let mut table: Vec<(&str, [Vec<f64>; 3])> = Vec::new();
    for _ in 0..9 {
        t.restart();
        let moving = r.time_frame(&t, &scene)?;
        let accumulating = r.time_frame(&t, &scene)?;
        for _ in 0..14 {
            r.time_frame(&t, &scene)?;
        }
        let settled = r.time_frame(&t, &scene)?;
        for &(name, _) in &moving {
            if !table.iter().any(|row| row.0 == name) {
                table.push((name, [Vec::new(), Vec::new(), Vec::new()]));
            }
            let pick =
                |list: &[(&str, f64)]| list.iter().find(|(n, _)| *n == name).map_or(0.0, |p| p.1);
            if let Some(row) = table.iter_mut().find(|row| row.0 == name) {
                row.1[0].push(pick(&moving));
                row.1[1].push(pick(&accumulating));
                row.1[2].push(pick(&settled));
            }
        }
    }
    if table.is_empty() {
        eprintln!("no timestamp queries on this adapter");
    }
    eprintln!("pass: moving / accumulating / settled, GPU ms (median of 9)");
    let mut sums = [0.0; 3];
    for (name, [a, b, c]) in table {
        let m = [median(a), median(b), median(c)];
        for k in 0..3 {
            sums[k] += m[k];
        }
        eprintln!("  {name}: {:.3} / {:.3} / {:.3}", m[0], m[1], m[2]);
    }
    eprintln!(
        "  all passes: {:.3} / {:.3} / {:.3}",
        sums[0], sums[1], sums[2]
    );
    let mut painter = r.ui_painter();
    let mut settle = Vec::new();
    for _ in 0..5 {
        t.restart();
        sync(r)?;
        let start = std::time::Instant::now();
        let mut frames = 0;
        while !t.settled() || frames == 0 {
            r.draw_frame(&t, &scene, &mut painter, &UiLayer::default())?;
            frames += 1;
        }
        sync(r)?;
        settle.push(start.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!(
        "from a camera move to 16 samples: median {:.1} ms",
        median(settle)
    );
    Ok(())
}

/// 5 quad spheres of `millions / 5` million triangles each in a row, the middle one marked:
/// clay and the baked look, each without and with the outline.
fn parts(r: &Renderer, millions: f64) -> Result<()> {
    let n = (millions * 1e6 / 5.0 / 12.0).sqrt() as u32;
    let sphere = scene::quad_sphere(n);
    let mut meshes = Vec::new();
    for i in 0..5 {
        let at = Similarity {
            translation: Vec3::new(2.5 * (i as f32 - 2.0), 0.0, 0.0),
            ..Similarity::IDENTITY
        };
        let mesh = r.upload(&sphere, UploadOptions::default())?;
        meshes.push(mesh.placed(at, true).marked(i == 2));
    }
    let library = Library::shipped();
    let gray = library.get("gray").ok_or("the gray look")?;
    let baked = r.upload_bake(&*bake(&gray.recipe)?);
    let points: Vec<[f32; 3]> = meshes.iter().flat_map(GpuMesh::fit_points).collect();
    let camera = Camera::fitted(Preset::Iso, &points, 16.0 / 9.0);
    let triangles: u64 = meshes.iter().map(|m| u64::from(m.triangle_count())).sum();
    eprintln!(
        "{} | 5 parts, {triangles} triangles | {WIDTH}x{HEIGHT}",
        r.gpu().adapter_name()
    );
    let drawn: Vec<&GpuMesh> = meshes.iter().collect();
    for (name, shading) in [
        ("clay", Shading::Clay),
        ("matcap baked gray", Shading::MatCap(&baked)),
    ] {
        for outline in [false, true] {
            let view = View {
                outline,
                ..View::new(camera, shading)
            };
            let label = format!("{name}, outline {}", if outline { "on" } else { "off" });
            moving(r, &drawn, view, &label)?;
            pass_times(r, &drawn, view)?;
        }
    }
    Ok(())
}

/// Each pass's GPU time for a moving frame of `meshes` (median of 9).
fn pass_times(r: &Renderer, meshes: &[&GpuMesh], view: View) -> Result<()> {
    let t = target(r, 16)?;
    let scene = Scene {
        meshes,
        view,
        viewport: [0, 0, WIDTH, HEIGHT],
    };
    let mut table: Vec<(&str, Vec<f64>)> = Vec::new();
    for _ in 0..9 {
        t.restart();
        for (name, ms) in r.time_frame(&t, &scene)? {
            match table.iter_mut().find(|row| row.0 == name) {
                Some(row) => row.1.push(ms),
                None => table.push((name, vec![ms])),
            }
        }
    }
    let line: Vec<String> = (table.into_iter())
        .map(|(name, ms)| format!("{name} {:.3}", median(ms)))
        .collect();
    eprintln!("  moving frame, GPU ms: {}", line.join(", "));
    Ok(())
}

/// `mesh` with a point color on every point (its position as a color) and a face group on
/// every face (the octant of its first corner, 1 to 8).
fn painted(mut mesh: PlainMesh) -> PlainMesh {
    let byte = |v: f32| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
    let words = (mesh.positions.iter())
        .map(|p| u32::from_le_bytes([byte(p[0]), byte(p[1]), byte(p[2]), 255]))
        .collect();
    let octant = |p: [f32; 3]| 1 + (0..3).map(|k| u32::from(p[k] > 0.0) << k).sum::<u32>();
    let groups = (mesh.faces.iter())
        .map(|f| octant(mesh.positions[f[0] as usize]))
        .collect();
    let column = |domain, values| Attribute { domain, values };
    let colors = column(AttrDomain::Vertex, AttrValues::U32(words));
    let groups = column(AttrDomain::Face, AttrValues::U32(groups));
    mesh.attributes.insert("color".into(), colors);
    mesh.attributes.insert("group".into(), groups);
    mesh
}

/// A quad sphere of `n` by `n` quads a side, with no color column, with point colors and with
/// face groups shown, in clay, the baked look and Lit gray; then the uploads' bytes and a
/// color-only update.
fn colors(r: &Renderer, n: u32) -> Result<()> {
    let sphere = scene::quad_sphere(n);
    let plain = r.upload(&sphere, UploadOptions::default())?;
    let colored = r.upload(&painted(sphere.clone()), UploadOptions::default())?;
    let library = Library::shipped();
    let gray = library.get("gray").ok_or("the gray look")?;
    let baked = r.upload_bake(&*bake(&gray.recipe)?);
    let camera = Camera::fitted(Preset::Iso, &plain.fit_points(), 16.0 / 9.0);
    eprintln!(
        "{} | {} triangles | {WIDTH}x{HEIGHT} | colors",
        r.gpu().adapter_name(),
        plain.triangle_count()
    );
    for (name, shading) in [
        ("clay", Shading::Clay),
        ("matcap baked gray", Shading::MatCap(&baked)),
        ("lit gray", Shading::Lit(&gray.recipe.material)),
    ] {
        for (what, mesh, face_groups) in [
            ("no color column", &plain, false),
            ("point colors", &colored, false),
            ("face groups", &colored, true),
        ] {
            let view = View {
                face_groups,
                ..View::new(camera, shading)
            };
            moving(r, &[mesh], view, &format!("{name}, {what}"))?;
            pass_times(r, &[mesh], view)?;
        }
    }
    eprintln!(
        "GPU mesh bytes: {} without colors, {} with point and group colors",
        plain.bytes(),
        colored.bytes()
    );
    color_update(r, &sphere)
}

/// A dab of radius 0.25 on a store of `sphere` (default node size) repainted, then moved by a
/// hair, each sent as an update: median milliseconds of 9 until the GPU has it, and the bytes.
#[expect(clippy::disallowed_methods, reason = "a timing tool, not engine code")]
fn color_update(r: &Renderer, sphere: &PlainMesh) -> Result<()> {
    let mut store = MeshStore::from_plain(sphere, NodeConfig::default())?;
    let color = store.add_attr(AttrDef::color())?;
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let mut nodes = store.query(Vec3::new(0.0, 0.0, 1.0), 0.25).brushed;
    nodes.sort_unstable();
    let points: usize = nodes.iter().map(|&n| store.node(n).pos.len()).sum();
    eprintln!(
        "a dab: {} of {} nodes, {points} points",
        nodes.len(),
        store.node_count()
    );
    for what in ["colors", "positions"] {
        let (mut runs, mut sent) = (Vec::new(), None);
        for k in 0..9u8 {
            if what == "colors" {
                let word = u32::from_le_bytes([255, 20 * k, 40, 255]);
                store.write(&nodes, ColumnSet::attr(color)).each(|m| {
                    if let AttrVec::U32(v) = m.attr(color) {
                        v.fill(word);
                    }
                });
            } else {
                let s = if k % 2 == 0 { 1.0001 } else { 1.0 / 1.0001 };
                store.write(&nodes, ColumnSet::POSITION).each(|m| {
                    for p in m.positions() {
                        *p = *p * s;
                    }
                });
            }
            let dirty = store.take_dirty();
            sync(r)?;
            let start = std::time::Instant::now();
            sent = Some(r.update_store(&mut mesh, &store, &dirty));
            sync(r)?;
            runs.push(start.elapsed().as_secs_f64() * 1e3);
        }
        let sent = sent.ok_or("nine runs")?;
        eprintln!(
            "{what} sent: median {:.3} ms, {} bytes, {} nodes' vertices, full {}",
            median(runs),
            sent.bytes,
            sent.nodes,
            sent.full
        );
    }
    Ok(())
}
