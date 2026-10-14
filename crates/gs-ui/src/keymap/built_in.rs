//! The built-in keys (decision 0025): the tables by context, as the keys sheet lists them, and
//! the titles of their own that some read there.

use std::sync::LazyLock;

use super::{Binding, Keymap};

/// The built-in keys whose other arguments their command's title does not say, each with a
/// title of its own that the keys sheet reads (decision 0062, row 818); the palette keeps its
/// command's.
pub(super) const TITLES: [(&str, &str); 7] = [
    ("Shift+Space", "Quick menu"),
    ("F6", "Materials"),
    ("Ctrl+L", "Show layout…"),
    ("Ctrl+Shift+Alt+I", "Save layout…"),
    ("[", "Size down"),
    ("]", "Size up"),
    ("C", "Pick color"),
];

/// One built-in key: the key, the command it runs, and the command's arguments written as TOML.
type Row = (&'static str, &'static str, &'static str);

/// The built-in keys by context, in the order the keys sheet lists them, each context a table of
/// the catalog: General (`interface.default-bindings`), View (`-view`), Sculpt (`-sculpt`), Mask
/// (`-mask`) and Geometry (`-geometry`). Every key works in the window outside a text field. A
/// catalog row whose command does not exist yet is reserved, not bound (the catalog's notes
/// list them).
const TABLE: [(&str, &[Row]); 5] = [
    ("General", GENERAL),
    ("View", VIEW),
    ("Sculpt", SCULPT),
    ("Mask", MASK),
    ("Geometry", GEOMETRY),
];

/// [`TABLE`] as bindings.
pub(super) static BUILT_IN: LazyLock<[(&str, Vec<Binding>); 5]> = LazyLock::new(|| {
    TABLE.map(|(context, rows)| {
        let bindings = rows
            .iter()
            .map(|&(key, command, args)| bind(key, command, args));
        (context, bindings.collect())
    })
});

/// The context the keys sheet lists keys under that only the user's changes bind.
pub(super) const YOURS: &str = "Your";

impl Default for Keymap {
    /// The built-in keys, every context's (see [`BUILT_IN`]).
    fn default() -> Self {
        Self {
            bindings: BUILT_IN.iter().flat_map(|(_, b)| b.clone()).collect(),
            changes: Vec::new(),
        }
    }
}

/// General keys (`interface.default-bindings`): the palette on Space (the left thumb; the
/// catalog puts the quick menu there, decision 0025 the palette, provisionally) and the quick
/// menu on Shift+Space (Blender's popup key); undo, redo, save and save as (Ctrl+Shift+S) where
/// every program has them; Ctrl+O opens a scene and Ctrl+Shift+T saves the active part as a Part file, Ctrl+N starts a
/// new scene and Ctrl+Alt+R reverts to the saved one, each through the palette's form as its
/// button does, so a key alone never drops work; Ctrl+Alt+S saves the next numbered file
/// (Blender's incremental save) and 9 a quick save; Tab hides and shows every panel and Comma
/// the Library; F2 held opens the brush wheel at the pen (let go to pick); F9 shows the adjust panel again for the newest step; Ctrl+L shows a layout by name
/// and Ctrl+Shift+Alt+I saves the one shown as a layout file; Ctrl+1 to 3 show the
/// arrangements saved as `arrangement-1` to 3, and Ctrl+Shift+1 to 3 save the one shown there
/// (provisional, decision 0025). Ctrl+Q quits, asking first while there are unsaved changes, as
/// File > Quit and closing the window do (on macOS Cmd+Q too, which the system sends as a close).
/// A key through the form runs `ui.palette` with `form` naming the command and the rest of its
/// arguments after it.
const GENERAL: &[Row] = &[
    ("Space", "ui.palette", ""),
    ("Ctrl+Q", "ui.palette", "quit = true"),
    ("Shift+Space", "ui.quick", "open = true"),
    ("Ctrl+Z", "history.undo", ""),
    ("Ctrl+Shift+Z", "history.redo", ""),
    ("Ctrl+Y", "history.redo", ""),
    ("Ctrl+S", "file.save", ""),
    (
        "Ctrl+Shift+S",
        "ui.palette",
        "form = \"file.save\"\nask = [\"path\"]",
    ),
    (
        "Ctrl+O",
        "ui.palette",
        "form = \"file.open\"\nask = [\"path\"]",
    ),
    (
        "Ctrl+Shift+T",
        "ui.palette",
        "form = \"file.save\"\nargs = { parts = [\"@active\"] }\nask = [\"path\"]",
    ),
    (
        "Ctrl+N",
        "ui.palette",
        "form = \"file.new\"\nask = [\"discard_changes\"]",
    ),
    ("Ctrl+Alt+R", "ui.palette", "form = \"file.revert\""),
    ("Ctrl+Alt+S", "file.save_next", ""),
    ("9", "file.quick_save", ""),
    ("Tab", "ui.layout", "chrome = \"flip\""),
    ("Comma", "ui.library", "open = \"flip\""),
    ("F2", "ui.wheel", "open = \"held\""),
    // F6 opens the Library's Materials tab, only off the 3D view (decision 0025, section 5).
    ("F6", "ui.library", "tab = \"matcaps\"\nopen = \"off-view\""),
    // F9 shows the adjust panel again (Blender's key; decision 0060, section 5, as amended).
    ("F9", "ui.adjust", "open = true"),
    (
        "Ctrl+L",
        "ui.palette",
        "form = \"ui.layout\"\nask = [\"name\"]",
    ),
    (
        "Ctrl+Shift+Alt+I",
        "ui.palette",
        "form = \"ui.layout\"\nask = [\"save\"]",
    ),
    ("Ctrl+1", "ui.layout", "name = \"arrangement-1\""),
    ("Ctrl+Shift+1", "ui.layout", "save = \"arrangement-1\""),
    ("Ctrl+2", "ui.layout", "name = \"arrangement-2\""),
    ("Ctrl+Shift+2", "ui.layout", "save = \"arrangement-2\""),
    ("Ctrl+3", "ui.layout", "name = \"arrangement-3\""),
    ("Ctrl+Shift+3", "ui.layout", "save = \"arrangement-3\""),
];

/// View keys (`interface.default-bindings-view`, decisions 0017 and 0025), most acting on the
/// view under the pointer, a Reference View's picture or else the 3D view: F frames every part,
/// Alt+F the mask, Period the last edit (Blender's Numpad period in sculpt mode), Shift+F
/// switches the wireframe, P switches perspective, V then X, Y or Z turns to an axis view
/// (again: the opposite side), V I the iso view, V H levels the view, V L locks the camera, and
/// Alt+Z and Shift+Alt+Z step back and forward through the camera history. Shift+P shows and
/// hides the floor grid and X turns X symmetry on and off. `view =
/// "pointed"` is [`POINTED`](crate::reference::POINTED): the view under the pointer.
const VIEW: &[Row] = &[
    ("F", "view.frame", "view = \"pointed\""),
    (
        "Alt+F",
        "view.frame",
        "target = \"mask\"\nview = \"pointed\"",
    ),
    (
        "Period",
        "view.frame",
        "target = \"last-edit\"\nview = \"pointed\"",
    ),
    (
        "Shift+F",
        "view.look",
        "flip_wireframe = true\nview = \"pointed\"",
    ),
    (
        "P",
        "view.lens",
        "projection = \"toggle\"\nview = \"pointed\"",
    ),
    ("Shift+P", "view.display", "floor = \"flip\""),
    ("X", "brush.set", "flip_mirror = \"x\""),
    (
        "V X",
        "view.align",
        "to = \"left\"\ntoggle = true\nview = \"pointed\"",
    ),
    (
        "V Y",
        "view.align",
        "to = \"top\"\ntoggle = true\nview = \"pointed\"",
    ),
    (
        "V Z",
        "view.align",
        "to = \"front\"\ntoggle = true\nview = \"pointed\"",
    ),
    ("V I", "view.align", "to = \"iso\"\nview = \"pointed\""),
    ("V H", "view.level", "view = \"pointed\""),
    ("V L", "view.lock", "view = \"pointed\""),
    ("Alt+Z", "view.undo", "view = \"pointed\""),
    ("Shift+Alt+Z", "view.redo", "view = \"pointed\""),
];

/// Sculpting keys for the left hand (`interface.default-bindings-sculpt`): B and two letters
/// pick a brush (decision 0017 as amended, item 11), every shipped brush its own, its letters
/// from its name where they are free; S, U and O held while moving set size,
/// strength and Falloff; [ and ] step the size; L turns stroke smoothing off and on; the
/// backtick goes back to the previous brush; Shift+X swaps the Main and Secondary colors (decision 0039, section 11: X is Mirror
/// X and V starts the view sequences), and C picks the color under the pen into the edited
/// one (`pieces/picker.rs`).
const SCULPT: &[Row] = &[
    ("B S T", "brush.set", "brush = \"Draw\""),
    ("B C B", "brush.set", "brush = \"Clay Build\""),
    ("B S S", "brush.set", "brush = \"Smooth Strong\""),
    ("B M E", "brush.set", "brush = \"Move Elastic\""),
    ("B M P", "brush.set", "brush = \"Smooth Polish\""),
    ("B C C", "brush.set", "brush = \"Clip Curve\""),
    ("B C L", "brush.set", "brush = \"Clay Line\""),
    ("B M V", "brush.set", "brush = \"Move\""),
    ("B S M", "brush.set", "brush = \"Smooth\""),
    ("B I N", "brush.set", "brush = \"Inflate\""),
    ("B P I", "brush.set", "brush = \"Pinch\""),
    ("B D S", "brush.set", "brush = \"Sharp Crease\""),
    ("B S H", "brush.set", "brush = \"Pull Out\""),
    ("B T D", "brush.set", "brush = \"Trim\""),
    ("B H P", "brush.set", "brush = \"Flat Polish\""),
    ("B C A", "brush.set", "brush = \"Clay\""),
    ("B M C", "brush.set", "brush = \"Move Connected\""),
    ("B P N", "brush.set", "brush = \"Pull Out Along Normal\""),
    ("B M K", "brush.set", "brush = \"Mask Pen\""),
    ("B S R", "brush.set", "brush = \"Select Rect\""),
    ("B U D", "brush.set", "brush = \"UV Density\""),
    ("B U H", "brush.set", "brush = \"UV Hint\""),
    ("B P A", "brush.set", "brush = \"Paint\""),
    ("B P B", "brush.set", "brush = \"Paint Blend\""),
    ("S", "brush.set", "size_px = \"drag\""),
    ("U", "brush.set", "strength = \"drag\""),
    ("O", "brush.set", "falloff = \"drag\""),
    ("[", "brush.set", "size_step = -1"),
    ("]", "brush.set", "size_step = 1"),
    (
        "L",
        "brush.set",
        "settings.stroke.smoothing.radius = \"flip\"",
    ),
    ("Backtick", "brush.set", "previous = true"),
    ("Shift+X", "color.set", "swap = true"),
    ("C", "color.set", "pick = \"pointer\""),
];

/// Mask, visibility and face group keys (`interface.default-bindings-mask`, decision 0017 as
/// amended, item 11), on the active part: Ctrl+H shows and hides the mask's darkening,
/// Ctrl+I inverts the mask, Ctrl+A masks everything and Alt+M
/// clears it, Invert and Clear taking hidden points too (`R9`); Ctrl+W groups the masked faces
/// and clears the mask; Ctrl+Shift+X grows the visible area, Ctrl+Shift+C beside it shrinks it
/// (Ctrl+Shift+S is Save scene as, as everywhere), and
/// Ctrl+Shift+A and Ctrl+Shift+Q grow it to whole shells and whole groups.
const MASK: &[Row] = &[
    ("Ctrl+H", "view.display", "mask = \"flip\""),
    ("Ctrl+I", "mask.invert", "scope = \"all\""),
    ("Ctrl+A", "mask.set", "value = 1"),
    ("Alt+M", "mask.set", "value = 0\nscope = \"all\""),
    (
        "Ctrl+W",
        "group.make",
        "maker = { by = \"masked\", clear_mask = true }",
    ),
    ("Ctrl+Shift+X", "visibility.grow", ""),
    ("Ctrl+Shift+C", "visibility.shrink", ""),
    ("Ctrl+Shift+A", "visibility.grow", "to = \"shells\""),
    ("Ctrl+Shift+Q", "visibility.grow", "to = \"groups\""),
];

/// Geometry and part keys (`interface.default-bindings-geometry`, decision 0017 as amended,
/// item 11), on the active part: Ctrl+D subdivides, D and Shift+D step a detail level up and
/// down, Ctrl+R voxel remeshes, Backslash turns Live Detail on and off, Ctrl+Shift+D
/// duplicates the part and Delete deletes it, asking nothing, as Part > Delete > This part
/// does (one undo brings it back). Backspace deletes it too, on every system: a Mac laptop's
/// delete key sends Backspace, and Blender's Industry Compatible keymap binds both. A text field
/// keeps it, as the keymap does not run while one has the keyboard.
const GEOMETRY: &[Row] = &[
    ("Ctrl+D", "level.subdivide", ""),
    ("D", "level.step", "by = 1"),
    ("Shift+D", "level.step", "by = -1"),
    ("Ctrl+R", "mesh.voxel_remesh", ""),
    ("Backslash", "live_detail.set", "toggle = true"),
    ("Ctrl+Shift+D", "part.duplicate", ""),
    ("Delete", "part.delete", ""),
    ("Backspace", "part.delete", ""),
];

/// `key` running `command` with `args`, written as TOML.
fn bind(key: &str, command: &str, args: &str) -> Binding {
    Binding {
        key: key.into(),
        command: command.into(),
        args: toml::from_str(args).unwrap_or_default(),
    }
}
