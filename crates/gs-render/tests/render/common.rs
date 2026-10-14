//! Shared helpers: one renderer per adapter for the whole run, framing, pixel counting, FLIP.

use gs_protocol::PlainMesh;
use gs_render::matcap::Recipe;
use gs_render::{
    AdapterKind, Camera, Fit, GpuMesh, Image, Lens, Meshes, Preset, Projection, RenderSize,
    Renderer, Shading, UploadOptions, View, Wire,
};
use std::path::PathBuf;
use std::sync::OnceLock;

pub use gs_render::BACKGROUND;

/// Opens a renderer, turning a panic into an error so that a broken device fails every test
/// at once instead of being opened again by each one.
fn open(kind: AdapterKind) -> Result<Renderer, String> {
    let opened = std::panic::catch_unwind(|| Renderer::new(kind));
    opened
        .map_err(|_| format!("{kind:?} panicked while opening"))?
        .map_err(|e| e.to_string())
}

pub fn lavapipe() -> &'static Renderer {
    static R: OnceLock<Result<Renderer, String>> = OnceLock::new();
    let opened = R.get_or_init(|| open(AdapterKind::Lavapipe));
    opened
        .as_ref()
        .expect("lavapipe is installed (mesa vulkan-swrast)")
}

type Opened = Vec<(AdapterKind, Renderer)>;

/// Renderers for the real GPUs on this machine (RTX 5090 and Radeon iGPU when present).
pub fn hardware() -> &'static [(AdapterKind, Renderer)] {
    static R: OnceLock<Result<Opened, String>> = OnceLock::new();
    let opened = R.get_or_init(|| {
        let kinds = AdapterKind::available().into_iter();
        kinds
            .filter(|&k| k != AdapterKind::Lavapipe)
            .map(|k| Ok((k, open(k)?)))
            .collect()
    });
    opened.as_ref().expect("every available adapter opens")
}

/// Every adapter on this machine, lavapipe first. A machine with none fails here, so a test
/// that loops over them cannot pass by asserting nothing (research 27 R14).
pub fn adapters() -> Vec<(AdapterKind, &'static Renderer)> {
    let soft = AdapterKind::available().contains(&AdapterKind::Lavapipe);
    let all: Vec<_> = soft
        .then(|| (AdapterKind::Lavapipe, lavapipe()))
        .into_iter()
        .chain(hardware().iter().map(|(k, r)| (*k, r)))
        .collect();
    assert!(
        !all.is_empty(),
        "no Vulkan adapter found: install lavapipe (mesa vulkan-swrast)"
    );
    all
}

pub fn upload(r: &Renderer, mesh: &PlainMesh) -> GpuMesh {
    r.upload(mesh, UploadOptions::default())
        .expect("valid mesh uploads")
}

/// A preset view fitted to the mesh for a square image.
pub fn fitted<'a>(mesh: &GpuMesh, preset: Preset, shading: Shading<'a>, wire: Wire) -> View<'a> {
    View {
        wire,
        ..View::new(Camera::fitted(preset, &mesh.fit_points(), 1.0), shading)
    }
}

/// Front orthographic view of the origin showing -1.1..1.1 both ways: in a 128-pixel image
/// x = 0 falls between columns 63 and 64 and each pixel is 2.2 / 128 wide; a unit sphere's
/// center is the image center.
pub fn front_ortho(shading: Shading<'_>) -> View<'_> {
    let camera = Camera {
        distance: 1.1,
        lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
        projection: Projection::Orthographic,
        ..Camera::preset(Preset::Front)
    };
    View::new(camera, shading)
}

/// Sum of the color channels.
pub fn luma(p: [u8; 4]) -> u32 {
    u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
}

pub fn snap<'m>(r: &Renderer, meshes: impl Into<Meshes<'m>>, view: &View, size: u32) -> Image {
    r.snapshot(meshes, view, RenderSize::square(size))
        .expect("snapshot renders")
}

/// Pixels a triangle covered.
pub fn covered(image: &Image) -> usize {
    image.rgba.chunks(4).filter(|p| *p != BACKGROUND).count()
}

/// Largest per-channel difference and the number of pixels that differ at all.
pub fn diff(a: &Image, b: &Image) -> (u8, usize) {
    assert_eq!(
        (a.width, a.height),
        (b.width, b.height),
        "images to compare have the same size"
    );
    let (mut max, mut count) = (0u8, 0usize);
    for (p, q) in a.rgba.chunks(4).zip(b.rgba.chunks(4)) {
        let d = (0..4).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        max = max.max(d);
        count += usize::from(d > 0);
    }
    (max, count)
}

/// NVIDIA FLIP between two images: worst pixel and pixels over 0.05 (the 10.3 rule's numbers).
pub fn flip(expected: &Image, actual: &Image) -> (f32, usize) {
    // Both images' RGB bytes in one pass, alpha dropped.
    let (mut e, mut a) = (Vec::new(), Vec::new());
    for (p, q) in expected.rgba.chunks(4).zip(actual.rgba.chunks(4)) {
        e.extend_from_slice(&p[..3]);
        a.extend_from_slice(&q[..3]);
    }
    let err = nv_flip::flip(
        nv_flip::FlipImageRgb8::with_data(expected.width, expected.height, &e),
        nv_flip::FlipImageRgb8::with_data(actual.width, actual.height, &a),
        nv_flip::DEFAULT_PIXELS_PER_DEGREE,
    )
    .to_vec();
    (
        err.iter().copied().fold(0.0, f32::max),
        err.iter().filter(|&&e| e > 0.05).count(),
    )
}

pub fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Every shipped recipe, by file stem.
pub fn shipped_recipes() -> Vec<(String, Recipe)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(repo_path("assets/matcaps"))
        .expect("assets/matcaps exists")
        .map(|e| e.expect("readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("recipe readable");
            let stem = p
                .file_stem()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            (
                stem,
                Recipe::parse(&text).unwrap_or_else(|e| panic!("{} parses: {e}", p.display())),
            )
        })
        .collect()
}

pub fn shipped(name: &str) -> Recipe {
    shipped_recipes()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| r)
        .expect("shipped recipe")
}
