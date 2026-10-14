//! One-shot geometry algorithms as pure functions, mesh in and mesh out: voxel remesh, splitting,
//! merging and extracting parts, reprojection, repair, decimation, booleans, quad remesh, Auto
//! UV. No GPU.
//!
//! Parallel work goes through `gs-par` only, so every result is the same at any thread count.
//! [`boolean`] also runs manifold-rust's parallel steps, on the caller's rayon pool as `gs-par`
//! does, and gives the same bits at any thread count too (decision 0057, section 4).
//! The one exception to "pure" is [`quad`], which runs a separate program (decision 0038).

pub mod boolean;
pub mod decimate;
mod math;
pub mod operation;
pub mod parts;
pub mod quad;
pub mod uv;
pub mod voxel;
pub mod weld;
