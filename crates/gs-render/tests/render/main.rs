//! gs-render tests. One binary, so the device opens once per run (docs/spikes/golden.md).
//! GPU tests render on lavapipe; tests that compare against real GPUs run on whichever of the
//! RTX 5090 and the Radeon iGPU are installed.
#![expect(
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows expect in tests"
)]

mod accumulate;
mod camera;
mod common;
mod cull;
mod depth;
mod determinism;
mod device;
mod display;
mod environments;
mod grid;
mod library;
mod lit;
mod live;
mod matcap;
mod mesh;
mod occlusion;
mod outline;
mod overlay;
mod paint;
mod peel;
mod placement;
mod preview;
mod scene;
mod shading;
mod sheet;
mod store;
mod stroke_classes;
mod subsurface;
mod wireframe;
mod zebra;
