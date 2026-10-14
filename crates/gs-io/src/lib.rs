//! File import and export formats, one module per format.
//!
//! | Module | Format |
//! |---|---|
//! | [`scene`] | Our scene and Part files (.gss), read and write, with the safe replace. Layout: `docs/decisions/0016-scene-files.md` |
//! | [`obj`] | Wavefront OBJ, read and write |
//! | [`ply`] | Stanford PLY, ASCII and binary, read and write |
//! | [`stl`] | STL, ASCII and binary, read and write |
//! | [`zbp`] | ZBP brush files (.ZBP), read only. Layout notes: `docs/decisions/0004-zbp-format.md` |
//! | [`ztl`] | ZTL tool and ZPR project files (.ZTL, .ZPR), read only. Layout notes: `docs/decisions/0005-ztl-format.md` |
//!
//! The pieces the brush, tool and project files share (preamble, object container, planar arrays,
//! setting records) live in one private module, `container`, and every reader reports problems
//! with one error type, [`ReadError`].
//!
//! The open mesh formats share one model of a mesh file in [`mesh`]: named objects, each a
//! `PlainMesh` with the columns the format can hold.
//!
//! Readers take bytes and return typed values or a typed error. They never panic on bad input
//! and never touch the scene: turning a file into presets, meshes or images is the job of the
//! import commands built on top.

mod container;
mod error;
pub mod mesh;
pub mod obj;
pub mod ply;
pub mod scene;
pub mod stl;
pub mod zbp;
pub mod ztl;

pub use error::ReadError;
