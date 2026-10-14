//! Shared helpers: one renderer per adapter for the whole run, scratch folders on disk, and
//! the golden spike's dented sphere.

use std::path::PathBuf;
use std::sync::OnceLock;

use gs_protocol::PlainMesh;
use gs_render::{AdapterKind, Image, Preset, Renderer, Sides};
use gs_testkit::Scene;
use gs_testkit::fixtures;
use gs_testkit::golden::{Goldens, Look, Show, render};

fn open(kind: AdapterKind) -> Result<Renderer, String> {
    Renderer::new(kind).map_err(|e| e.to_string())
}

pub fn lavapipe() -> &'static Renderer {
    static R: OnceLock<Result<Renderer, String>> = OnceLock::new();
    let opened = R.get_or_init(|| open(AdapterKind::Lavapipe));
    opened.as_ref().expect("lavapipe is installed")
}

/// Renderers for the real GPUs on this machine, at the references' occlusion quality: these
/// checks measure noise between drivers, and the integrated GPU's half-resolution occlusion
/// (decision 0019) is a quality step, not noise (on the cube's creases it is 22 levels off the
/// full one). gs-render's occlusion tests check that step.
pub fn hardware() -> &'static [(AdapterKind, Renderer)] {
    static R: OnceLock<Result<Vec<(AdapterKind, Renderer)>, String>> = OnceLock::new();
    let opened = R.get_or_init(|| {
        (AdapterKind::available().into_iter())
            .filter(|&k| k != AdapterKind::Lavapipe)
            .map(|k| Ok((k, open(k)?.with_full_occlusion())))
            .collect()
    });
    opened.as_ref().expect("every available adapter opens")
}

/// An empty folder of its own under the test target's scratch space (on disk).
pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("golden")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("old scratch removed");
    }
    std::fs::create_dir_all(&dir).expect("scratch created");
    dir
}

/// Golden folders in scratch space with the given scene files.
pub fn goldens(name: &str, scenes: &[(&str, &str)]) -> Goldens {
    let dir = scratch(name);
    let g = Goldens {
        root: dir.join("goldens"),
        work: dir.join("work"),
        matcaps: Goldens::repo().matcaps,
    };
    std::fs::create_dir_all(g.scenes()).expect("scenes folder");
    for (scene, text) in scenes {
        write_scene(&g, scene, text);
    }
    g
}

pub fn write_scene(g: &Goldens, scene: &str, text: &str) {
    std::fs::write(g.scenes().join(format!("{scene}.toml")), text).expect("scene written");
}

/// A scene file with one command step.
pub fn scene_file(looks: &str, command: &str, args: &str) -> String {
    format!(
        "about = \"test\"\nlooks = [{looks}]\nclosed = true\n\n[[step]]\ncommand = \"{command}\"\nargs = {args}\n"
    )
}

pub fn unit(p: [f32; 3]) -> [f32; 3] {
    let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    p.map(|c| c / l)
}

/// The golden spike's stroke: 8 inward dabs along an arc on the upper front of a unit sphere,
/// `depth` deep (0.12; 0.1194 is a 0.5% weaker stroke).
pub fn dented(depth: f32) -> PlainMesh {
    let mut mesh = fixtures::icosphere(6);
    let (a, b) = (unit([-0.2, 0.75, 0.65]), unit([0.55, 0.45, 0.7]));
    for k in 0..8 {
        let t = k as f32 / 7.0;
        let c = unit(std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t));
        for p in &mut mesh.positions {
            let d: [f32; 3] = std::array::from_fn(|i| p[i] - c[i]);
            let u = 1.0 - (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) / (0.35 * 0.35);
            if u > 0.0 {
                let w = u * u * (3.0 - 2.0 * u) * depth * 0.25;
                let n = unit(*p);
                *p = std::array::from_fn(|i| p[i] - n[i] * w);
            }
        }
    }
    mesh
}

/// The dented sphere as a one-part scene, rendered on `r` from the front-right.
pub fn dented_picture(r: &Renderer, depth: f32, look: &Look) -> Image {
    let parts: Scene = [("sphere".to_string(), dented(depth))].into();
    let how = (Preset::Iso, Sides::Front, Show::default());
    render(r, &parts, how, look, &Goldens::repo().matcaps).expect("renders")
}
