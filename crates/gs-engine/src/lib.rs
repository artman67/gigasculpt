//! Command trait, registry, executor, document, history, log and socket server.
//!
//! Command definitions live in one folder per domain, next to the code they call:
//! `primitive/`, `part/`, `mesh/`, `scene/`, `history/`, `session/`, `catalog/`, `debug/`, `ui/`,
//! `view/`, `display/`, `material/`, `file/`, `sculpt/`, `alpha/`, `pick/`, `mask/`,
//! `visibility/`, `group/`, `transform/`, `gizmo/`.
//! Adding a command means one new file with a struct, an `impl Command` and one
//! `register_command!` line. Nothing else changes.
//!
//! Each part is a gs-core `MeshStore` (layout B, decision 0001). Commands only see the `cx.doc`
//! write guards: the first write to a part in a call opens a mesh-store history step, so undo
//! and redo swap node columns and no command writes undo code (see `document`).

#![expect(
    clippy::result_large_err,
    reason = "CommandError is the wire error type; it is built once per failed call, so boxing it in every command signature would cost clarity for no speed"
)]

pub mod cli;
mod clock;
mod command;
mod ctx;
mod document;
mod effect;
mod engine;
pub mod errors;
mod example;
mod job;
pub mod library;
mod log;
mod mask;
mod math;
mod pinning;
mod registry;
mod repro;
pub mod server;

mod alpha;
mod boolean;
mod catalog;
mod debug;
mod display;
mod exchange;
mod file;
mod gizmo;
mod group;
mod history;
mod image;
mod level;
mod material;
mod mesh;
mod paint;
mod part;
mod pick;
mod primitive;
mod scene;
mod sculpt;
mod session;
mod symmetry;
mod transform;
mod ui;
mod view;
mod visibility;

pub use boolean::{LiveDraw, LiveMesh, LivePeel, PeelOperand, Stand, Tint};
pub use clock::{Clock, ManualClock};
pub use command::{Command, CommandSpec, PartArg, Unavailable, ViewArg, registry};
pub use ctx::{Ctx, Origin};
pub use display::{
    AutoWord, DisplaySettings, Environment, EnvironmentInfo, KeyLight, LIGHTS, LightSetting,
    MaskTint, Occlusion, OcclusionRadius, Subsurface, SubsurfaceMode, Transform,
};
pub use display::{Background, FloorGrid, FloorPreset, WireStyle};
pub use document::{DocTxn, Numbering, PartMut, VertexOrder};
pub use engine::{Engine, EngineConfig};
pub use example::Example;
pub use file::{AutosaveSettings, FreeSpace, StateFolder};
pub use gs_core::rng::CommandRng;
pub use gs_protocol::Request;
#[doc(hidden)]
pub use inventory;
pub use level::write_level_uvs;
pub use log::StandardLog;
pub use material::{
    MaterialInfo, MaterialPalette, MaterialSet, Materials, PaletteEntry, RefusedMaterial,
};
pub use sculpt::{
    Blocked, BrushInfo, BrushState, DetailMode, DetailSettings, DetailState, Increments, PenDown,
    PenSettings, PenStroke, Refusal, Slot, Slots, brushes,
};
pub use session::{ReferenceState, SessionState, StateValues};
pub use ui::{
    AdjustInfo, BarrelClick, BarrelTwo, CallReport, DeformSettings, Eraser, GizmoSettings, Hand,
    HeldCurve, Hop, KeyBinding, KeymapInfo, LayoutInfo, LayoutPart, LibraryInfo, MainView,
    MenuInfo, MenuOpen, Navigation, NoRenderer, NoWindow, PaletteInfo, PaletteRow, PanelInfo,
    PartReport, PenReads, PenTaken, PerfInfo, PreferencesInfo, QuickInfo, RenderHost, ResetScope,
    SETTINGS_VERSION, Screenshot, SectionInfo, SettingInfo, SettingsInfo, ShownReference, Shows,
    Surface, Timing, UiAdjust, UiCurve, UiDeform, UiDescription, UiGizmo, UiKeymap, UiLayout,
    UiLibrary, UiMenu, UiNavigation, UiPalette, UiQuick, UiSettings, UiWheel, ViewportInfo,
    WheelInfo, WidgetInfo, WindowHost, WindowSettings,
};
pub use view::{
    AGENT_VIEWS, Alignment, ExactCamera, Labelled, Link, NamedView, OrbitArg, PivotRule, RefLook,
    ReferenceInfo, RenderRequest, Shading, Source, ViewInfo, ViewSpec, Views,
};
