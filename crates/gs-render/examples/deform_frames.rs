//! What the deformer preview costs the graphics card (decision 0041, amended 2026-10-12 for
//! `DF9a`, section 11, and 2026-10-13 for `DF9b`). At 10M points (a quad sphere of 10,077,698),
//! for a bend, noise, a soft 4 by 4 by 4 cage and a curve through 32 points under a gradient: the open's
//! weighing on the CPU (gs-core `MeshStore::weights`), the open's upload (the weights, the block
//! and any table: the bytes, and the time until the GPU has them), the table's build alone
//! (gs-core `Transform::prepared`), an update's block and table, and against them one update
//! of the CPU path (the transform on the CPU, then the changed nodes sent, `update_store`). At
//! about 1M points (998,786): a moving frame at 1920x1080 without and with each preview, from
//! the same camera. Medians of 5; the peak memory (`VmHWM`) at the end. Lavapipe's times are
//! relative only.
//!
//! ```text
//! gate/heavy.sh cargo run -p gs-render --release --example deform_frames -- ADAPTER [N] [M]
//! ```
//!
//! `N` and `M` are the spheres' cells per cube edge (default 1296 and 408).

mod peak;
#[expect(dead_code, reason = "shared with the tests, which use the rest")]
#[path = "../tests/render/scene.rs"]
mod scene;

use std::time::Instant;

use gs_core::deform::{Curve, Deform, Gradient, Lattice, Offset, Scope};
use gs_core::symmetry::Symmetry;
use gs_core::transform::{Distance, Frame, Op, RIGID, Transform, Weights};
use gs_core::{Aabb, MeshStore, NodeConfig, Quat, Vec3};
use gs_render::{
    AdapterKind, Camera, GpuMesh, Preset, Preview, RenderSize, Renderer, Scene, Shading, UiLayer,
    View,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[expect(clippy::disallowed_methods, reason = "a timing tool reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn ms(from: Instant) -> f64 {
    from.elapsed().as_secs_f64() * 1e3
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Reading back a 1x1 frame waits for the whole queue (as the perf suite syncs).
fn sync(r: &Renderer) -> Result<()> {
    r.read_frame(&r.frame_target(RenderSize::square(1))?)?;
    Ok(())
}

/// A rigid `op` about the frame and in the box the window sends (`DF7a`), faded by `gradient`.
fn placed(op: Deform, gradient: Option<Gradient>) -> [Transform; 1] {
    [Transform {
        frame: Frame {
            origin: Vec3::new(0.05, 0.03, 0.02),
            axes: Quat::IDENTITY,
        },
        op: Op::Deform(op),
        weights: Weights {
            mask: true,
            falloff: RIGID,
            reach: 1.0,
            distance: Distance::Straight,
        },
        symmetry: Symmetry::default(),
        scope: Scope {
            bounds: Some(Aabb {
                lo: Vec3::new(-1.05, -1.03, -1.02),
                hi: Vec3::new(0.95, 0.97, 0.98),
            }),
            gradient,
            ..Scope::default()
        },
    }]
}

/// A bend of `k / 5` radians toward x along y.
fn bend(k: f32) -> [Transform; 1] {
    let op = Deform::Bend {
        axis: 1,
        toward: 0,
        angle: 0.2 * k,
    };
    placed(op, None)
}

/// Noise of cells 0.05 wide (40 across the part), `0.004 k` deep: half a cell at step 7.
fn noise(k: f32) -> [Transform; 1] {
    let op = Deform::Noise {
        amount: 0.004 * k,
        scale: 0.05,
        seed: 7,
    };
    placed(op, None)
}

/// A soft 4 by 4 by 4 cage: one inner point lifted `0.02 k`, two more moved and held.
fn cage(k: f32) -> [Transform; 1] {
    let offsets = vec![
        Offset {
            at: [1, 2, 1],
            by: [0.0, 0.02 * k, 0.0],
        },
        Offset {
            at: [2, 1, 2],
            by: [0.1, 0.0, -0.05],
        },
        Offset {
            at: [2, 3, 1],
            by: [0.0, 0.0, 0.1],
        },
    ];
    let op = Deform::Lattice(Lattice {
        soft: true,
        size: [4, 4, 4],
        offsets,
    });
    placed(op, None)
}

/// A curve through 32 points along y, a wave across the part, twisted `k / 6` radians from end
/// to end, faded along y by a gradient.
fn curve(k: f32) -> [Transform; 1] {
    let points = (0..32)
        .map(|i| {
            let y = -1.03 + 2.0 * i as f32 / 31.0;
            [
                0.2 * libm::sinf(3.0 * y),
                y,
                0.15 * libm::cosf(2.0 * y) - 0.15,
            ]
        })
        .collect();
    let twist = (0..32).map(|i| k / 6.0 * i as f32 / 31.0).collect();
    let scale = (0..32)
        .map(|i| 1.0 + 0.1 * libm::sinf(i as f32 / 4.0))
        .collect();
    let op = Deform::Curve(Curve {
        axis: 1,
        points: Some(points),
        twist: Some(twist),
        scale: Some(scale),
    });
    let gradient = Gradient {
        axis: 1,
        from: 0.0,
        to: 1.0,
    };
    placed(op, Some(gradient))
}

/// A deformer at drag step `k`, 0 at the open.
type Make = fn(f32) -> [Transform; 1];

/// The deformers measured.
const DEFORMERS: [(&str, Make); 4] = [
    ("a bend", bend),
    ("noise of cells 0.05 wide", noise),
    ("a soft 4x4x4 cage", cage),
    ("a curve of 32 under a gradient", curve),
];

fn sphere(n: u32) -> Result<MeshStore> {
    let mut store = MeshStore::from_plain(&scene::quad_sphere(n), NodeConfig::default())?;
    store.take_dirty();
    Ok(store)
}

/// The most bytes of `runs` and the median of their times.
fn summed(runs: &[(u64, f64)]) -> (u64, f64) {
    let bytes = runs.iter().map(|r| r.0).max().unwrap_or_default();
    (bytes, median(runs.iter().map(|r| r.1).collect()))
}

/// At 10M: the weighing, the open's upload, the table's build, an update's block and table, for
/// each deformer on the card; then each one's CPU path, which moves the points.
fn uploads(r: &Renderer, n: u32) -> Result<()> {
    let mut store = sphere(n)?;
    let mut mesh = r.upload_store(&store);
    sync(r)?;
    eprintln!(
        "{} | {n}: {} live points",
        r.gpu().adapter_name(),
        store.vertex_count()
    );
    for (k, &(what, make)) in DEFORMERS.iter().enumerate() {
        card(r, &store, &mut mesh, (what, make), 10 * k as u64)?;
        let (bytes, time) = summed(&builds(make));
        eprintln!("  {what}: its table's build, {bytes} bytes, median {time:.3} ms");
    }
    for (what, make) in DEFORMERS {
        let mut runs = Vec::new();
        let mut moved = Vec::new();
        for k in 1..6 {
            let t = now();
            store.transform(&make(k as f32)[0]);
            moved.push(ms(t));
            let dirty = store.take_dirty();
            sync(r)?;
            let t = now();
            let sent = r.update_store(&mut mesh, &store, &dirty).bytes;
            sync(r)?;
            runs.push((sent, ms(t)));
        }
        let (bytes, time) = summed(&runs);
        eprintln!(
            "  {what}, the CPU path's update: the transform median {:.1} ms, then {bytes} bytes, \
             median {time:.2} ms until the GPU has them",
            median(moved)
        );
    }
    Ok(())
}

/// The table's build alone at five steps of a drag: gs-core preparing the transform, the
/// renderer's first step at each update.
fn builds(make: Make) -> Vec<(u64, f64)> {
    (1..6)
        .map(|k| {
            let ops = make(k as f32);
            let t = now();
            let table = ops[0].prepared().table();
            (16 * table.len() as u64, ms(t))
        })
        .collect()
}

/// One deformer on the card at 10M: the weighing at the open, five opens (each weighed anew,
/// stamps from `stamp`), five updates, then the preview dropped.
fn card(
    r: &Renderer,
    store: &MeshStore,
    mesh: &mut GpuMesh,
    (what, make): (&str, Make),
    stamp: u64,
) -> Result<()> {
    let (mut weigh, mut weights) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        let t = now();
        weights = store.weights(&make(0.0)[0]).ok_or("no plane splits it")?;
        weigh.push(ms(t));
    }
    let mut send = |ops: &[Transform], weighed: u64| -> Result<(u64, f64)> {
        sync(r)?;
        let t = now();
        let preview = Preview {
            ops,
            weights: &weights,
            weighed,
        };
        let sent = r.preview(mesh, Some(preview));
        sync(r)?;
        Ok((sent, ms(t)))
    };
    let opened: Vec<(u64, f64)> = (1..6)
        .map(|k| send(&make(0.0), stamp + k))
        .collect::<Result<_>>()?;
    let updated: Vec<(u64, f64)> = (1..6)
        .map(|k| send(&make(k as f32), stamp + 5))
        .collect::<Result<_>>()?;
    r.preview(mesh, None);
    eprintln!(
        "  {what}: weighing at the open, median {:.1} ms, {} slots",
        median(weigh),
        weights.len()
    );
    for (step, runs) in [("the open's upload", opened), ("an update", updated)] {
        let (bytes, time) = summed(&runs);
        eprintln!("  {what}: {step}, {bytes} bytes, median {time:.2} ms until the GPU has them");
    }
    Ok(())
}

/// A moving frame of `mesh` (each one fresh, as after a camera move): median ms of 5 runs of 10.
fn moving(r: &Renderer, mesh: &GpuMesh, view: View) -> Result<f64> {
    let size = RenderSize {
        width: 1920,
        height: 1080,
        samples: 16,
    };
    let target = r.frame_target(size)?;
    let meshes = [mesh];
    let scene = Scene {
        meshes: &meshes,
        view,
        viewport: [0, 0, size.width, size.height],
    };
    let mut painter = r.ui_painter();
    let mut draw = || {
        target.restart();
        r.draw_frame(&target, &scene, &mut painter, &UiLayer::default())
    };
    for _ in 0..3 {
        draw()?;
    }
    sync(r)?;
    let mut runs = Vec::new();
    for _ in 0..5 {
        let t = now();
        for _ in 0..10 {
            draw()?;
        }
        sync(r)?;
        runs.push(ms(t) / 10.0);
    }
    Ok(median(runs))
}

/// At about 1M: a moving frame without the preview, then with each deformer previewed at step 7
/// of its drag (the bend at 80 degrees).
fn frames(r: &Renderer, m: u32) -> Result<()> {
    let store = sphere(m)?;
    let mut mesh = r.upload_store(&store);
    let camera = Camera::fitted(Preset::Iso, &mesh.fit_points(), 16.0 / 9.0);
    let view = || View::new(camera, Shading::Clay);
    let still = moving(r, &mesh, view())?;
    eprintln!(
        "{m}: {} triangles at 1920x1080, a moving frame: {still:.2} ms still",
        mesh.triangle_count(),
    );
    for (k, (what, make)) in DEFORMERS.iter().enumerate() {
        let ops = make(7.0);
        let weights = store.weights(&ops[0]).ok_or("no plane splits it")?;
        let preview = Preview {
            ops: &ops,
            weights: &weights,
            weighed: k as u64 + 1,
        };
        r.preview(&mut mesh, Some(preview));
        let previewed = moving(r, &mesh, view())?;
        eprintln!(
            "  {what} previewed: {previewed:.2} ms ({:+.1}%)",
            100.0 * (previewed / still - 1.0)
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = AdapterKind::from_name(args.first().map_or("lavapipe", String::as_str))
        .ok_or("adapter is lavapipe, nvidia or radv")?;
    let n: u32 = args.get(1).map_or("1296", String::as_str).parse()?;
    let m: u32 = args.get(2).map_or("408", String::as_str).parse()?;
    let r = Renderer::new(kind)?;
    uploads(&r, n)?;
    frames(&r, m)?;
    eprintln!("peak {:.2} GB", peak::peak_kib() / 1e6);
    Ok(())
}
