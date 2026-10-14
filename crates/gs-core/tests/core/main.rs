//! gs-core tests. Only the kinds ARCHITECTURE 10.1 admits: determinism across thread counts,
//! round trips, undo and redo exactness against recorded snapshots (a model test), locality,
//! node-split invariants, differential checks against brute-force references, work-counter
//! limits, and self-tests proving the oracle rejects broken data.

#[path = "../common/mod.rs"]
mod common;

mod alphas;
mod automask_checks;
mod bins;
mod brush_behaviors;
mod brush_calls;
mod brush_checks;
mod brush_claims;
mod brush_kit;
mod brush_presets;
mod brush_reference;
mod brush_save;
mod closest;
mod columns;
mod curve_by_pen;
mod curves;
mod cycles;
mod dab_settings;
mod deform;
mod deform_cage;
mod deform_curve;
mod depth_checks;
mod determinism;
mod differential;
mod exact;
mod gestures;
mod groups;
mod hidden;
mod history;
mod kernel_checks;
mod levels;
mod live;
mod live_stroke;
mod locality;
mod mask;
mod mask_grid;
mod oracles;
mod paint;
mod paint_channels;
mod paint_stroke;
mod pinch_pull;
mod plain;
mod recipes;
mod share;
mod sink;
mod smooth_modes;
mod split;
mod stroke_anchored;
mod stroke_curve;
mod stroke_masks;
mod stroke_pipeline;
mod stroke_rect;
mod stroke_spacing;
mod stroke_types;
mod symmetry;
mod upkeep;
mod uv_hints;
mod wrap;
