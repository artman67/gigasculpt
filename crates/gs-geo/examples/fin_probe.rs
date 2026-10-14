//! Decision 0057's repair 5 (step LB3) at scale: a closed quad sphere of 1,002,252 triangles with
//! one fin (an extra triangle on one of its edges, so that edge has three faces and the operand is
//! read by the generalized winding number), repaired alone (`voxel::repair_operand`) at resolution
//! 256. Prints the route, whether it was turned, the faces out, the volume, the work and the wall
//! time at 24 threads (median of 3); then the same for the finned sphere inside out and for the
//! sphere without its fin; then the process's peak memory.
//!
//! ```text
//! gate/heavy.sh cargo run --release -p gs-geo --example fin_probe -- [cells per edge] [resolution]
//! ```
//!
//! `289 256` (the default) is the question's case.

use std::io::Write;
use std::time::Instant;

use gs_geo::voxel::{CellSize, Settings, repair_operand};
use gs_protocol::{NO_VERTEX, PlainMesh};

#[expect(clippy::disallowed_methods, reason = "a timing tool reads the clock")]
fn now() -> Instant {
    Instant::now()
}

/// The process's peak memory, as `/proc/self/status` gives it.
fn peak() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let line = status.lines().find_map(|l| l.strip_prefix("VmHWM:"));
    line.map_or("unknown".into(), |l| l.trim().to_string())
}

fn say(line: &str) {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}

/// gs-testkit's cube of `n` by `n` quads a side, pushed out onto the unit sphere.
fn sphere(n: u32) -> PlainMesh {
    let mut mesh = gs_testkit::fixtures::cube(n);
    for p in &mut mesh.positions {
        let l = p.iter().map(|c| f64::from(*c) * f64::from(*c)).sum::<f64>();
        *p = p.map(|c| (f64::from(c) / l.sqrt()) as f32);
    }
    mesh
}

/// `mesh` with a fin: a triangle on its first face's first edge, standing out from it.
fn fin(mut mesh: PlainMesh) -> PlainMesh {
    let [a, b] = [0, 1].map(|k| mesh.positions[mesh.faces[0][k] as usize]);
    let tip = std::array::from_fn(|i| 1.05 * (a[i] + b[i]) / 2.0);
    mesh.positions.push(tip);
    let new = (mesh.positions.len() - 1) as u32;
    mesh.faces
        .push([mesh.faces[0][0], mesh.faces[0][1], new, NO_VERTEX]);
    mesh
}

/// `mesh` with every face turned over.
fn flipped(mut mesh: PlainMesh) -> PlainMesh {
    for f in &mut mesh.faces {
        if f[3] == NO_VERTEX {
            f.swap(1, 2);
        } else {
            f.swap(1, 3);
        }
    }
    mesh
}

fn run(what: &str, mesh: &PlainMesh, resolution: u32) {
    let settings = Settings {
        cell: CellSize::Resolution(resolution),
        project: false,
        blur: 0.0,
    };
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..3 {
        let t = now();
        let out = gs_par::with_threads(24, || repair_operand(mesh, &settings));
        times.push(t.elapsed().as_secs_f64());
        last = Some(out.unwrap_or_else(|e| panic!("{e}")));
    }
    times.sort_by(f64::total_cmp);
    let Some(out) = last else { return };
    let volume = gs_testkit::measure::stats(&out.remeshed.mesh).volume;
    say(&format!(
        "{what}: {} faces in, {:?}, turned {}, {} quads out, volume {volume:.6}, \
         24 threads {:.3} s (runs {times:.3?})\n  {:?}",
        mesh.faces.len(),
        out.remeshed.sign,
        out.turned,
        out.remeshed.mesh.faces.len(),
        times[1],
        out.remeshed.work,
    ));
}

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<u32>().ok());
    let n = args.next().flatten().unwrap_or(289);
    let resolution = args.next().flatten().unwrap_or(256);
    let closed = sphere(n);
    let finned = fin(closed.clone());
    run("sphere with a fin", &finned, resolution);
    run(
        "sphere with a fin, inside out",
        &flipped(finned),
        resolution,
    );
    run("sphere", &closed, resolution);
    say(&format!("peak memory {}", peak()));
}
