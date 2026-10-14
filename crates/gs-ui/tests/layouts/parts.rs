//! Scene tray > Parts (decision 0033, section 7): a tap on a row's name makes its part active
//! and lights it alone, an eye flips its part and Shift on an eye gives every eye the state the
//! tap gives this one, and the header's Part menu (decision 0060 as amended for H4) works on the
//! active part with no form.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::rig::{Ui, center, held};

/// A sphere, a cube and a sphere named head, with the scene tray showing only Parts.
fn three() -> Ui {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({ "center": [0, 0, 3] }));
    let head = json!({ "name": "head", "center": [3, 0, 0] });
    ui.ok("primitive.sphere", head);
    ui.open_only(&["parts"]);
    ui
}

/// The scene tray's widgets running `command`.
fn running(ui: &mut Ui, command: &str) -> Vec<Value> {
    let widgets = ui.widgets("tray").into_iter();
    widgets
        .filter(|w| w["command"]["command"] == command)
        .collect()
}

/// The scene tray's widget labelled `label` that runs `command`.
fn widget(ui: &mut Ui, label: &str, command: &str) -> Value {
    let found = running(ui, command)
        .into_iter()
        .find(|w| w["label"] == label);
    found.unwrap_or_else(|| panic!("no {label} running {command} in the scene tray"))
}

/// A tap on the scene tray's widget labelled `label` that runs `command`.
fn tap(ui: &mut Ui, label: &str, command: &str) {
    let [x, y] = center(&widget(ui, label, command)["rect"]);
    ui.tap_at(x, y);
}

/// A tap on the eye on `part`'s row.
fn tap_eye(ui: &mut Ui, part: &str) {
    let [_, y] = center(&widget(ui, part, "part.select")["rect"]);
    let eyes = ui
        .widgets("tray")
        .into_iter()
        .filter(|w| w["label"] == "Visible");
    let found = eyes
        .map(|w| center(&w["rect"]))
        .find(|e| (e[1] - y).abs() < 1.0);
    let [x, y] = found.unwrap_or_else(|| panic!("no eye on {part}'s row"));
    ui.tap_at(x, y);
}

/// The names of the parts `part.list` marks with `key`, in list order.
fn marked(ui: &mut Ui, key: &str) -> Vec<String> {
    let list = ui.ok("part.list", json!({}))["parts"].clone();
    let parts = list.as_array().cloned().unwrap_or_default();
    let marked = parts.into_iter().filter(|p| p[key] == json!(true));
    marked
        .filter_map(|p| Some(p["name"].as_str()?.into()))
        .collect()
}

/// The names whose row is lit, in list order.
fn lit(ui: &mut Ui) -> Vec<String> {
    let names = running(ui, "part.select").into_iter();
    let lit = names.filter(|w| w["selected"] == json!(true));
    lit.filter_map(|w| Some(w["label"].as_str()?.into()))
        .collect()
}

#[test]
fn a_tap_on_a_name_makes_its_part_active_and_lights_it_alone() {
    let mut ui = three();
    assert_eq!(lit(&mut ui), ["head"], "the part added last");
    tap(&mut ui, "cube", "part.select");
    assert_eq!(ui.state()["parts.active"], json!("cube"));
    assert_eq!(lit(&mut ui), ["cube"]);
    assert_eq!(
        widget(&mut ui, "sphere", "part.select")["command"]["args"],
        json!({ "part": "sphere" }),
        "a row runs part.select with its part's name, as an agent would"
    );
}

#[test]
fn an_eye_flips_its_part_and_shift_gives_every_eye_its_new_state() {
    let mut ui = three();
    let all = ["sphere", "cube", "head"];
    ui.input(&[], held("Shift"));
    tap_eye(&mut ui, "cube");
    assert_eq!(marked(&mut ui, "visible"), Vec::<String>::new());
    assert_eq!(
        marked(&mut ui, "shown"),
        ["head"],
        "every part hidden but the active one, which always shows"
    );
    tap_eye(&mut ui, "cube");
    assert_eq!(
        marked(&mut ui, "visible"),
        all,
        "on a hidden part: all show"
    );
    ui.input(&[], Modifiers::default());
    tap_eye(&mut ui, "cube");
    assert_eq!(
        marked(&mut ui, "visible"),
        ["sphere", "head"],
        "the cube alone"
    );
    tap_eye(&mut ui, "cube");
    assert_eq!(marked(&mut ui, "visible"), all);
}

/// The parts named `p01` to `p<n>`, in order, as small cubes.
fn numbered(ui: &mut Ui, n: u32) {
    for i in 1..=n {
        let cube = json!({ "name": format!("p{i:02}"), "size": 0.2, "subdivisions": 1 });
        ui.ok("primitive.cube", cube);
    }
}

/// The names `p<from>` to `p<to>`.
fn names(from: u32, to: u32) -> Vec<String> {
    (from..=to).map(|i| format!("p{i:02}")).collect()
}

/// The part names the list shows, top to bottom.
fn rows(ui: &mut Ui) -> Vec<String> {
    let names = running(ui, "part.select").into_iter();
    names
        .filter_map(|w| Some(w["label"].as_str()?.into()))
        .collect()
}

#[test]
fn the_list_shows_its_row_count_and_scrolls_to_every_part() {
    let mut ui = Ui::new();
    numbered(&mut ui, 12);
    ui.open_only(&["parts"]);
    assert_eq!(
        ui.state()["settings.part_rows"],
        json!(8),
        "eight rows at first"
    );
    assert_eq!(
        rows(&mut ui),
        names(5, 12),
        "eight rows, scrolled to the active part, the one made last"
    );
    ui.ok("part.select", json!({ "part": "p01" }));
    assert_eq!(
        rows(&mut ui),
        names(1, 8),
        "the active part scrolls into sight"
    );
    // A slider sends a fraction: it rounds to whole rows.
    ui.ok("ui.settings", json!({ "part_rows": 3.4 }));
    assert_eq!(ui.state()["settings.part_rows"], json!(3));
    assert_eq!(rows(&mut ui), names(1, 3));
    for rows in [0.0, 24.6] {
        let (code, _) = ui.err("ui.settings", json!({ "part_rows": rows }));
        assert_eq!(code, gs_protocol::ErrorCode::ArgsInvalid, "{rows} rows");
    }
    // A drag up the list scrolls it, a row's height at a time, until the last part shows.
    let mut seen = rows(&mut ui);
    for _ in 0..12 {
        let shown = running(&mut ui, "part.select");
        let first = center(&shown[0]["rect"]);
        let last = center(&shown[shown.len() - 1]["rect"]);
        ui.touch(last[0], last[1], false);
        ui.touch(last[0], last[1], true);
        ui.touch(last[0], last[1] - 10.0, true);
        ui.touch(first[0], first[1], true);
        ui.touch(first[0], first[1], false);
        let now = rows(&mut ui);
        assert_eq!(now.len(), 3, "still three rows: {now:?}");
        seen.extend(now);
        if seen.contains(&"p12".to_string()) {
            break;
        }
    }
    assert!(
        seen.contains(&"p12".to_string()),
        "the drags reach the last part: {seen:?}"
    );
    assert_eq!(
        ui.state()["parts.active"],
        json!("p01"),
        "a drag selects nothing"
    );
    let all = ui.labels("tray");
    assert!(
        !all.iter().any(|l| l == "Add sphere"),
        "part.add's buttons are gone from under the list"
    );
}

#[test]
fn a_scene_of_a_thousand_parts_stays_usable() {
    use gs_protocol::{BatchParams, Outcome, Request, Step};
    // The same small stroke on a sphere, alone and among a thousand parts: the bytes it sends.
    let stroke = json!({ "brush": "Draw", "size": 0.05, "samples": [{ "position": [0, 0, 1] }] });
    let mut alone = Ui::with_looks();
    alone.ok("primitive.sphere", json!({ "subdivisions": 32 }));
    alone.ok("brush.stroke", stroke.clone());
    let sent_alone = uploaded(&alone);
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({ "subdivisions": 32 }));
    let cube = |i: u32| Step {
        command: "primitive.cube".into(),
        args: json!({ "name": format!("c{i:03}"), "size": 0.05, "subdivisions": 1 }),
    };
    let batch = BatchParams {
        commands: (1..1000).map(cube).collect(),
        if_rev: None,
    };
    let made = ui.session.request(Request::Batch(batch));
    assert!(matches!(made.outcome, Outcome::Ok(_)), "{:?}", made.outcome);
    let list = ui.ok("part.list", json!({}))["parts"].clone();
    let list = list.as_array().cloned().unwrap_or_default();
    assert_eq!(list.len(), 1000, "every part listed");
    let ids: Vec<u64> = list.iter().filter_map(|p| p["id"].as_u64()).collect();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "in order, each with its id"
    );
    assert_eq!(list[999]["name"], json!("c999"));
    // The panel: its rows and History under them, without scrolling the tray.
    ui.open_only(&["parts", "history"]);
    assert_eq!(rows(&mut ui).len(), 8, "eight rows of a thousand");
    assert!(
        rows(&mut ui).contains(&"c999".to_string()),
        "the active part, made last"
    );
    assert!(ui.labels("tray").iter().any(|l| l.starts_with("Undo")));
    ui.ok("part.select", json!({ "part": "sphere" }));
    assert_eq!(rows(&mut ui).first().map(String::as_str), Some("sphere"));
    // A stroke on the active part sends what it sends with the part alone; a camera move,
    // Dynamic off or on, sends nothing.
    ui.ok("brush.stroke", stroke);
    assert!(sent_alone > 0);
    assert_eq!(
        uploaded(&ui),
        sent_alone,
        "the stroke sends its part's nodes only"
    );
    ui.ok("view.orbit", json!({ "view": "main", "right_deg": 20 }));
    assert_eq!(uploaded(&ui), 0, "a camera move sends nothing");
    ui.ok("ui.settings", json!({ "dynamic_isolate": true }));
    let turn = |end: bool| json!({ "view": "main", "right_deg": 10, "gesture": 3, "end": end });
    ui.ok("view.orbit", turn(false));
    assert_eq!(uploaded(&ui), 0, "nor does a Dynamic drag");
    ui.ok("view.orbit", turn(true));
    assert_eq!(uploaded(&ui), 0);
}

/// Bytes the window sent to the GPU after the last call.
fn uploaded(ui: &Ui) -> u64 {
    ui.host
        .lock()
        .render
        .as_ref()
        .map_or(0, gs_ui::Render::uploaded)
}

/// `count` more small cubes, made in one batch, numbered on from the parts there are.
fn more_cubes(ui: &mut Ui, count: usize) {
    use gs_protocol::{BatchParams, Outcome, Request, Step};
    let had = ui.host.lock().shell.parts.len();
    let cube = |i: usize| Step {
        command: "primitive.cube".into(),
        args: json!({ "name": format!("c{i:04}"), "size": 0.05, "subdivisions": 1 }),
    };
    let batch = BatchParams {
        commands: (had..had + count).map(cube).collect(),
        if_rev: None,
    };
    let made = ui.session.request(Request::Batch(batch));
    assert!(matches!(made.outcome, Outcome::Ok(_)), "{:?}", made.outcome);
}

#[test]
fn the_window_s_work_after_a_call_grows_with_the_part_count_not_its_square() {
    // After each call the window finds every part's GPU copy, and asks the engine about the
    // controls it drew (the parts list draws only its rows in sight): at 1,000 and at 2,000
    // parts, the copies it looks at and the controls it asks about.
    let mut ui = Ui::with_looks();
    ui.open_only(&["parts"]);
    let mut work = Vec::new();
    for _ in 0..2 {
        more_cubes(&mut ui, 1000);
        ui.settle();
        ui.ok("view.orbit", json!({ "view": "main", "right_deg": 5 }));
        let mut studio = ui.host.lock();
        let looked = studio.render.as_ref().map_or(0, gs_ui::Render::looked);
        work.push((looked, studio.shell.controls().len()));
    }
    let [(looked, asked), (looked_2, asked_2)] = work[..] else {
        unreachable!("two counts")
    };
    assert!(looked >= 1000, "each part's copy looked for: {looked}");
    assert!(
        looked_2 <= 2 * looked,
        "twice the parts, at most twice the copies looked at: {looked} then {looked_2}"
    );
    assert_eq!(
        asked_2, asked,
        "as many controls asked about at 2,000 parts"
    );
}

/// The labels in the scene tray but the empty list's line and folded Detail's level: the summary
/// after Parts' title.
fn summary(ui: &mut Ui) -> Vec<String> {
    let widgets = ui.widgets("tray").into_iter();
    let labels = widgets.filter(|w| w["role"] == "label");
    let words = labels.filter_map(|w| Some(w["label"].as_str()?.to_string()));
    let others = |l: &String| l.starts_with("No parts") || l.starts_with("Level ");
    words.filter(|l| !others(l)).collect()
}

/// Parts' title names the active part after it, open or folded (decision 0060, section 7, as
/// amended for S1); an empty scene names none.
#[test]
fn the_parts_title_names_the_active_part_open_or_folded() {
    let mut ui = three();
    assert_eq!(summary(&mut ui), ["head"], "the part made last");
    tap(&mut ui, "cube", "part.select");
    assert_eq!(summary(&mut ui), ["cube"]);
    ui.open_only(&[]);
    assert_eq!(summary(&mut ui), ["cube"], "folded too");
    let mut ui = Ui::new();
    ui.open_only(&["parts"]);
    assert_eq!(summary(&mut ui), Vec::<String>::new(), "no part");
}

/// Rows shown left the tray for Settings > Interface (decision 0060, section 7), where it sets
/// how many rows the list shows, as it did.
#[test]
fn rows_shown_is_in_settings_interface_and_sets_the_list_s_rows() {
    let mut ui = Ui::new();
    numbered(&mut ui, 12);
    ui.open_only(&["parts"]);
    let tray = ui.labels("tray");
    assert!(!tray.iter().any(|l| l == "Rows shown"), "{tray:?}");
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    ui.press("settings > Interface");
    ui.scroll_to("settings", "Rows shown");
    let slider = crate::pen::widget(&mut ui, "settings", "Rows shown");
    assert_eq!(slider["role"], "slider");
    assert_eq!(slider["command"]["command"], "ui.settings");
    let r = ui.rect("settings > Rows shown");
    ui.tap_at(r[2] - 4.0, f64::midpoint(r[1], r[3]));
    let none = Modifiers::default();
    ui.input(&[gs_platform::Input::Text("3".into())], none);
    ui.tap("Return", none);
    assert_eq!(ui.state()["settings.part_rows"], json!(3));
    assert_eq!(rows(&mut ui).len(), 3, "three rows");
}

/// In a narrow scene tray each part's row fits (S1's notes, item 3): a long name is cut with
/// "…" to the room its switches leave, whole in `ui.describe`, not cut off by the
/// list's edge, and nothing, History's buttons under it included, reaches past the tray's
/// edge. At about 190 points the eye's and Paint's words leave the name less than a button's
/// least width, until they are icons (decision 0060, section 7).
#[test]
fn in_a_narrow_tray_a_long_part_name_is_cut_and_each_row_fits() {
    let mut ui = Ui::new();
    numbered(&mut ui, 11);
    let long = "a very long part name for the narrow tray";
    let cube = json!({ "name": long, "size": 0.2, "subdivisions": 1 });
    ui.ok("primitive.cube", cube);
    crate::history::narrow(&mut ui, 240.0);
    ui.open_only(&["parts", "history"]);
    let tray = ui.rect("tray");
    let named = widget(&mut ui, long, "part.select");
    let [_, one_row] = crate::history::size(&widget(&mut ui, "p11", "part.select"));
    assert_eq!(crate::history::size(&named)[1], one_row, "one row: {named}");
    let cut: Vec<_> = (running(&mut ui, "part.select").into_iter())
        .filter(|w| !w["cut"].is_null())
        .collect();
    assert!(
        cut.is_empty(),
        "no name cut off by the list's edge: {cut:?}"
    );
    let widgets = ui.widgets("tray");
    let past = widgets.iter().filter(|w| {
        let r = |i: usize| w["rect"][i].as_f64().unwrap_or_default();
        r(0) < tray[0] || r(2) > tray[2]
    });
    let past: Vec<_> = past.map(|w| (&w["label"], &w["rect"])).collect();
    assert!(past.is_empty(), "past the tray {tray:?}: {past:?}");
}

/// When Rows shown shrinks, the list scrolls to keep the active part in sight (S1's notes,
/// item 3), as it does when the active part changes.
#[test]
fn fewer_rows_shown_keep_the_active_part_in_sight() {
    let mut ui = Ui::new();
    numbered(&mut ui, 12);
    ui.open_only(&["parts"]);
    assert_eq!(
        rows(&mut ui),
        names(5, 12),
        "eight rows, the last made active"
    );
    ui.ok("ui.settings", json!({ "part_rows": 3 }));
    assert_eq!(rows(&mut ui), names(10, 12));
}

/// A pick of the Part menu's row `label` (decision 0060 as amended for H4).
fn part_menu(ui: &mut Ui, label: &str) {
    crate::header::open(ui, "Part");
    ui.press(&format!("menu > {label}"));
}

/// The Part menu works on the part lit in the list, with no form; with no part left its rows
/// are gray (tests/layouts/menus.rs, the Part menu with no part).
#[test]
fn the_operations_work_on_the_active_part_with_no_form() {
    let mut ui = three();
    tap(&mut ui, "cube", "part.select");
    part_menu(&mut ui, "Duplicate");
    assert_eq!(ui.state()["parts.active"], json!("cube-2"), "the copy");
    part_menu(&mut ui, "This part");
    assert_eq!(marked(&mut ui, "active"), ["head"], "the next part");
    let all = ["sphere", "cube", "head"];
    assert_eq!(marked(&mut ui, "visible"), all, "the copy is gone");
    assert!(ui.widgets("palette").is_empty(), "no form opened");
    part_menu(&mut ui, "All");
    crate::header::open(&mut ui, "Part");
    let duplicate = crate::header::item(&mut ui, "Duplicate");
    assert_eq!(
        duplicate["enabled"],
        json!(false),
        "no part, nothing to copy"
    );
    assert!(duplicate["reason"].is_string(), "{duplicate}");
}
