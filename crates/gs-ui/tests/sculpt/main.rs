//! Sculpting in the window: the pen's strokes and the sculpting keys, through the real window
//! host and engine, as the window drives them. Registry properties and differential checks
//! against the command path (ARCHITECTURE 10.1).

mod background;
mod curve;
mod deform_preview;
mod detail;
mod drag_rect;
mod eraser;
mod framing;
mod gizmo;
mod gizmo_words;
mod held;
mod keys;
mod line;
mod live_boolean;
#[path = "../common/looks.rs"]
mod looks;
mod mirror;
mod navigation;
mod off_model;
mod pen;
mod pressure;
#[path = "../common/region.rs"]
mod region;
mod rig;
mod ring;
mod ring_navigating;
mod symmetry;
mod view;

mod voxel_mode;
mod words_wrap;
