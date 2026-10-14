//! The interface (Workbench) over one shell: arranging it for the other hand keeps everything
//! but the look, every control runs a registered command, the pen works the trays, held
//! modifiers show on the cursor, the brush wheel picks in one gesture, the palette finds every
//! control and asks for arguments, and the layout is a plain file; toggles show the session's
//! state and brush sets follow the engine's brushes; a crashed session's work is offered back
//! with commands; the settings window sets the interface scale, which survives a restart; the
//! quick menu opens at the pointer; the status line counts the scene; the History buttons name
//! their step; a slider sets an argument inside a table; one look command sets every view's
//! MatCap and edges, and a drag of a material's slider is one palette step; the parts list
//! selects the active part, which its operations work on, and shows its row count of any number
//! of parts; Isolate's options hide the other parts while the camera moves and toggle Isolate
//! with a click on empty canvas; a slider takes a typed value and the wheel; the view's options
//! and the quick menu fit a short window; the Alpha palette says each alpha's bits on hover and
//! keeps the recent ones; one section opens at a time unless Shift is held, and the trays and
//! the rest of the panels hide and come back as they were; Next and Previous step through the
//! layouts and Restore shows the Workbench as shipped; Ctrl over a
//! button describes it; Open and a first Save ask for what they lack, and a file's path
//! starts in the folder that fits it; a key binds from the keys sheet, a taken one naming what
//! holds it, and one key or every key goes back as built; each default key asks for its catalog
//! row's command, the keys sheet lists them by context, and Tab hides the panels; the brush
//! tray's Smooth and Step set the stroke, and the shelf's Save brush and Load brush ask for the
//! file; a curve copies, pastes, inverts and steps from its graph's tools, every pressure curve
//! switches as size's does and snaps to full near its top, and the Pen set sets the pen's feel.

mod alpha;
mod arrangements;
mod bindings;
mod blender;
mod brush_depth;
mod brush_rows;
mod clock;
mod controls;
mod cursor;
mod curve_by_pen;
mod curves;
mod deform;
mod deformer_handles;
mod descriptions;
mod files;
mod header;
mod history;
mod hold;
mod icons;
mod isolate;
mod keymap;
mod keys_page;
mod library;
mod look;
#[path = "../common/looks.rs"]
mod looks;
mod materials;
mod menus;
mod modifiers;
mod palette;
mod parts;
mod pen;
mod presets;
mod quick;
mod recovery;
#[path = "../common/region.rs"]
mod region;
mod rig;
mod scene;
mod settings;
mod shelf;
mod short;
mod sliders;
mod slots;
mod state;
mod status;
mod switching;
mod symmetry;
mod themes;
mod tray_scroll;
mod trays;
mod wheel;
mod with_pointer;

// Win-checks (2026-10-12): binding from a control, the quick menu's wheel, a slider alone and
// the settings file.
mod bind_click;
mod export;
mod quick_wheel;
mod slider_alone;

// The Live Detail switch dimmed by the brush's automask (decision 0021, section 3).
mod automask_switch;

// The stroke picker in Brush tray > Stroke (decision 0051, section 7).
mod stroke_type;

// Preferences: kept at once in one file, pages, search, Reset and the brush at start.
mod preferences;

// Win-feel (2026-10-12): the mouse hovers as a pen.
mod mouse_hover;

// Tray-fit (2026-10-12): no row runs past its panel's edge.
mod tray_fit;

// Files-win (2026-10-12): an 800-wide window's status line and shelf (Narrow-win).
mod narrow;

// Form-ui (2026-10-12): the palette's form draws each argument by its kind; a choice is never
// typed (decision 0025, rule 13).
mod form;

// P8a (2026-10-12): Brush tray > Paint and the Parts list's paint icon (decision 0039, sections
// 10 and 11).
mod paint;
// The Gizmo's deformers in Brush tray > Brush (decision 0041, amended 2026-10-12, DF7b).
mod gizmo_deformer;

// P8b (2026-10-12): Paint's picker, the swatch's drag pick and C (decision 0039, section 11).
mod picker;

// H2 (2026-10-13): the adjust panel (decision 0060, section 5, as amended).
mod adjust;

// R1 (2026-10-13): the view buttons' options in compact rows (decision 0060 as amended for R1).
mod view_options;
// S1 (2026-10-13): Scene tray > Detail (decision 0060, section 7, as amended).
mod detail;

// H7 (2026-10-13): the header fits a narrow view (decision 0060 as amended for H7).
mod header_fit;

// Views (2026-10-13): the 3D view names the side it shows (review 0009, C5).
mod view_name;

// Files (2026-10-13): a path typed into a file form is the path that runs, every file action
// asks the system's file dialog, the form standing in where none can show, and the window's
// title, close question, dropped files and command line (decision 0016, section 17, amended
// 2026-10-13).
mod dialogs;
mod paths;
mod window;

// Paste (2026-10-14): copy, cut and paste in a text field go through the system clipboard, and
// outside one the same keys stay the keymap's (review 0009, B1).
mod clipboard;

// Refusals (2026-10-14): what a person sees when the engine says no (decision 0025, section 2,
// amended 2026-10-14; review 0009, C1, C2, C4 and C6).
mod refusals;

// Escape (2026-10-14): one Escape closes one thing, the one on top (decision 0025, section 5,
// amended 2026-10-14).
mod escape;

// Rough-forms (2026-10-14): a form's life ends when it is answered (decision 0025, section 2,
// amended 2026-10-14).
mod sent;

// Rough-forms (2026-10-14): how a form asks, and a question's Enter and Replace (review 0009,
// R3, R5, R7, R8, R13 and R15).
mod asking;

// Rough-forms (2026-10-14): File > Quit, Ctrl+Q and Delete (review 0009, R10 and R12).
mod quitting;

// Rough-forms (2026-10-14): Bind a key as one gesture (review 0009, C11).
mod bind_gesture;

// UI-words (2026-10-14): a row that cannot run now is gray with why before it is tapped
// (review 0009, R4; decision 0025, section 6), and a refusal reads as a person's sentence
// wherever it comes from (decision 0025, section 2).
mod gray_rows;
mod refused_words;
// UI-keys (2026-10-14): an arrow, Enter or Tab goes to the one thing in front (decision 0025,
// section 5, amended 2026-10-14).
mod front;
