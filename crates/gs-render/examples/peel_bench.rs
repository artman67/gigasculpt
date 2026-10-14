//! What Live Boolean's peel costs on a GPU, and how many layers its rays need (decision 0057,
//! section 9, as amended for LB8).
//!
//! ```text
//! gate/heavy.sh cargo run -p gs-render --release --locked --example peel_bench -- ADAPTER [RUNS]
//! ```
//!
//! Research 26's 1M body (`sphere(289)`, 1,002,252 triangles) minus its 20k cutter
//! (`sphere(41)` placed by `shapes::cutter()`), then the same cutter in a 10M body
//! (`sphere(913)`, 10,002,828 triangles), at 2560x1440 in the fitted iso view, every frame drawn
//! as after a camera move. For each: each pass's GPU time from timestamp queries (median of
//! RUNS, 9 by default), the peel's own (its layer, resolve and last passes) summed, with the cutter
//! peeled and outlined as the window draws it and with both drawn as parts; then the wall time
//! of a moving frame both ways (median of RUNS runs of 5 frames).
//!
//! Then the layers the peel needs on the 1M pair's rays (research 26's pair, `sphere(204)` and
//! its copy placed by `shapes::pair_b()`), from the CPU, for each operation: one ray a pixel of a
//! 1024 by 1024 orthographic view down each axis, and the layer at which each ray that hits the
//! pair first finds the cut inside (the one the peel draws).

#[expect(
    dead_code,
    reason = "shared with gs-geo's boolean tests, which use the rest"
)]
#[path = "../../gs-geo/tests/boolean/shapes.rs"]
mod shapes;

use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_protocol::{NO_VERTEX, PlainMesh};
use gs_render::{
    AdapterKind, Camera, GpuMesh, Operand, Preset, RenderSize, Renderer, Scene, Shading, UiLayer,
    UploadOptions, View,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WIDTH: u32 = 2560;
const HEIGHT: u32 = 1440;
/// The theme's Subtract tint is the window's; any color costs the same.
const TINT: [u8; 3] = [230, 90, 70];
/// The CPU's rays: a `GRID` by `GRID` view of the square from `-EXTENT` to `EXTENT`.
const GRID: usize = 1024;
const EXTENT: f32 = 1.5;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = AdapterKind::from_name(args.first().map_or("nvidia", String::as_str))
        .ok_or("adapter is lavapipe, nvidia or radv")?;
    let runs: usize = args.get(1).map_or("9", String::as_str).parse()?;
    let r = Renderer::new(kind)?;
    eprintln!(
        "{} | {WIDTH}x{HEIGHT} | peels: {}",
        r.gpu().adapter_name(),
        r.peels()
    );
    let cutter = shapes::sphere(41);
    for n in [289, 913] {
        timed(&r, (&shapes::sphere(n), &cutter), runs)?;
    }
    let pair = shapes::sphere(204);
    for operation in ["union", "difference", "intersect"] {
        for axis in 0..3 {
            let parts = [(&pair, Similarity::IDENTITY), (&pair, shapes::pair_b())];
            layers(&crossings(&parts, axis), operation, axis);
        }
    }
    Ok(())
}

/// `body` minus `cutter` placed by `shapes::cutter()`: each pass's GPU time and the moving
/// frame's wall time, peeled and drawn as parts.
fn timed(r: &Renderer, (body, cutter): (&PlainMesh, &PlainMesh), runs: usize) -> Result<()> {
    let base = r.upload(body, UploadOptions::default())?;
    let cut = r.upload(cutter, UploadOptions::default())?;
    let cut = cut.placed(shapes::cutter(), true);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let view = View::new(
        Camera::fitted(Preset::Iso, &base.fit_points(), aspect),
        Shading::Clay,
    );
    eprintln!(
        "{} triangles minus {}:",
        base.triangle_count(),
        cut.triangle_count()
    );
    let base = base.peeled(Some(Operand::Base));
    let cut = cut
        .peeled(Some(Operand::Difference(TINT)))
        .ghosted(Some(TINT));
    frames(r, &[&base, &cut], view, ("peeled", runs))?;
    let (base, cut) = (base.peeled(None), cut.peeled(None).ghosted(None));
    frames(r, &[&base, &cut], view, ("as parts", runs))
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Each pass's GPU time of a moving frame of `meshes` (median of `runs`), the peel's summed,
/// then the moving frame's wall time.
#[expect(clippy::disallowed_methods, reason = "a timing tool, not engine code")]
fn frames(
    r: &Renderer,
    meshes: &[&GpuMesh],
    view: View,
    (label, runs): (&str, usize),
) -> Result<()> {
    let size = RenderSize {
        width: WIDTH,
        height: HEIGHT,
        samples: 16,
    };
    let t = r.frame_target(size)?;
    let scene = Scene {
        meshes,
        view,
        viewport: [0, 0, WIDTH, HEIGHT],
    };
    let mut table: Vec<(&str, Vec<f64>)> = Vec::new();
    for _ in 0..runs {
        t.restart();
        for (name, ms) in r.time_frame(&t, &scene)? {
            match table.iter_mut().find(|row| row.0 == name) {
                Some(row) => row.1.push(ms),
                None => table.push((name, vec![ms])),
            }
        }
    }
    let rows: Vec<(&str, f64)> = table.into_iter().map(|(n, ms)| (n, median(ms))).collect();
    let peel: f64 = (rows.iter())
        .filter(|(n, _)| n.starts_with("peel"))
        .map(|r| r.1)
        .sum();
    let all: f64 = rows.iter().map(|r| r.1).sum();
    let line: Vec<String> = rows.iter().map(|(n, ms)| format!("{n} {ms:.3}")).collect();
    eprintln!("  {label}, GPU ms: {}", line.join(", "));
    eprintln!(
        "  {label}: the peel {peel:.3} ms, all passes {all:.3} ms; targets {} bytes",
        t.peel_bytes()
    );
    let mut painter = r.ui_painter();
    let mut draw = || {
        t.restart();
        r.draw_frame(&t, &scene, &mut painter, &UiLayer::default())
    };
    draw()?;
    r.read_frame(&r.frame_target(RenderSize::square(1))?)?;
    let mut walls = Vec::new();
    for _ in 0..runs {
        let start = std::time::Instant::now();
        for _ in 0..5 {
            draw()?;
        }
        // Reading back a 1x1 frame waits for the whole queue.
        r.read_frame(&r.frame_target(RenderSize::square(1))?)?;
        walls.push(start.elapsed().as_secs_f64() * 1e3 / 5.0);
    }
    eprintln!(
        "  {label}: median {:.2} ms a moving frame (wall)",
        median(walls)
    );
    Ok(())
}

/// A ray's crossings of the pair's surfaces: its depth along the ray, the operand, and whether
/// it faces the ray (a front face).
type Crossing = (f32, usize, bool);

/// Each ray's crossings, one ray a pixel of a `GRID` by `GRID` orthographic view down `axis`.
fn crossings(parts: &[(&PlainMesh, Similarity)], axis: usize) -> Vec<Vec<Crossing>> {
    let mut rays = vec![Vec::new(); GRID * GRID];
    for (operand, &(mesh, at)) in parts.iter().enumerate() {
        let points: Vec<[f32; 3]> = (mesh.positions.iter())
            .map(|&p| at.point(Vec3::from_array(p)).to_array())
            .collect();
        for face in &mesh.faces {
            let corners: Vec<[f32; 3]> = (face.iter())
                .filter(|&&c| c != NO_VERTEX)
                .map(|&c| points[c as usize])
                .collect();
            for k in 1..corners.len() - 1 {
                let triangle = [corners[0], corners[k], corners[k + 1]];
                cross(&mut rays, triangle, axis, operand);
            }
        }
    }
    rays
}

/// Adds `triangle`'s crossing to each ray whose pixel center it covers, the rays looking down
/// `axis` (depth is minus the coordinate along it).
fn cross(rays: &mut [Vec<Crossing>], triangle: [[f32; 3]; 3], axis: usize, operand: usize) {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let flat = triangle.map(|p| [p[u], p[v]]);
    // Twice the signed area of `p` and the edge across from corner `k`.
    let edge = |k: usize, p: [f32; 2]| {
        let (a, b) = (flat[(k + 1) % 3], flat[(k + 2) % 3]);
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let area = edge(2, flat[2]);
    if area == 0.0 {
        return;
    }
    let cell = 2.0 * EXTENT / GRID as f32;
    let pixel = |x: f32| ((x + EXTENT) / cell - 0.5).clamp(0.0, (GRID - 1) as f32);
    let span = |k: usize| {
        let lo = flat.iter().map(|p| p[k]).fold(f32::MAX, f32::min);
        let hi = flat.iter().map(|p| p[k]).fold(f32::MIN, f32::max);
        pixel(lo).ceil() as usize..=pixel(hi).floor() as usize
    };
    for j in span(1) {
        for i in span(0) {
            let p = [
                (i as f32 + 0.5) * cell - EXTENT,
                (j as f32 + 0.5) * cell - EXTENT,
            ];
            let w = [0, 1, 2].map(|k| edge(k, p) / area);
            if w.iter().all(|&w| w >= 0.0) {
                let depth = -(0..3).map(|k| w[k] * triangle[k][axis]).sum::<f32>();
                rays[j * GRID + i].push((depth, operand, area > 0.0));
            }
        }
    }
}

/// The histogram of the layer at which each ray first finds `operation` of the pair inside,
/// with its mean and 99th percentile, and the rays that never do.
fn layers(rays: &[Vec<Crossing>], operation: &str, axis: usize) {
    let inside = |m: [bool; 2]| match operation {
        "union" => m[0] || m[1],
        "difference" => m[0] && !m[1],
        _ => m[0] && m[1],
    };
    let mut found: Vec<usize> = Vec::new();
    let mut never = 0;
    for ray in rays.iter().filter(|r| !r.is_empty()) {
        let mut ray = ray.clone();
        ray.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut mask = [false; 2];
        let first = ray.iter().position(|&(_, operand, front)| {
            mask[operand] = front;
            inside(mask)
        });
        match first {
            Some(k) => found.push(k + 1),
            None => never += 1,
        }
    }
    found.sort_unstable();
    let most = found.last().copied().unwrap_or(0);
    let counts: Vec<String> = (1..=most)
        .map(|k| format!("{k}: {}", found.iter().filter(|&&f| f == k).count()))
        .collect();
    let mean = found.iter().sum::<usize>() as f64 / found.len().max(1) as f64;
    let p99 = found.get(found.len() * 99 / 100).copied().unwrap_or(0);
    eprintln!(
        "{operation} down {}: {} rays find it at layer {}; mean {mean:.2}, p99 {p99}; {never} never",
        ["x", "y", "z"][axis],
        found.len(),
        counts.join(", ")
    );
}
