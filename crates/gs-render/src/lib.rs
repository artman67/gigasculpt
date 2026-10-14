//! The only crate that touches wgpu: GPU vertex and index pools, culling, MatCap viewport,
//! offscreen snapshots and contact sheets (ARCHITECTURE section 6).
//!
//! The flow for a snapshot:
//!
//! 1. [`Renderer::new`] opens a headless device on a chosen adapter (lavapipe by default, which
//!    renders byte-identically run to run; see `docs/spikes/golden.md`).
//! 2. A mesh goes to the GPU as one vertex pool and one index pool cut into [`Chunk`]s whose
//!    triangle order is cache-optimized:
//!    - [`Renderer::upload_store`] draws straight from gs-core's node-chunked
//!      [`gs_core::MeshStore`]: each node's vertices are one range of the vertex pool and its
//!      faces one chunk. After an edit [`Renderer::update_store`] feeds the nodes the store
//!      reports dirty through gs-core's node sink, packs them and writes only those
//!      (decision 0012);
//!    - [`Renderer::upload`] copies a borrowed [`RenderMesh`] (a `PlainMesh`) in fixed-size
//!      chunks.
//! 3. [`Renderer::snapshot`] draws the mesh into an RGBA8 image for a [`View`]: an orbit
//!    [`Camera`], a [`Shading`] mode, an optional wireframe, and how the light is shown
//!    ([`Display`], [`Occlusion`], the floor grid). Where the device has
//!    `multi_draw_indexed_indirect_count`, a compute pass first culls chunks outside the view
//!    (and, for meshes drawn with [`Sides::Front`], chunks facing away), and the rest are drawn
//!    with one indirect call; elsewhere every chunk is drawn.
//! 4. [`Renderer::contact_sheet`] tiles several views into one image with their names burned in.
//!
//! The window takes the same path: [`Renderer::for_window`] opens a device that can present to
//! a window, [`Renderer::draw_frame`] draws a [`Scene`] and the interface's [`UiLayer`] into a
//! [`FrameTarget`], and [`WindowSurface::present`] copies the frame to the screen.
//! [`Renderer::read_frame`] reads the same frame back for screenshots.
//!
//! A frame (decision 0019) draws the geometry once, into reversed float depth and a surface
//! target of normals; ambient occlusion (GTAO, in scene units) and the lighting pass work from
//! those, and one display pass shows the light (exposure, view transform, the floor grid,
//! dither), averaging jittered samples while nothing changes. Materials follow one physically
//! based model on the CPU ([`shading`]); a look's MatCap is a deterministic bake of that model
//! on a ball ([`matcap::bake`]), two layers drawn as `albedo x D + S` with occlusion on their
//! environment light.
//!
//! gs-render does not depend on the engine. It reads gs-core's mesh storage only through its
//! public views (nodes, positions, normals, faces, dirty bits).

mod accumulate;
mod color;
mod cull;
mod deform;
mod device;
pub mod display;
mod error;
mod font;
mod frame;
mod grid;
mod image;
mod lit;
mod live;
pub mod matcap;
mod math;
mod mesh;
mod nodes;
mod occlusion;
mod part;
mod passes;
mod peel;
mod pipeline;
mod pool;
pub mod shading;
mod snapshot;
mod split;
mod staging;
mod streams;
mod timing;
mod ui;
mod uniforms;
mod window;

pub use cull::Drawn;
pub use deform::Preview;
pub use device::{AdapterKind, Capabilities, Gpu, WGPU_VERSION};
pub use display::{Display, ViewTransform};
pub use error::RenderError;
pub use frame::{FrameTarget, Scene};
pub use grid::FloorGrid;
/// What [`Renderer::upload_store`] turns into a GPU mesh, for callers without gs-core.
pub use gs_core::MeshStore;
pub use gs_core::camera::{Camera, Fit, Lens, Preset, Projection};
pub use image::Image;
pub use lit::{
    CLAY_LOOK, DEFAULT_ENVIRONMENT, GpuEnvironment, Subsurface, SubsurfaceMode, clay_material,
};
pub use mesh::{
    Bounds, Chunk, FOLD_NOISE, RenderMesh, UploadOptions, encode_normal, vertex_normals,
};
pub use nodes::Upload;
pub use occlusion::{DEFAULT_RADIUS_FRACTION, Occlusion};
pub use peel::{Operand, PEEL_OPERANDS};
pub use pool::{GpuMatCap, GpuMesh, PoolBytes, Sides};
pub use snapshot::{
    BACKGROUND, Background, Light, MAX_SAMPLES, Meshes, RenderSize, Renderer, Shading, View, Wire,
    WireStyle,
};
pub use ui::{UiImage, UiLayer, UiMesh, UiPainter, UiTexture, UiVertex};
pub use window::{SurfaceInfo, SurfaceSource, WindowSurface};
