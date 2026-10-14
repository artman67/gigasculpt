//! Binding keys from the interface (decision 0025, section 5): `ui.keymap` binds a free key,
//! names the holder of a taken one and moves the key once the form's `replace` is confirmed,
//! puts one key or every key back, and writes only the changes to the user's `keymap.toml`; a
//! wheel notch binds as a key does.

use gs_platform::{Input, Modifiers};
use gs_protocol::{ErrorCode, Step};
use gs_ui::{Chord, Keymap};
use serde_json::{Value, json};

use crate::rig::{Ui, folder, held};
use crate::scene::parts;

/// What the window's keymap runs for `key` pressed alone, if anything.
pub(crate) fn runs(ui: &Ui, key: &str) -> Option<String> {
    let chord = Chord::parse(key).unwrap_or_else(|e| panic!("{e}"));
    let studio = ui.host.lock();
    (studio.shell.keymap.lookup(&chord)).map(|b| b.command.clone())
}

/// The palette's form, if one is open: its command and the fields it asks.
fn form(ui: &Ui) -> Option<(String, Vec<String>)> {
    let studio = ui.host.lock();
    let form = studio.shell.palette.form.as_ref()?;
    let fields = form.fields.iter().map(|(name, _)| name.clone()).collect();
    Some((form.step.command.clone(), fields))
}

/// The settings window open at its Keys page (decision 0060, section 7), drawn again as the
/// window's loop draws it: a page taller than its room scrolls from its second frame, which
/// moves the window when the header takes two rows.
fn keys_page(ui: &mut Ui) {
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    ui.wait(0.1);
}

/// The `[[bind]]` entries the keymap file holds.
fn binds(path: &std::path::Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{e}"));
    text.lines().filter(|l| *l == "[[bind]]").count()
}

/// Catalog `interface.keymap`: "binding a key to a command and pressing it runs the command".
/// Settings > Keys' Bind a key… asks what the key runs (then takes the keys pressed:
/// `bind_gesture.rs`); typed, as an agent binds, a free key bound runs its command with its
/// arguments, and a command that does not exist is refused by name.
#[test]
fn a_free_key_bound_runs_its_command() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    keys_page(&mut ui);
    ui.press("settings > Bind a key…");
    let asked = ui.labels("palette");
    assert!(
        asked.iter().any(|l| l == "Pick what the key runs"),
        "{asked:?}"
    );
    ui.ok("ui.palette", json!({ "open": false }));
    // A frame without the form, so its field lets go of the keyboard.
    ui.settle();
    let (code, message) = ui.err(
        "ui.keymap",
        json!({ "bind": "Ctrl+K", "command": "primitive.cub" }),
    );
    assert_eq!(code, ErrorCode::ArgsInvalid);
    assert!(message.contains("primitive.cub"), "{message}");
    assert_eq!(runs(&ui, "Ctrl+K"), None, "a refused binding binds nothing");
    let cube =
        json!({ "bind": "Ctrl+K", "command": "primitive.cube", "args": { "subdivisions": 2 } });
    let info = ui.ok("ui.keymap", cube);
    let change =
        json!([{ "key": "Ctrl+K", "command": "primitive.cube", "args": { "subdivisions": 2 } }]);
    assert_eq!(info["changes"], change);
    ui.tap("K", held("Ctrl"));
    assert_eq!(parts(&mut ui), 2, "Ctrl+K adds a cube");
}

/// Catalog `interface.keymap`: "binding key 4 that another action uses names that action and
/// override moves the key". A key another binding holds is refused naming it and its context,
/// the form asks to replace it, and once that is confirmed the key runs the new command and the
/// old one has no key; a key that starts sequences names them and, replaced, drops them.
#[test]
fn a_taken_key_names_its_holder_and_moves_once_replace_is_confirmed() {
    let mut ui = Ui::new();
    let step = |args: Value| Step {
        command: "ui.keymap".into(),
        args,
    };
    ui.host.lock().shell.queue(step(
        json!({ "bind": "Ctrl+D", "command": "file.quick_save" }),
    ));
    ui.settle();
    // The question says why itself, alone: no problem line beside it (review 0009).
    let asked = ui.labels("palette");
    // By its title, not its id (review 0009).
    let named = "Ctrl+D runs Subdivide in the window";
    assert!(asked.iter().any(|l| l.contains(named)), "{asked:?}");
    let problem = ui.host.lock().shell.status.problem.clone();
    assert_eq!(problem, None, "the question shows alone");
    assert_eq!(
        form(&ui),
        Some(("ui.keymap".into(), vec!["replace".into()]))
    );
    assert_eq!(
        runs(&ui, "Ctrl+D").as_deref(),
        Some("level.subdivide"),
        "nothing moves yet"
    );
    ui.press("palette > Replace");
    assert_eq!(form(&ui), None, "confirmed");
    assert_eq!(
        runs(&ui, "Ctrl+D").as_deref(),
        Some("file.quick_save"),
        "the key moved"
    );
    let subdivide = ui
        .host
        .lock()
        .shell
        .keymap
        .key_for("level.subdivide")
        .map(String::from);
    assert_eq!(subdivide, None, "Subdivide has no key now");
    // B starts the brush sequences: binding it alone names them, and replacing drops them.
    let b = json!({ "bind": "B", "command": "ui.library", "args": { "open": true } });
    let (code, message) = ui.err("ui.keymap", b.clone());
    assert_eq!(code, ErrorCode::Precondition);
    let holder = "B S T runs Draw in the window";
    assert!(message.contains(holder), "{message}");
    let mut replace = b;
    replace["replace"] = json!(true);
    ui.ok("ui.keymap", replace);
    let sequences = ui
        .host
        .lock()
        .shell
        .keymap
        .bindings
        .iter()
        .filter(|b| b.key.starts_with("B "))
        .count();
    assert_eq!(sequences, 0, "no sequence starts with B now");
}

/// Catalog `interface.keymap`: "deleting the custom binding gives the old action its key
/// back; the keymap file round trips; Restore defaults reproduces interface.default-bindings
/// exactly". The file holds only the changes and reads back as the window's keymap; the keys
/// sheet's Restore puts one key back alone, and its Restore defaults puts every key back and
/// leaves the file with no change.
#[test]
fn one_key_or_every_key_is_restored_and_the_file_holds_only_changes() {
    let path = folder("keymap-restore").join("keymap.toml");
    let (keymap, _) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = Ui::new();
    {
        let mut studio = ui.host.lock();
        studio.shell.keymap = keymap;
        studio.shell.keymap_file = Some(path.clone());
    }
    for change in [
        json!({ "bind": "Ctrl+K", "command": "primitive.cube" }),
        json!({ "bind": "Ctrl+D", "command": "file.quick_save", "replace": true }),
        json!({ "bind": "Space", "command": "", "replace": true }),
    ] {
        ui.ok("ui.keymap", change);
    }
    assert_eq!(binds(&path), 3, "only the changes are written");
    let (read, _) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        read,
        ui.host.lock().shell.keymap,
        "the file reads back as the window's keymap"
    );
    assert_eq!(runs(&ui, "Space"), None, "an empty command unbinds");
    keys_page(&mut ui);
    ui.press("settings > Restore Ctrl+D");
    assert_eq!(
        runs(&ui, "Ctrl+D").as_deref(),
        Some("level.subdivide"),
        "its own key back"
    );
    assert_eq!(
        runs(&ui, "Ctrl+K").as_deref(),
        Some("primitive.cube"),
        "the rest stay"
    );
    assert_eq!(runs(&ui, "Space"), None);
    assert_eq!(binds(&path), 2);
    ui.press("settings > Restore defaults");
    assert_eq!(
        ui.host.lock().shell.keymap,
        Keymap::default(),
        "the built-in keys exactly"
    );
    assert_eq!(binds(&path), 0, "and the file holds no change");
    let (read, _) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, Keymap::default());
}

/// Catalog `interface.keymap`: "a wheel notch bound to Size changes Size and not the camera".
/// Alt+WheelUp and Alt+WheelDown bound to Size's step move it a step a notch, as ] and [ do,
/// with the pointer over the 3D view, which does not dolly for them; the wheel alone still
/// dollies it.
#[test]
fn a_wheel_notch_bound_to_size_steps_it_and_not_the_camera() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    for (key, step) in [("Alt+WheelUp", 1), ("Alt+WheelDown", -1)] {
        let bind = json!({ "bind": key, "command": "brush.set", "args": { "size_step": step } });
        ui.ok("ui.keymap", bind);
    }
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let size = |ui: &mut Ui| ui.state()["brush.size_px"].clone();
    let start = size(&mut ui);
    ui.tap("]", Modifiers::default());
    let stepped = size(&mut ui);
    assert_ne!(stepped, start, "] steps Size");
    ui.tap("[", Modifiers::default());
    assert_eq!(size(&mut ui), start, "[ steps it back");
    let camera = |ui: &mut Ui| ui.ok("view.get", json!({ "view": "main" }))["camera"].to_string();
    let seen = camera(&mut ui);
    let notch = |dy| [Input::Wheel { dx: 0.0, dy }];
    ui.input(&notch(1.0), held("Alt"));
    assert_eq!(size(&mut ui), stepped, "a notch up is ]");
    ui.input(&notch(-1.0), held("Alt"));
    assert_eq!(size(&mut ui), start, "a notch down is [");
    assert_eq!(camera(&mut ui), seen, "the camera stays");
    assert!(ui.logged("view.dolly").is_empty(), "no dolly");
    ui.input(&notch(1.0), Modifiers::default());
    assert_eq!(ui.logged("view.dolly").len(), 1, "the wheel alone dollies");
}

/// Whether the palette, asked for `label`, shows a row of that label.
fn palette_shows(ui: &mut Ui, label: &str) -> bool {
    let rows = ui.ok("ui.palette", json!({ "query": label }))["rows"].clone();
    (rows.as_array().into_iter().flatten()).any(|r| r["label"] == label)
}

/// Decision 0062, row 818: Settings' Keys page says what each key does by a title the palette
/// also shows (a control's label, or the title its form's Run reads), never by a command's name;
/// a slider key by its value's title, as the tray's slider. A built-in key whose other arguments
/// its command's title does not say reads a title of its own (`Binding::title`: `[` reads Size
/// down), never its arguments.
#[test]
fn every_key_on_the_keys_sheet_reads_a_title_the_palette_shows() {
    let mut ui = Ui::new();
    // Tall enough for the whole sheet: `ui.describe` lists only what shows.
    ui.host.lock().resize([1800, 8000], 1.0);
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    let labels = ui.labels("settings");
    let from = labels.iter().position(|l| l == "General keys");
    let from = from.unwrap_or_else(|| panic!("no keys in {labels:?}"));
    // The window's foot (Reset page, Reset all, Close) follows the sheet.
    let to = labels.iter().position(|l| l == "Reset page");
    let rows: Vec<String> = (labels[from..to.unwrap_or(labels.len())])
        .iter()
        .filter(|l| !l.ends_with(" keys"))
        .cloned()
        .collect();
    let keys: Vec<String> = (ui.host.lock().shell.keymap.by_context().iter())
        .flat_map(|(_, b)| b.iter().map(|b| format!("{}: ", b.key)))
        .collect();
    let missing: Vec<&String> = (keys.iter())
        .filter(|k| !rows.iter().any(|r| r.starts_with(k.as_str())))
        .collect();
    assert!(missing.is_empty(), "every key shows: {missing:?}");
    let specs = gs_engine::registry();
    let own: Vec<&str> = (Keymap::default().bindings.iter())
        .filter_map(gs_ui::Binding::title)
        .collect();
    let mut raw = Vec::new();
    for row in rows {
        let (_, does) = row.split_once(": ").unwrap_or_else(|| panic!("{row}"));
        let word = |c: char| !c.is_alphanumeric() && c != '.' && c != '_';
        let named = does.split(word).any(|w| specs.iter().any(|s| s.name == w));
        let titled = does.starts_with("hold and move: ") || {
            let does = does.strip_prefix("hold: ").unwrap_or(does);
            let head = does
                .split(" (")
                .next()
                .unwrap_or(does)
                .trim_end_matches('…');
            specs.iter().any(|s| s.title == head)
                || own.contains(&does)
                || palette_shows(&mut ui, head)
        };
        if named || !titled || does.contains(" = ") {
            raw.push(row.clone());
        }
    }
    assert!(raw.is_empty(), "not by a title the palette shows: {raw:#?}");
    for (key, title) in [("[", "Size down"), ("F6", "Materials"), ("C", "Pick color")] {
        let row = format!("{key}: {title}");
        assert!(labels.contains(&row), "{row} in {labels:?}");
    }
}

/// The key the button of `brush` records in its family's window (`ui.describe`).
fn brush_key(ui: &mut Ui, family: &str, brush: &str) -> Value {
    ui.open_family(family);
    let button = ui
        .widgets("family")
        .into_iter()
        .find(|w| w["label"] == brush);
    button.unwrap_or_else(|| panic!("no {brush} in the {family} family"))["key"].clone()
}

/// One answer to which key picks a brush: its button says the key that picks it, a Smooth
/// brush's too, though its button runs `brush.slot` and its key `brush.set`. A key the user
/// binds to it shows over its built-in one, and a brush whose keys are all removed has none.
#[test]
fn a_brush_buttons_key_is_the_one_that_picks_it_in_your_keymap() {
    let mut ui = Ui::new();
    let draw = brush_key(&mut ui, "Build", "Draw");
    assert_eq!(draw, json!("B S T"), "Draw's built-in key");
    let smooth = brush_key(&mut ui, "Smooth", "Smooth");
    assert_eq!(
        smooth,
        json!("B S M"),
        "its key, though its button runs brush.slot"
    );
    let draw = json!({ "bind": "K", "command": "brush.set", "args": { "brush": "Draw" } });
    ui.ok("ui.keymap", draw);
    let bound = brush_key(&mut ui, "Build", "Draw");
    assert_eq!(bound, json!("K"), "the key you bound");
    ui.ok("ui.keymap", json!({ "restore": "K" }));
    ui.ok("ui.keymap", json!({ "bind": "B S T", "command": "" }));
    let removed = brush_key(&mut ui, "Build", "Draw");
    assert_eq!(removed, Value::Null, "no key left");
}

/// Holding a slot's keys strokes with the brush on that slot, so they pick it too: a brush on a
/// slot names them after its key, as the slots are now, and a brush nothing picks says so.
#[test]
fn a_brush_on_a_slot_is_picked_by_holding_its_keys_too() {
    let built_in = Keymap::default();
    let empty = Keymap {
        bindings: Vec::new(),
        changes: Vec::new(),
    };
    let slots = gs_engine::Slots::default();
    let clay = gs_engine::Slots {
        mask: "Clay".into(),
        ..slots.clone()
    };
    for (keymap, slots, brush, says) in [
        (&built_in, &slots, "Smooth", "B S M or Shift held"),
        (&built_in, &slots, "Mask Pen", "B M K or Ctrl held"),
        (&built_in, &slots, "Select Rect", "B S R or Ctrl+Shift held"),
        (&built_in, &slots, "Draw", "B S T"),
        (&built_in, &clay, "Clay", "B C A or Ctrl held"),
        (&built_in, &clay, "Mask Pen", "B M K"),
        (&empty, &slots, "Draw", "No key"),
        (&empty, &slots, "Smooth", "Shift held"),
    ] {
        let text = keymap.brush_keys(brush, slots).text();
        assert_eq!(text, says, "{brush} on {slots:?}");
    }
}
