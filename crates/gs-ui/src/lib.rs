//! The only crate that touches egui: the window's interface, its keymap and layout files, and
//! the window itself.
//!
//! - [`Shell`] draws the interface (status line, the shown layout's panels, 3D view and cursor,
//!   brush wheel, command palette) and turns every action into a command for the engine.
//!   Nothing the interface does is out of reach of agents.
//! - [`Directions`] are the layout files: the interface, Workbench (decision 0025), and any
//!   arrangement the user adds of the same shared pieces and controls; `ui.layout` picks one
//!   and the hand it is mirrored for.
//! - [`Studio`] runs the shell in its own egui context and draws it, with the document's parts,
//!   into a frame through gs-render. Screenshots come from the same code.
//! - [`Host`] is the studio as the engine sees it (`pen.input`, `ui.describe`,
//!   `ui.screenshot`, `ui.palette`, `ui.layout`, `ui.wheel`), and its offscreen renderer (`view.snapshot`,
//!   `view.contact_sheet`, `material.select`); [`Offscreen`] renders those for headless
//!   sessions. Both draw the selected look from their [`MatCaps`].
//! - [`app`] is the window: SDL through gs-platform, the surface, and the engine with its
//!   socket server in the same process.

mod adjust;
pub mod app;
mod applies;
mod arrange;
mod blender;
mod canvas;
mod curve;
mod descriptions;
mod direction;
mod draw;
mod family;
mod figure;
mod floats;
mod fuzzy;
mod gizmo;
mod glue;
mod header;
mod held;
mod host;
mod icons;
mod keymap;
mod keys;
mod layout;
mod library;
mod lighting;
mod matcaps;
mod navigate;
mod offscreen;
mod palette;
mod perf;
mod pieces;
mod preferences;
mod pressure;
mod quick;
mod recent;
mod record;
mod recovery;
mod reference;
mod settings;
mod shell;
mod status;
mod studio;
mod text;
mod theme;
mod values;
mod viewport;
mod wheel;

pub use blender::Exchange;
pub use direction::{
    CONTROLS_DIR, Control, Controls, Direction, Directions, Edge, LibraryTab, PanelSpec, RefPlace,
    Set, WIDGETS, to_toml,
};
pub use fuzzy::score;
pub use glue::{MousePen, PenRouter, events, key, layer, split_pen};
pub use held::Held;
pub use host::Host;
pub use icons::{Entry as IconEntry, ICONS, entries as icon_entries, mask as icon_mask};
pub use keymap::{Binding, Chord, Keymap, Press, Pressed};
pub use layout::{Layout, LayoutFile};
pub use matcaps::MatCaps;
pub use navigate::{Navigator, Row};
pub use offscreen::Offscreen;
pub use palette::Palette;
pub use perf::{PenTimes, Perf};
pub use preferences::Preferences;
pub use recovery::{Offer, Recovery};
pub use reference::RefPanel;
pub use shell::Shell;
pub use status::Status;
pub use studio::{Frame, Render, Studio};
pub use theme::{Theme, Themes, contrast, fonts, theme};
