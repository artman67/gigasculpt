//! Preview pictures and timing for gs-render.
//!
//! ```text
//! cargo run -p gs-render --release --example previews -- render [adapter] [out-dir]
//! cargo run -p gs-render --release --example previews -- time [adapter] [quads-per-cube-edge]
//! cargo run -p gs-render --release --example previews -- bake [out-dir]
//! cargo run -p gs-render --release --example previews -- transforms [adapter] [out-dir]
//! cargo run -p gs-render --release --example previews -- occlusion [adapter] [out-dir]
//! cargo run -p gs-render --release --example previews -- studios adapter mesh-dir [out-dir]
//! ```
//!
//! `render` writes a quad sphere with a dent in clay, every shipped MatCap, zebra and
//! wireframe, and a contact sheet of every preset. `time` reports upload and snapshot times
//! for a quad sphere (289 gives about 1M triangles). `bake` bakes every shipped look, reports
//! how long each took, and writes each MatCap as it shows (PBR Neutral, exposure 0).
//! `transforms` writes a folded form in several looks under each view transform, named
//! `<transform>--<look>.png`, for choosing between them. `occlusion` writes the folded form in
//! three looks with ambient occlusion off and on, at one and sixteen samples
//! (`<look>--ao-<off|on>--<samples>.png`), and a sphere over the floor grid at three zooms a
//! thousand times apart (`grid--<zoom>.png`). `studios` bakes every look under every shipped
//! environment and draws each OBJ file in `mesh-dir` (framed as the engine frames a named view:
//! `voxel-cube` from the iso view, the rest from the front) at sixteen samples, named
//! `<environment>--<look>--<shape>.png`, for choosing the default environment; it first reports
//! the turn each shipped probe is given.

#[expect(dead_code, reason = "shared with the tests, which use the rest")]
#[path = "../tests/render/scene.rs"]
mod scene;

use gs_render::display::encode;
use gs_render::matcap::{Baked, Library, bake};
use gs_render::{
    AdapterKind, Camera, Display, Image, Occlusion, Preset, RenderSize, Renderer, Shading,
    UploadOptions, View, ViewTransform, Wire,
};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, default: &str| args.get(i).cloned().unwrap_or_else(|| default.to_owned());
    match arg(0, "render").as_str() {
        "render" => render(
            AdapterKind::from_name(&arg(1, "lavapipe"))
                .ok_or("adapter is lavapipe, nvidia or radv")?,
            &PathBuf::from(arg(2, "previews")),
        ),
        "time" => time(
            AdapterKind::from_name(&arg(1, "lavapipe"))
                .ok_or("adapter is lavapipe, nvidia or radv")?,
            arg(2, "289").parse()?,
        ),
        "bake" => bakes(&PathBuf::from(arg(1, "previews"))),
        "transforms" => transforms(
            AdapterKind::from_name(&arg(1, "lavapipe"))
                .ok_or("adapter is lavapipe, nvidia or radv")?,
            &PathBuf::from(arg(2, "previews")),
        ),
        "occlusion" => occlusion(
            AdapterKind::from_name(&arg(1, "lavapipe"))
                .ok_or("adapter is lavapipe, nvidia or radv")?,
            &PathBuf::from(arg(2, "previews")),
        ),
        "studios" => studios(
            AdapterKind::from_name(&arg(1, "lavapipe"))
                .ok_or("adapter is lavapipe, nvidia or radv")?,
            &PathBuf::from(arg(2, "meshes")),
            &PathBuf::from(arg(3, "previews")),
        ),
        other => Err(
            format!("unknown command {other}; see the comment at the top of previews.rs").into(),
        ),
    }
}

fn sphere() -> gs_protocol::PlainMesh {
    let mut mesh = scene::quad_sphere(48);
    scene::dent(&mut mesh, 0.12);
    mesh
}

fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// A sculpted-looking form: an egg with V-shaped folds and three round pits (the same formula
/// as the MatCap preview sheets' folded form).
fn folds() -> gs_protocol::PlainMesh {
    let mut mesh = scene::quad_sphere(96);
    let pits = [[-0.35, 0.35, 0.87], [0.4, 0.1, 0.91], [0.05, -0.45, 0.89]];
    for p in &mut mesh.positions {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        let [x, y, z] = p.map(|c| f64::from(c / l));
        let wave = libm::sin(5.0 * (0.8 * x + 0.6 * y) + 1.4 * libm::sin(3.0 * z)).abs();
        let wave2 = libm::sin(7.0 * (0.5 * y - 0.85 * z) + 0.8 * libm::cos(2.0 * x)).abs();
        let mut r = 1.0 + 0.07 * (wave - 0.6) + 0.025 * (wave2 - 0.6);
        for [cx, cy, cz] in pits {
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy) + (z - cz) * (z - cz);
            r -= 0.06 * libm::exp(-d2 / 0.006);
        }
        *p = [x * r * 0.95, y * r * 1.12, z * r * 0.92].map(|c| c as f32);
    }
    mesh
}

fn transforms(kind: AdapterKind, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let renderer = Renderer::new(kind)?;
    let form = folds();
    let mesh = renderer.upload(&form, UploadOptions::default())?;
    let library = Library::shipped();
    let size = RenderSize {
        width: 512,
        height: 512,
        samples: 4,
    };
    for name in ["gray", "red-wax", "skin-4", "white", "chrome-a", "metal-1"] {
        let look = library.get(name).ok_or("a shipped look")?;
        let matcap = renderer.upload_bake(&*bake(&look.recipe)?);
        for transform in ViewTransform::ALL {
            let mut view = View::new(
                Camera::fitted(Preset::Front, &form.positions, 1.0),
                Shading::MatCap(&matcap),
            );
            view.display.transform = transform;
            let file = format!("{}--{name}.png", transform.name());
            save(
                &out.join(file),
                &renderer.snapshot(&mesh, &view, size)?.png()?,
            )?;
        }
    }
    Ok(())
}

fn occlusion(kind: AdapterKind, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let renderer = Renderer::new(kind)?;
    let form = folds();
    let mesh = renderer.upload(&form, UploadOptions::default())?;
    let library = Library::shipped();
    let camera = Camera::fitted(Preset::Front, &form.positions, 1.0);
    for name in ["gray", "red-wax", "white"] {
        let look = library.get(name).ok_or("a shipped look")?;
        let matcap = renderer.upload_bake(&*bake(&look.recipe)?);
        for (ao, occlusion) in [("off", Occlusion::OFF), ("on", Occlusion::default())] {
            for samples in [1, 16] {
                let view = View {
                    occlusion,
                    ..View::new(camera, Shading::MatCap(&matcap))
                };
                let size = RenderSize {
                    width: 768,
                    height: 768,
                    samples,
                };
                let file = format!("{name}--ao-{ao}--{samples}.png");
                save(
                    &out.join(file),
                    &renderer.snapshot(&mesh, &view, size)?.png()?,
                )?;
            }
        }
    }
    let ball = renderer.upload(&scene::quad_sphere(24), UploadOptions::default())?;
    let fitted = Camera::fitted(Preset::Iso, &ball.fit_points(), 1.0);
    // Zooming toward a point on the floor in front of the ball: the plane lies 0.2% of the
    // scene's height below its lowest point (grid.rs).
    let b = ball.bounds();
    let floor = b.min[1] - 0.002 * (b.max[1] - b.min[1]);
    for zoom in [1.0f32, 1e-3, 1e-6] {
        let camera = Camera {
            distance: fitted.distance * zoom,
            target: [0.4, floor, 1.3],
            ..fitted
        };
        let view = View {
            grid: true,
            ..View::new(camera, Shading::Clay)
        };
        let size = RenderSize {
            width: 512,
            height: 512,
            samples: 4,
        };
        let file = format!("grid--{zoom:e}.png");
        save(
            &out.join(file),
            &renderer.snapshot(&ball, &view, size)?.png()?,
        )?;
    }
    Ok(())
}

/// A Wavefront OBJ file's vertices and faces (triangles and quads only).
fn read_obj(path: &Path) -> Result<gs_protocol::PlainMesh> {
    let mut mesh = gs_protocol::PlainMesh::default();
    for line in std::fs::read_to_string(path)?.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let c: Vec<f32> = words
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?;
                mesh.positions.push([c[0], c[1], c[2]]);
            }
            Some("f") => {
                let mut face = [u32::MAX; 4];
                for (slot, word) in face.iter_mut().zip(words) {
                    *slot = word.split('/').next().unwrap_or(word).parse::<u32>()? - 1;
                }
                mesh.faces.push(face);
            }
            _ => {}
        }
    }
    Ok(mesh)
}

fn studios(kind: AdapterKind, meshes: &Path, out: &Path) -> Result<()> {
    use gs_render::shading::environment::SHIPPED;
    std::fs::create_dir_all(out)?;
    let renderer = Renderer::new(kind)?;
    let mut shapes = Vec::new();
    for shape in ["sphere", "jitter", "voxel-cube", "folds"] {
        let form = read_obj(&meshes.join(format!("{shape}.obj")))?;
        let mesh = renderer.upload(&form, UploadOptions::default())?;
        let b = mesh.bounds();
        let corners: Vec<[f32; 3]> = (0..8)
            .map(|i| {
                let pick = |bit: usize, axis: usize| {
                    if i >> bit & 1 == 1 {
                        b.max[axis]
                    } else {
                        b.min[axis]
                    }
                };
                [pick(0, 0), pick(1, 1), pick(2, 2)]
            })
            .collect();
        let preset = if shape == "voxel-cube" {
            Preset::Iso
        } else {
            Preset::Front
        };
        shapes.push((shape, mesh, Camera::fitted(preset, &corners, 1.0)));
    }
    let size = RenderSize {
        width: 512,
        height: 512,
        samples: 16,
    };
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/environments");
    for file in [
        "studio_small_09_1k.hdr",
        "studio_small_08_1k.hdr",
        "photo_studio_01_1k.hdr",
    ] {
        let probe =
            gs_render::shading::hdr::Equirect::read_hdr(&std::fs::read(assets.join(file))?)?;
        eprintln!(
            "{file}: turned {:.1} degrees",
            probe.key_turn().to_degrees()
        );
    }
    for (env, _) in SHIPPED {
        for look in Library::shipped().looks {
            let mut recipe = look.recipe.clone();
            env.clone_into(&mut recipe.environment);
            let matcap = renderer.upload_bake(&*bake(&recipe)?);
            for (shape, mesh, camera) in &shapes {
                let view = View::new(*camera, Shading::MatCap(&matcap));
                let file = format!("{env}--{}--{shape}.png", look.name);
                save(
                    &out.join(file),
                    &renderer.snapshot(mesh, &view, size)?.png()?,
                )?;
            }
        }
    }
    Ok(())
}

/// A baked MatCap as it shows: `albedo x D + S` through the default display, 8-bit sRGB.
fn shown(baked: &Baked) -> Image {
    let mut rgba = Vec::with_capacity((baked.size * baked.size * 4) as usize);
    for y in 0..baked.size {
        for x in 0..baked.size {
            let [r, g, b] = encode(Display::default().show(baked.radiance(x, y)));
            rgba.extend([r, g, b, 255]);
        }
    }
    Image {
        width: baked.size,
        height: baked.size,
        rgba,
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "a timing tool, not engine code; ARCHITECTURE 10.4 keeps wall-clock out of gates"
)]
fn bakes(out: &Path) -> Result<()> {
    use std::time::Instant;
    std::fs::create_dir_all(out)?;
    for look in Library::shipped().looks {
        let t = Instant::now();
        let baked = bake(&look.recipe)?;
        eprintln!("{}: {:.0} ms", look.name, t.elapsed().as_secs_f64() * 1e3);
        save(
            &out.join(format!("matcap-{}.png", look.name)),
            &shown(&baked).png()?,
        )?;
    }
    Ok(())
}

fn render(kind: AdapterKind, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let renderer = Renderer::new(kind)?;
    let sphere = sphere();
    let mesh = renderer.upload(&sphere, UploadOptions::default())?;
    // Framed on the vertices themselves: tighter than the chunk boxes for a small mesh.
    let frame = |preset| Camera::fitted(preset, &sphere.positions, 1.0);
    let save = |name: &str, bytes: Vec<u8>| save(&out.join(name), &bytes);
    let library = Library::shipped();
    let mut looks = Vec::new();
    for look in &library.looks {
        looks.push((
            look.name.clone(),
            renderer.upload_bake(&*bake(&look.recipe)?),
        ));
    }
    let size = RenderSize {
        width: 768,
        height: 768,
        samples: 4,
    };
    let mut shots = vec![
        ("sphere-clay.png".to_owned(), Shading::Clay, Wire::Off),
        ("sphere-zebra.png".to_owned(), Shading::Zebra, Wire::Off),
    ];
    for (name, matcap) in &looks {
        shots.push((
            format!("sphere-matcap-{name}.png"),
            Shading::MatCap(matcap),
            Wire::Off,
        ));
    }
    shots.push((
        "sphere-wire.png".to_owned(),
        Shading::MatCap(&looks[0].1),
        Wire::Auto,
    ));
    for (file, shading, wire) in shots {
        let view = View {
            wire,
            ..View::new(frame(Preset::Iso), shading)
        };
        save(&file, renderer.snapshot(&mesh, &view, size)?.png()?)?;
    }
    let shading = Shading::MatCap(&looks[1].1);
    let views: Vec<(&str, View)> = Preset::ALL
        .iter()
        .map(|&p| {
            let view = View::new(frame(p), shading);
            (p.name(), view)
        })
        .collect();
    let tile = RenderSize {
        width: 384,
        height: 384,
        samples: 4,
    };
    save(
        "contact-sheet.png",
        renderer.contact_sheet(&mesh, &views, tile, 4)?.png()?,
    )
}

#[expect(
    clippy::disallowed_methods,
    reason = "a timing tool, not engine code; ARCHITECTURE 10.4 keeps wall-clock out of gates"
)]
fn time(kind: AdapterKind, n: u32) -> Result<()> {
    use std::time::Instant;
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let renderer = Renderer::new(kind)?;
    let open = ms(t);
    let mut sphere = scene::quad_sphere(n);
    scene::dent(&mut sphere, 0.12);
    let t = Instant::now();
    let mesh = renderer.upload(&sphere, UploadOptions::default())?;
    let upload = ms(t);
    for (size, samples) in [(1024, 1), (1024, 4), (512, 1)] {
        let target = RenderSize {
            width: size,
            height: size,
            samples,
        };
        let v = View::new(
            Camera::fitted(Preset::Iso, &sphere.positions, 1.0),
            Shading::Clay,
        );
        let t = Instant::now();
        renderer.snapshot(&mesh, &v, target)?;
        let first = ms(t);
        let mut warm: Vec<f64> = (0..9)
            .map(|_| {
                let t = Instant::now();
                renderer.snapshot(&mesh, &v, target).map(|_| ms(t))
            })
            .collect::<std::result::Result<_, _>>()?;
        warm.sort_by(f64::total_cmp);
        eprintln!(
            "{} | {} triangles | open {open:.0} ms | upload {upload:.0} ms | {size}x{size} x{samples}: first {first:.1} ms, warm median {:.1} ms (min {:.1})",
            renderer.gpu().adapter_name(),
            mesh.triangle_count(),
            warm[4],
            warm[0]
        );
    }
    Ok(())
}
