//! Layout files: the interface (Workbench, decision 0025) is a plain TOML file arranging the
//! pieces, and the controls it draws from are shared, a file per tray. A control is a label and
//! the command it runs, exactly what a key binding or the palette runs, so the interface can
//! reach nothing an agent cannot.
//!
//! ```toml
//! # layouts/workbench.toml
//! title = "Workbench"
//! order = 1
//! wheel = { show = "brushes", radius = 320 }
//!
//! [[panel]]
//! id = "shelf"
//! title = "Brushes"
//! edge = "top"
//! size = 64
//! show = ["brushes", "recall"]
//! ```
//!
//! ```toml
//! [header]
//! menus = ["file"]
//! show = ["view", "look"]
//! options = [
//!   { id = "look", label = "Look", show = ["shading", "looks", "tone"] },
//! ]
//! ```
//!
//! A panel shows pieces in order: a set of controls by name (`symmetry`), one group of a set
//! (`set/Group`), or a shared widget ([`WIDGETS`]), such as the brushes the engine lists
//! (`brushes`, or one family: `brushes/Mask`). Along a shelf the pieces run one after another;
//! in a tray (a left or right panel) each is a section under its title, folding shut by it. The
//! 3D view's header (`header`, decision 0060) shows a title for each of its `menus`, opening that
//! piece under it as a menu or a dropdown panel, and `show`'s pieces at the pen end, where an
//! entry may name one of its `options` buttons, whose options show that entry's pieces. The settings
//! window (`ui.settings`) shows the pieces in `settings`, and the quick menu at the pointer
//! (`ui.quick`) those in `quick`, and the Library (`ui.library`) a tab for each entry of
//! `library`, its piece's controls as tiles. The `brush` widget shows the active tool's settings: the
//! pieces of the first `[[tool]]` whose `when` state value is true, or of one without `when`
//! (`{ id = "brush", show = ["size", "strength", "falloff"] }`). A control that sets
//! something names the `session.state` value it shows (decision 0013). The shipped files (`assets/layouts/`) are
//! built in; a file in the user's `layouts` folder replaces the shipped one of its name or adds
//! an arrangement. The controls are a file per tray in `controls/` (`controls/brush-tray.toml`),
//! and a user file in `layouts/controls/` named like one tray's file replaces that tray's
//! controls; any other name there is refused. A broken file is refused with the line and key
//! that are wrong, and the one it would have replaced stays (decision 0011).
//!
//! The file's types are in `spec`, the controls in `controls`, and reading the files (the
//! shipped and the user's, each checked as it is added) in `load`.

mod controls;
mod load;
mod spec;

pub use controls::{Control, Controls, Set, put};
pub use load::{CONTROLS_DIR, Directions, is_shipped, parse, to_toml};
pub use spec::{
    Direction, Edge, HeaderOptions, HeaderSpec, LibraryTab, PaletteSpec, PanelSpec, Place,
    RefPlace, ReferenceSpec,
};

/// The shared widgets a panel can show besides sets of controls, by name and title: the
/// brushes (from `brush.list`, by family), a button per family (`families`, opening the rest of
/// the family at the pointer: `family.rs`), the active tool's settings (`brush`, from the
/// layout's tools, titled Basics), the brush's size, strength and Softness sliders, the size
/// titled Reach (a soft gizmo drag's), the Pressure popover with the brush's pressure curves
/// (`pressure`), Lock, Sticky and Follow mask for the gizmo shown (`pose`, titled Pose), the tip
/// (`curve`: its picture, Softness, Edit shape… and Exact), the
/// MatCaps, the active material's sliders (`material`), the parts, a button per part to show it
/// alone (Isolate), the keys cheat sheet, the shipped alphas, the Alpha palette (`alpha`: the
/// brush's alpha and the recent ones, then the `alpha` set), the
/// Deform section (`deform`) and the layouts.
pub const WIDGETS: [(&str, &str); 21] = [
    ("brushes", "Brushes"),
    ("families", "Brushes"),
    ("brush", "Basics"),
    ("size", "Size"),
    ("strength", "Strength"),
    ("falloff", "Softness"),
    ("reach", "Reach"),
    ("pressure", "Pressure"),
    ("pose", "Pose"),
    ("curve", "Tip"),
    ("looks", "Looks"),
    ("material", "Material"),
    ("parts", "Parts"),
    ("isolate", "Isolate"),
    ("keys", "Keys"),
    ("alphas", "Alphas"),
    ("alpha", "Alpha"),
    ("deform", "Deform"),
    ("layouts", "Layouts"),
    ("favorites", "Favorites"),
    ("themes", "Themes"),
];

/// A piece's name and the group it picks, if any: `brushes/Mask` is `brushes` and `Mask`.
pub fn split(piece: &str) -> (&str, Option<&str>) {
    match piece.split_once('/') {
        Some((name, group)) => (name, Some(group)),
        None => (piece, None),
    }
}

/// The title a piece shows above it: its set's, or its widget's.
pub fn title<'a>(controls: &'a Controls, piece: &'a str) -> &'a str {
    let widget = WIDGETS.iter().find(|w| w.0 == split(piece).0).map(|w| w.1);
    let set = controls.piece(piece).map(|(set, _)| set.title.as_str());
    set.or(widget).unwrap_or(piece)
}
