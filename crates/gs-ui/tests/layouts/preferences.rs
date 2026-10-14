//! Preferences (decision 0052): the window keeps every setting its settings window shows in
//! `preferences.toml`, written by the next frame after any change, with a version and every key
//! it does not know kept; at start it runs each setting's own command with the file's value, so
//! the pen's feel, the view moves and the brush at start survive a restart. The window's pages,
//! search and Reset (asked first) are driven by `ui.settings`, as its widgets drive them.
//! Voxel Remesh hole closing is one of them, on the Geometry page; `layout.toml` no longer
//! holds what moved, and an old one that does hands it over once, at launch before any frame;
//! the Files page names the pinned brush (decision 0052 as amended 2026-10-12). Each window that
//! starts from files starts as the app's does (`Ui::launch`).

use std::path::{Path, PathBuf};

use gs_platform::{Input, Modifiers};
use gs_protocol::ErrorCode;
use serde_json::{Value, json};

use crate::rig::{Ui, folder};

/// A window started from the files in `dir` as the `gigasculpt` window starts (`Ui::launch`,
/// its state folder `dir/state`), after its first frames have heard the session and run what
/// `preferences.toml` asked for.
pub fn started(dir: &Path) -> Ui {
    let mut ui = Ui::launch(dir, &dir.join("state"));
    ui.settle();
    ui
}

/// A window started from the folder of the preferences file `path` (see [`started`]).
fn with_preferences(path: &Path) -> Ui {
    started(path.parent().unwrap_or(path))
}

/// The preferences file in a new folder `name` inside `target/`.
fn file(name: &str) -> PathBuf {
    folder(name).join("preferences.toml")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A number as the `f32` it stands for.
fn near(v: &Value, want: f32) -> bool {
    v.as_f64().is_some_and(|x| (x as f32 - want).abs() < 1e-5)
}

#[test]
fn a_change_is_written_at_once_and_keys_the_window_does_not_know_are_kept() {
    let path = file("prefs-written");
    let old =
        "version = 1\nfuture = \"kept\"\n\n[pen]\nmystery = 3\n\n[navigation]\norbit_speed = 1.5\n";
    std::fs::write(&path, old).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = with_preferences(&path);
    assert!(
        near(&ui.state()["navigation.orbit_speed"], 1.5),
        "read at start"
    );
    // Every setting the file did not hold is written from the session's value.
    let text = read(&path);
    assert!(text.contains("sticky_keys = true"), "{text}");
    assert!(text.contains("lazy_pressure = 0.0"), "{text}");

    ui.ok("ui.settings", json!({ "center_snap": false }));
    ui.settle();
    assert!(
        read(&path).contains("center_snap = false"),
        "{}",
        read(&path)
    );
    ui.ok("pen.set", json!({ "lazy_pressure": 0.3 }));
    ui.settle();
    let text = read(&path);
    assert!(
        text.contains("lazy_pressure = 0.3\n"),
        "f32 as written: {text}"
    );
    for kept in ["version = 1", "future = \"kept\"", "mystery = 3"] {
        assert!(text.contains(kept), "{kept} kept: {text}");
    }

    // A file from a newer version is read but never written over.
    let newer = file("prefs-newer");
    let from_later = "version = 2\n\n[navigation]\norbit_speed = 0.5\n";
    std::fs::write(&newer, from_later).unwrap_or_else(|e| panic!("{e}"));
    let mut later = with_preferences(&newer);
    assert!(near(&later.state()["navigation.orbit_speed"], 0.5));
    later.ok("ui.navigation", json!({ "orbit_speed": 0.8 }));
    later.settle();
    assert_eq!(read(&newer), from_later, "left as it was");
    let problem = later.host.lock().shell.status.problem.clone();
    assert!(problem.is_some_and(|p| p.contains("version 2")), "said why");
}

#[test]
fn reset_puts_back_a_setting_a_page_or_everything_once_confirmed() {
    let mut ui = Ui::new();
    let state = ui.state();
    let orbit = state["navigation.orbit_speed"].clone();
    ui.ok(
        "pen.set",
        json!({ "size_sensitivity": 50, "lazy_pressure": 0.5 }),
    );
    ui.ok("ui.navigation", json!({ "orbit_speed": 1.5 }));
    ui.err("ui.settings", json!({ "page": "Nowhere" }));
    ui.err(
        "ui.settings",
        json!({ "reset": { "setting": "brush.size" } }),
    );

    // One setting: asked first, Cancel keeps it, Reset puts back its default.
    let one = json!({ "reset": { "setting": "pen.size_sensitivity" } });
    let asked = ui.ok("ui.settings", one.clone());
    assert!(asked["asking"].is_string(), "{asked}");
    ui.settle();
    assert_eq!(ui.state()["pen.size_sensitivity"], json!(50.0), "not yet");
    let cancelled = ui.ok("ui.settings", json!({ "confirm": false }));
    assert!(cancelled["asking"].is_null(), "{cancelled}");
    ui.settle();
    assert_eq!(ui.state()["pen.size_sensitivity"], json!(50.0), "cancelled");
    ui.ok("ui.settings", one);
    let done = ui.ok("ui.settings", json!({ "confirm": true }));
    assert_eq!(done["reset"], json!(["pen.size_sensitivity"]), "{done}");
    ui.settle();
    let state = ui.state();
    assert_eq!(state["pen.size_sensitivity"], json!(100.0));
    assert_eq!(state["pen.lazy_pressure"], json!(0.5), "only that setting");

    // A page: every setting on it, none elsewhere.
    ui.ok("pen.set", json!({ "color_sensitivity": 20 }));
    let page = json!({ "reset": { "page": "Pen" }, "confirm": true });
    let done = ui.ok("ui.settings", page);
    let names = done["reset"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        names.len(),
        2,
        "the two that were not at their defaults: {done}"
    );
    ui.settle();
    let state = ui.state();
    assert_eq!(state["pen.color_sensitivity"], json!(100.0));
    assert_eq!(state["pen.lazy_pressure"], json!(0.0));
    assert!(
        near(&state["navigation.orbit_speed"], 1.5),
        "another page's"
    );

    // Everything.
    ui.ok("ui.layout", json!({ "scale": 1.25 }));
    ui.ok("ui.settings", json!({ "reset": "all", "confirm": true }));
    ui.settle();
    let state = ui.state();
    assert_eq!(state["navigation.orbit_speed"], orbit);
    assert_eq!(state["layout.scale"], json!(1.0));
    // Reset runs each setting's own command, so what changes the session is logged.
    assert!(!ui.logged("pen.set").is_empty(), "Reset is logged");
}

#[test]
fn every_setting_is_a_command_and_search_finds_it_by_name() {
    let mut ui = Ui::new();
    let info = ui.ok("ui.settings", json!({}));
    let pages = json!([
        "Pen",
        "Interface",
        "Navigation",
        "Cursor",
        "Files",
        "Geometry",
        "Keys"
    ]);
    assert_eq!(info["pages"], pages);
    let listed = info["listed"].as_array().cloned().unwrap_or_default();
    assert!(listed.len() >= 30, "{} settings", listed.len());
    // Each by a name of its own: a slider or a switch by its label, a choice by its row's.
    let mut labels: Vec<&str> = listed.iter().filter_map(|s| s["label"].as_str()).collect();
    labels.sort_unstable();
    let twice: Vec<&&str> = labels
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| &w[0])
        .collect();
    assert!(twice.is_empty(), "settings named alike: {twice:?}");
    let state = ui.state();
    for setting in &listed {
        let name = setting["name"].as_str().unwrap_or_default();
        assert!(state.get(name).is_some(), "{name} is a session.state name");
        let step = &setting["reset"];
        let command = step["command"].as_str().unwrap_or_default();
        if !state[name].is_null() {
            let refused = ui.dry_run(command, step["args"].clone());
            assert_eq!(refused, None, "{name}: {step}");
        }
        let label = setting["label"].as_str().unwrap_or_default();
        let found = ui.ok("ui.settings", json!({ "search": label }))["found"].clone();
        let names: Vec<&str> = (found.as_array().into_iter().flatten())
            .filter_map(|f| f["name"].as_str())
            .collect();
        assert!(names.contains(&name), "{label} finds {name}: {names:?}");
    }
    // By command and by page too, every word in the text.
    let found = ui.ok("ui.settings", json!({ "search": "view.settings" }))["found"].clone();
    assert_eq!(
        found.as_array().map(Vec::len),
        Some(7),
        "the view moves, Orbit and Pivot among them: {found}"
    );
    let found = ui.ok("ui.settings", json!({ "search": "pen lazy" }))["found"].clone();
    assert_eq!(found[0]["name"], json!("pen.lazy_pressure"), "{found}");
    let cleared = ui.ok("ui.settings", json!({ "search": "" }));
    assert_eq!(cleared["found"], json!([]));
}

#[test]
fn with_move_pinned_a_relaunch_starts_with_move_and_the_pens_feel() {
    let path = file("prefs-relaunch");
    let mut ui = with_preferences(&path);
    ui.ok("brush.set", json!({ "brush": "Move" }));
    let pinned = ui.ok("ui.settings", json!({ "launch_brush": "current" }));
    assert_eq!(pinned["launch_brush"], json!("Move"));
    ui.ok(
        "pen.set",
        json!({ "strength_sensitivity": 40, "lazy_pressure": 0.3 }),
    );
    ui.ok(
        "brush.set",
        json!({ "brush": "Clay Build", "strength_increment": 0.1 }),
    );
    ui.ok("view.settings", json!({ "snap_deg": 15 }));
    ui.settle();
    assert_eq!(ui.state()["startup.brush"], json!("Move"));
    drop(ui);

    let mut again = with_preferences(&path);
    let state = again.state();
    assert_eq!(state["brush"], json!("Move"), "the pinned brush");
    assert_eq!(state["pen.strength_sensitivity"], json!(40.0));
    assert!(near(&state["pen.lazy_pressure"], 0.3), "{state}");
    assert!(near(&state["brush.strength_increment"], 0.1), "{state}");
    assert_eq!(state["view.snap_deg"], json!(15));
    assert!(!again.logged("pen.set").is_empty(), "set by a logged call");

    // Unpinned, the brush used last is the one at start.
    again.ok("ui.settings", json!({ "launch_brush": "last" }));
    again.ok("brush.set", json!({ "brush": "Clay Build" }));
    again.settle();
    drop(again);
    let mut third = with_preferences(&path);
    assert_eq!(third.state()["brush"], json!("Clay Build"));
}

#[test]
fn the_search_box_finds_a_setting_and_its_row_opens_its_page() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true }));
    ui.settle();
    let labels = ui.labels("settings");
    for label in [
        "Search",
        "Pen",
        "Interface",
        "Navigation",
        "Size sensitivity",
    ] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    assert!(!labels.iter().any(|l| l == "Orbit speed"), "one page shows");

    let none = Modifiers::default();
    ui.press("settings > Search");
    ui.input(&[Input::Text("orbit".into())], none);
    ui.settle();
    assert_eq!(ui.ok("ui.settings", json!({}))["search"], json!("orbit"));
    let labels = ui.labels("settings");
    assert!(
        labels.iter().any(|l| l == "Orbit speed"),
        "found: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l == "Size sensitivity"),
        "{labels:?}"
    );

    ui.press("settings > Orbit speed");
    let info = ui.ok("ui.settings", json!({}));
    assert_eq!(info["page"], json!("Navigation"), "{info}");
    assert_eq!(info["search"], json!(""));
    let labels = ui.labels("settings");
    assert!(labels.iter().any(|l| l == "Drag zoom speed"), "{labels:?}");
}

#[test]
fn reset_asks_first_and_cancel_keeps_the_settings() {
    let mut ui = Ui::new();
    ui.ok("pen.set", json!({ "lazy_pressure": 0.5 }));
    ui.ok("ui.settings", json!({ "open": true }));
    ui.settle();
    assert!(ui.widgets("reset").is_empty(), "nothing asked yet");

    ui.press("settings > Reset page");
    let labels = ui.labels("reset");
    for label in ["Reset", "Cancel"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    ui.press("reset > Cancel");
    assert!(ui.widgets("reset").is_empty(), "Cancel shuts the question");
    assert_eq!(ui.state()["pen.lazy_pressure"], json!(0.5), "kept");

    ui.press("settings > Reset page");
    ui.press("reset > Reset");
    assert!(ui.widgets("reset").is_empty(), "Reset shuts the question");
    assert_eq!(ui.state()["pen.lazy_pressure"], json!(0.0), "put back");
    let (code, _) = ui.err("ui.settings", json!({ "confirm": true }));
    assert_eq!(code, ErrorCode::Precondition, "nothing left to confirm");
}

#[test]
fn hole_closing_is_on_the_geometry_page_search_finds_it_and_reset_puts_back_large_triangles() {
    let path = file("prefs-hole-closing");
    let mut ui = with_preferences(&path);
    assert_eq!(ui.state()["voxel.close_holes"], json!(1), "at first");
    ui.ok("ui.settings", json!({ "open": true, "page": "Geometry" }));
    ui.settle();
    let labels = ui.labels("settings");
    for label in ["Voxel Remesh", "Hole closing", "None", "Large triangles"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    let found = ui.ok("ui.settings", json!({ "search": "hole closing" }))["found"].clone();
    assert_eq!(found[0]["name"], json!("voxel.close_holes"), "{found}");
    assert_eq!(found[0]["page"], json!("Geometry"), "{found}");
    ui.ok("ui.settings", json!({ "search": "" }));
    ui.settle();
    ui.press("settings > None");
    assert_eq!(ui.state()["voxel.close_holes"], json!(0), "a tap sets it");
    let text = read(&path);
    assert!(text.contains("[voxel]\nclose_holes = 0\n"), "{text}");
    drop(ui);

    let mut again = with_preferences(&path);
    assert_eq!(again.state()["voxel.close_holes"], json!(0), "kept");
    let reset = json!({ "reset": { "setting": "voxel.close_holes" }, "confirm": true });
    again.ok("ui.settings", reset);
    again.settle();
    assert_eq!(again.state()["voxel.close_holes"], json!(1), "put back");
    assert!(read(&path).contains("close_holes = 1"), "{}", read(&path));
}

/// Decision 0062, row 805: Orbit style and the pivot are rows of Settings > Navigation, as
/// Blender keeps Orbit Method and Orbit Around Selection in Preferences > Navigation; neither is
/// a button anywhere else. A tap sets the view's, lit on the one in use, and a restart keeps it.
#[test]
fn orbit_and_pivot_are_rows_on_the_navigation_page_and_survive_a_restart() {
    let path = file("prefs-orbit-pivot");
    let mut ui = with_preferences(&path);
    assert_eq!(ui.state()["view.orbit"], json!("y"), "a turntable at first");
    assert_eq!(ui.state()["view.pivot"], json!("target"));
    ui.ok("ui.settings", json!({ "open": true, "page": "Navigation" }));
    ui.settle();
    ui.press("settings > Free");
    ui.press("settings > Last edit");
    let state = ui.state();
    assert_eq!(state["view.orbit"], json!("free"));
    assert_eq!(state["view.pivot"], json!("last-edit"));
    let lit = |ui: &mut Ui, label: &str| {
        let widgets = ui.widgets("settings");
        let w = widgets.iter().find(|w| w["label"] == label);
        w.is_some_and(|w| w["selected"] == json!(true))
    };
    assert!(lit(&mut ui, "Free") && lit(&mut ui, "Last edit"), "lit");
    assert!(!lit(&mut ui, "Y"), "only the one in use");
    ui.settle();
    drop(ui);

    let mut again = with_preferences(&path);
    let state = again.state();
    assert_eq!(state["view.orbit"], json!("free"), "kept");
    assert_eq!(state["view.pivot"], json!("last-edit"), "kept");
}

/// A `layout.toml` written before `preferences.toml` existed: the scale, the theme, a view move,
/// two settings the window shows (Rows shown since S1) and one no page lists.
const OLD_LAYOUT: &str = "ui_scale = 1.5\ntheme = \"light\"\n\n[navigation]\norbit_speed = 1.25\n\n\
                          [settings]\ncenter_snap = false\npart_rows = 5\nclick_isolate = true\n";

/// What of [`OLD_LAYOUT`] moved, as `layout.toml` names it.
const MOVED: [&str; 6] = [
    "ui_scale",
    "theme",
    "[navigation]",
    "orbit_speed",
    "center_snap",
    "part_rows",
];

/// What of [`OLD_LAYOUT`] moved, as `preferences.toml` writes it.
const HANDED: [&str; 5] = [
    "scale = 1.5",
    "theme = \"light\"",
    "orbit_speed = 1.25",
    "center_snap = false",
    "part_rows = 5",
];

#[test]
fn layout_toml_no_longer_holds_what_moved_and_an_old_one_hands_it_over_once() {
    let dir = folder("prefs-one-home");
    let (layout, prefs) = (dir.join("layout.toml"), dir.join("preferences.toml"));
    std::fs::write(&layout, OLD_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = started(&dir);
    let state = ui.state();
    assert_eq!(state["layout.scale"], json!(1.5), "read once");
    assert_eq!(state["layout.theme"], json!("light"));
    assert!(near(&state["navigation.orbit_speed"], 1.25), "{state}");
    assert_eq!(state["settings.center_snap"], json!(false));
    let kept = read(&layout);
    for name in MOVED {
        assert!(!kept.contains(name), "{name} left layout.toml: {kept}");
    }
    assert!(
        kept.contains("click_isolate = true"),
        "no page lists it: {kept}"
    );
    let text = read(&prefs);
    for value in HANDED {
        assert!(text.contains(value), "{value} in preferences.toml: {text}");
    }

    // A change is written to preferences.toml only.
    ui.ok("ui.layout", json!({ "scale": 2.0 }));
    ui.ok("ui.layout", json!({ "theme": "high-contrast" }));
    ui.ok("ui.navigation", json!({ "orbit_speed": 0.5 }));
    ui.ok(
        "ui.settings",
        json!({ "center_snap": true, "dynamic_isolate": true }),
    );
    ui.settle();
    let kept = read(&layout);
    for name in MOVED {
        assert!(!kept.contains(name), "{name} not written: {kept}");
    }
    assert!(kept.contains("dynamic_isolate = true"), "{kept}");
    let text = read(&prefs);
    assert!(text.contains("orbit_speed = 0.5"), "{text}");
    drop(ui);

    // An old layout.toml again: where preferences.toml holds a value, it wins.
    std::fs::write(&layout, OLD_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    let mut again = started(&dir);
    let state = again.state();
    assert_eq!(state["layout.scale"], json!(2.0), "{state}");
    assert_eq!(state["layout.theme"], json!("high-contrast"));
    assert!(near(&state["navigation.orbit_speed"], 0.5), "{state}");
    assert_eq!(state["settings.center_snap"], json!(true));
    assert!(!read(&layout).contains("ui_scale"), "{}", read(&layout));
}

/// The hand-over is at launch, before any frame (decision 0052 as amended by step Prefs-c), so
/// a window stopped before its first frame loses nothing.
#[test]
fn a_window_stopped_before_its_first_frame_has_handed_an_old_layout_toml_over() {
    let dir = folder("prefs-early-stop");
    let (layout, prefs) = (dir.join("layout.toml"), dir.join("preferences.toml"));
    std::fs::write(&layout, OLD_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    drop(Ui::launch(&dir, &dir.join("state")));
    let text = read(&prefs);
    for value in HANDED {
        assert!(text.contains(value), "{value} in preferences.toml: {text}");
    }
    assert!(!text.contains("sticky_keys"), "a default waits: {text}");
    let kept = read(&layout);
    for name in MOVED {
        assert!(!kept.contains(name), "{name} left layout.toml: {kept}");
    }
    assert!(
        kept.contains("click_isolate = true"),
        "no page lists it: {kept}"
    );
    let mut again = started(&dir);
    let state = again.state();
    assert_eq!(state["layout.scale"], json!(1.5), "{state}");
    assert_eq!(state["layout.theme"], json!("light"));
    assert!(near(&state["navigation.orbit_speed"], 1.25), "{state}");
    assert_eq!(state["settings.center_snap"], json!(false));
}

/// The rig starts a window as the app does (decision 0052 as amended by step Prefs-c): a
/// relaunch reads `preferences.toml` with nothing asked a second time.
#[test]
fn a_relaunch_through_the_rig_reads_preferences_toml() {
    let dir = folder("prefs-rig-launch");
    let state = dir.join("state");
    let launch = || {
        let mut ui = Ui::launch(&dir, &state);
        ui.settle();
        ui
    };
    let mut ui = launch();
    ui.ok("pen.set", json!({ "lazy_pressure": 0.3 }));
    ui.settle();
    let text = read(&dir.join("preferences.toml"));
    assert!(text.contains("lazy_pressure = 0.3"), "written: {text}");
    ui.session.engine.close();
    drop(ui);
    let mut again = launch();
    let state = again.state();
    assert!(near(&state["pen.lazy_pressure"], 0.3), "read: {state}");
}

#[test]
fn the_files_page_names_the_pinned_brush() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true, "page": "Files" }));
    ui.settle();
    // The two buttons after the Brush at start group's label.
    let shown = |ui: &mut Ui| {
        ui.scroll_to("settings", "Last used");
        let labels = ui.labels("settings");
        let at = labels.iter().position(|l| l == "Brush at start");
        let at = at.unwrap_or_else(|| panic!("{labels:?}"));
        labels[at + 1..at + 3].to_vec()
    };
    assert_eq!(shown(&mut ui), ["Last used", "Pinned"], "nothing pinned");
    assert_eq!(ui.state()["startup.pinned"], Value::Null);
    ui.ok("brush.set", json!({ "brush": "Move" }));
    ui.settle();
    ui.press("settings > Pinned");
    assert_eq!(ui.state()["startup.pinned"], json!("Move"));
    assert_eq!(shown(&mut ui), ["Last used", "Pinned Move"], "named");
    ui.ok("brush.set", json!({ "brush": "Clay Build" }));
    ui.settle();
    assert_eq!(shown(&mut ui), ["Last used", "Pinned Move"], "still Move");
    ui.press("settings > Last used");
    assert_eq!(ui.state()["startup.pinned"], Value::Null);
    assert_eq!(shown(&mut ui), ["Last used", "Pinned"], "unpinned");
}
