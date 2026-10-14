//! gs-engine tests. Only the kinds ARCHITECTURE 10.1 admits: registry properties checked on
//! every command (including the session policy table), a model test of the undo history with a
//! log replay, oracle self-tests for the hasher and the validator, differential checks against
//! slow independent references, and socket round trips. Everything goes through commands, the
//! way clients use the engine, and most of it as an agent, the way most clients connect.

mod active;
mod atomic;
mod automask_detail;
mod boolean;
mod boolean_props;
mod canvas;
mod catalog;
mod claims;
mod colorize;
mod decimate;
mod deform;
mod deform_levels;
mod deform_preview;
mod deform_session;
mod deformer_handles;
mod determinism;
mod effects;
mod exchange;
mod fill;
mod flip;
mod frame;
mod freeze;
mod gizmo;
mod groups;
mod history;
mod image_diff;
mod levels;
mod limits;
mod live_boolean;
mod live_issues;
mod log_tail;
mod mask_grid;
mod mask_make;
mod masked;
mod materials;
mod mesh_diff;
mod model;
mod navigation;
mod numbering;
mod oracles;
mod paint;
mod parallel_transforms;
mod parts;
mod pick_color;
mod placed;
mod policy;
mod primitive_levels;
mod primitives;
mod project_detail;
mod quad_remesh;
mod queries;
mod reconstruct;
mod reference;
mod reflections;
mod replay;
mod rogue;
mod roles;
mod sample;
mod smooth;
mod socket;
mod support;
mod unfreeze_columns;
mod uv_hints;
mod views;
mod visibility;
mod voxel_remesh;
mod wire;

mod voxel_mode;
