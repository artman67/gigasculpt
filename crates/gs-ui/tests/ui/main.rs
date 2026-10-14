//! gs-ui tests: the interface driven headlessly with `egui_kittest` against a real engine, the
//! keymap and layout files, fuzzy matching, the glue to egui, and the studio's frames on
//! lavapipe.

mod app;
mod asking;
mod background;
mod busy;
mod canvas;
mod display;
mod face_groups;
mod files;
mod fuzzy;
mod geometry;
mod glue;
mod heatmap;
mod icons;
mod looks;
mod materials;
mod navigate;
mod outline;
mod palette;
mod perf;
#[path = "../common/region.rs"]
mod region;
mod rig;
mod sliders;
mod studio;
mod theme;
