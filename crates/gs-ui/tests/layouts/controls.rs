//! Every control the window shows runs a registered command with arguments it accepts, and is
//! big enough for a pen: one check over the window, with the header's Look options open,
//! with the brush wheel open, with the palette open, with each Library tab open, with each
//! header menu open and with each scene tray section open on its own. A control whose command
//! cannot run now is drawn disabled and says why (decision 0025, section 6), and one that runs
//! its command leaves it in the command log as any call does. A layout's `line` puts controls on
//! one row and `while` shows one only while a state value holds (decision 0060, section 8).

use gs_protocol::ErrorCode;
use gs_ui::Directions;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::Ui;

/// The smallest a control may be, in points, so the pen hits it.
const PEN_TARGET: f64 = 30.0;

/// Whether `label` names a shipped button that only lists its options (Open a recent scene,
/// decision 0016, section 17, item 7), or a header options button (Look, decision 0060 as
/// amended for H5): it runs no command of its own.
fn lists_only(label: &Value) -> bool {
    let all = Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    let mut header = (all.list.iter()).filter_map(|d| d.header.as_ref());
    if header.any(|h| h.options.iter().any(|o| *label == o.label.as_str())) {
        return true;
    }
    let mut controls = all.controls.sets.iter().flat_map(|s| &s.controls);
    controls.any(|c| {
        let named = *label == c.label.as_str() || c.short.as_deref().is_some_and(|s| label == s);
        named && c.options.is_some() && c.command.is_none()
    })
}

/// Checks every control on screen, or in panel `only`; returns how many there were.
fn check_all(ui: &mut Ui, brushes: &[Value], seen: &mut Vec<String>, only: Option<&str>) -> usize {
    let d = ui.describe();
    let mut count = 0;
    let panels = d["panels"].as_array().into_iter().flatten();
    for panel in panels.filter(|p| only.is_none_or(|id| p["id"] == id)) {
        for w in panel["widgets"].as_array().into_iter().flatten() {
            let role = w["role"].as_str().unwrap_or_default();
            if !["button", "toggle", "slider", "list item"].contains(&role) {
                continue;
            }
            let at = format!("{} in {}", w["label"], panel["id"]);
            let [top, bottom] = [1, 3].map(|i| w["rect"][i].as_f64().unwrap_or_default());
            // A control partly scrolled out of its list is reported cut to what shows, and says so.
            let height = bottom - top;
            assert!(
                height >= PEN_TARGET || w["cut"] == true,
                "{at} is {height} points tall, under a pen target"
            );
            if w["enabled"] == false {
                assert!(
                    w["reason"].is_string(),
                    "{at} is disabled without saying why"
                );
                continue;
            }
            let step = &w["command"];
            // The brush tray's brush name, Pressure…, the tip's picture and Edit shape… open a
            // popover (decision 0060, section 6).
            let openers = ["Pressure…", "Tip picture", "Edit shape…"];
            let pops = panel["id"] == "brush"
                && (openers.contains(&w["label"].as_str().unwrap_or_default())
                    || brushes.contains(&w["label"]));
            if step.is_null() && (pops || lists_only(&w["label"])) {
                continue;
            }
            let command = step["command"]
                .as_str()
                .unwrap_or_else(|| panic!("{at} runs nothing"));
            let refused = ui.dry_run(command, step["args"].clone());
            // A palette row of a command that needs arguments asks for them before it runs.
            let asks = panel["id"] == "palette";
            let bad: &[ErrorCode] = if asks {
                &[ErrorCode::UnknownCommand]
            } else {
                &[
                    ErrorCode::UnknownCommand,
                    ErrorCode::ArgsInvalid,
                    ErrorCode::NotFound,
                ]
            };
            assert!(
                !refused.is_some_and(|c| bad.contains(&c)),
                "{at}: {step} is refused: {refused:?}"
            );
            if let Some(brush) = step["args"].get("brush") {
                assert!(
                    brushes.contains(brush),
                    "{at} picks {brush}, which is no brush"
                );
            }
            seen.push(command.to_string());
            count += 1;
        }
    }
    count
}

/// Checks the header's menus, each open in turn (decision 0060, section 2), as `check_all` does.
fn check_menus(ui: &mut Ui, brushes: &[Value], seen: &mut Vec<String>) {
    let menus = ui.ok("ui.menu", json!({}))["menus"].clone();
    for menu in menus.as_array().into_iter().flatten() {
        ui.ok("ui.menu", json!({ "open": menu }));
        assert!(
            check_all(ui, brushes, seen, Some("menu")) > 0,
            "{menu} has items"
        );
    }
    ui.ok("ui.menu", json!({ "open": false }));
}

#[test]
fn every_control_runs_a_registered_command_and_fits_a_pen() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let listed = ui.ok("brush.list", json!({}));
    let brushes: Vec<Value> = (listed["brushes"].as_array().into_iter().flatten())
        .map(|b| b["name"].clone())
        .collect();
    ui.open_look();
    // The falloff curve starts shut; open it, so its points are checked too.
    ui.host.lock().shell.layout.layout.fold("curve");
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let mut seen = Vec::new();
    let shown = check_all(&mut ui, &brushes, &mut seen, None);
    assert!(shown > 0, "controls show");
    ui.ok("ui.wheel", json!({ "open": true }));
    let with_wheel = check_all(&mut ui, &brushes, &mut seen, None);
    ui.ok("ui.wheel", json!({ "open": false }));
    assert!(with_wheel > brushes.len(), "the wheel adds every brush");
    ui.ok("ui.palette", json!({ "query": "" }));
    check_all(&mut ui, &brushes, &mut seen, None);
    ui.ok("ui.palette", json!({ "open": false }));
    ui.ok("ui.settings", json!({ "open": true }));
    check_all(&mut ui, &brushes, &mut seen, None);
    ui.ok("ui.settings", json!({ "open": false }));
    // The Library, each tab in turn: every tile is a control.
    let tabs = ui.ok("ui.library", json!({}))["tabs"].clone();
    for tab in tabs.as_array().into_iter().flatten() {
        ui.ok("ui.library", json!({ "tab": tab }));
        check_all(&mut ui, &brushes, &mut seen, Some("library"));
    }
    ui.ok("ui.library", json!({ "open": false }));
    check_menus(&mut ui, &brushes, &mut seen);
    // The scene tray is longer than the window: each of its sections alone, so every control
    // in it shows, then Detail with its level ⋯ open. A second level lets the level commands run.
    ui.ok("level.subdivide", json!({}));
    for id in ui.open_only(&[]) {
        ui.open_only(&[&id]);
        check_all(&mut ui, &brushes, &mut seen, Some("tray"));
    }
    ui.open_only(&["detail"]);
    ui.press("tray > ···");
    check_all(&mut ui, &brushes, &mut seen, Some("tray"));
    let reached = [
        "brush.set",
        "view.look",
        "view.display",
        "view.frame",
        "view.lens",
        "view.lock",
        "ui.layout",
        "history.undo",
        "primitive.sphere",
        "ui.wheel",
        "ui.palette",
        "mesh.voxel_remesh",
        "file.save",
        "exchange.write",
        "ui.menu",
        "level.subdivide",
        "level.step",
        "level.freeze",
        "live_detail.set",
        "part.set_visible",
        "part.delete",
        "mask.set",
        "mask.invert",
        "mask.filter",
        "mask.make",
        "visibility.hide",
        "visibility.grow",
        "visibility.show_all",
        "group.make",
        "ui.settings",
        "ui.navigation",
        "ui.library",
        "view.environment",
    ];
    for command in reached {
        assert!(
            seen.iter().any(|c| c == command),
            "a control reaches {command}"
        );
    }
}

/// The scene tray's control `label`, in its section `section` open on its own.
fn tray_control(ui: &mut Ui, section: &str, label: &str) -> Value {
    ui.open_only(&[section]);
    let found = ui.widgets("tray").into_iter().find(|w| w["label"] == label);
    found.unwrap_or_else(|| panic!("the scene tray shows {label}"))
}

#[test]
fn a_control_whose_command_cannot_run_is_disabled_until_it_can() {
    let mut ui = Ui::new();
    // An empty scene has nothing to subdivide: the button says why, and still names its command.
    let off = tray_control(&mut ui, "detail", "Subdivide");
    assert_eq!(off["enabled"], false, "{off}");
    assert_eq!(off["reason"], "The scene has no parts", "{off}");
    assert_eq!(off["command"]["command"], "level.subdivide", "{off}");
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let on = tray_control(&mut ui, "detail", "Subdivide");
    assert_eq!(on["enabled"], true, "{on}");
    assert!(on.get("reason").is_none(), "{on}");
    // A level command refused for the scene's levels says so the same way (decision 0030):
    // tests/layouts/detail.rs, the level's ⋯.
}

#[test]
fn a_tray_control_runs_its_command_into_the_log_as_any_call_does() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    tray_control(&mut ui, "detail", "Subdivide");
    let [x, y] = middle(ui.rect("tray > Subdivide"));
    ui.tap_at(x, y);
    let lines = ui.logged("level.");
    let ran: Vec<_> = lines.iter().map(|l| (&l["commands"], &l["role"])).collect();
    assert_eq!(
        ran,
        [(&json!(["level.subdivide"]), &json!("human"))],
        "{lines:?}"
    );
}

/// The settings file's sets with the set `text` (TOML, one `[[set]]`) after them, as a user's
/// controls file would put it: whether the loader takes it.
fn with_settings(ui: &mut Ui, text: &str) -> Result<(), String> {
    let file = "controls/settings.toml";
    let mut studio = ui.host.lock();
    let directions = &mut studio.shell.directions;
    let shipped = directions.control_files().find(|(f, _)| *f == file);
    let shipped =
        shipped.map_or_else(|| panic!("the settings file"), |(_, c)| gs_ui::to_toml(c))?;
    directions.add(file, &format!("{shipped}\n{text}"))
}

/// Puts the set `name`, written `text`, in the scene tray, open on its own.
fn with_set(ui: &mut Ui, name: &str, text: &str) {
    with_settings(ui, text).unwrap_or_else(|e| panic!("{name}: {e}"));
    {
        let mut studio = ui.host.lock();
        let shown = studio
            .shell
            .directions
            .list
            .iter_mut()
            .find(|d| d.name == "workbench");
        let tray = shown.and_then(|d| d.panels.iter_mut().find(|p| p.id == "tray"));
        let tray = tray.unwrap_or_else(|| panic!("the scene tray"));
        tray.show.push(name.into());
    }
    ui.open_only(&[name]);
}

/// Draws the window after a state change, as its next frames would.
fn refresh(ui: &mut Ui) {
    ui.settle();
    ui.state();
    ui.settle();
}

/// The rectangle of the scene tray's widget `label`.
fn at(ui: &mut Ui, label: &str) -> [f64; 4] {
    let tray = ui.widgets("tray");
    let found = tray.iter().find(|w| w["label"] == label);
    let found = found.unwrap_or_else(|| panic!("the tray shows {label}: {tray:?}"));
    std::array::from_fn(|i| found["rect"][i].as_f64().unwrap_or_default())
}

/// Decision 0060, section 8's `line`: controls with the same `line` share one row after that
/// label, each showing its `short`; the control after them starts a row of its own.
#[test]
fn line_puts_controls_on_one_row_after_its_label_each_showing_its_short() {
    let mut ui = Ui::new();
    let set = r#"
[[set]]
name = "line-test"
title = "Line test"
controls = [
  { label = "Mirror X", short = "X", line = "Mirror", command = "brush.set", args = { flip_mirror = "x" }, shows = "symmetry.x" },
  { label = "Mirror Y", short = "Y", line = "Mirror", command = "brush.set", args = { flip_mirror = "y" }, shows = "symmetry.y" },
  { label = "Mirror Z", short = "Z", line = "Mirror", command = "brush.set", args = { flip_mirror = "z" }, shows = "symmetry.z" },
  { label = "Symmetry on", command = "symmetry.set", args = { on = "flip" }, shows = "symmetry.on" },
]
"#;
    with_set(&mut ui, "line-test", set);
    let labels = ui.labels("tray");
    assert!(
        !labels.iter().any(|l| l.starts_with("Mirror ")),
        "shorts only: {labels:?}"
    );
    let row = ["Mirror", "X", "Y", "Z"].map(|label| at(&mut ui, label));
    let centre = |r: [f64; 4]| f64::midpoint(r[1], r[3]);
    for (r, next) in row.iter().zip(&row[1..]) {
        assert!(
            r[2] <= next[0] + 0.5,
            "{row:?} reads left to right on one row"
        );
        assert!(
            (centre(*r) - centre(*next)).abs() <= 2.0,
            "{row:?} on one row"
        );
    }
    let after = at(&mut ui, "Symmetry on");
    assert!(
        after[1] >= row[1][3] - 0.5,
        "{after:?} starts a row under {row:?}"
    );
}

/// Decision 0060, section 8's `while`: a control shows only while its state value holds: a
/// name while that value is truthy, a table while each value in it is equal, a list while any
/// of its entries holds. A value `session.state` does not report is refused.
#[test]
fn while_hides_a_control_until_its_state_value_holds() {
    let mut ui = Ui::new();
    let set = r#"
[[set]]
name = "while-test"
title = "While test"
controls = [
  { label = "Always", command = "view.level" },
  { label = "While X", command = "view.frame", while = "symmetry.x" },
  { label = "While X radial", command = "view.lock", while = { "symmetry.x_type" = "radial" } },
  { label = "While Y or X parallel", command = "view.lens", args = { projection = "toggle" }, while = ["symmetry.y", { "symmetry.x_type" = "parallel" }] },
]
"#;
    with_set(&mut ui, "while-test", set);
    let shown = |ui: &mut Ui, change: Value| {
        ui.ok("symmetry.set", change);
        refresh(ui);
        let labels = ui.labels("tray");
        let ours = [
            "Always",
            "While X",
            "While X radial",
            "While Y or X parallel",
        ];
        ours.into_iter()
            .filter(|l| labels.iter().any(|s| s == l))
            .collect::<Vec<_>>()
    };
    let none = json!({ "x": false, "y": false, "x_type": "mirror" });
    assert_eq!(shown(&mut ui, none), ["Always"]);
    assert_eq!(shown(&mut ui, json!({ "x": true })), ["Always", "While X"]);
    let radial = shown(&mut ui, json!({ "x_type": "radial" }));
    assert_eq!(radial, ["Always", "While X", "While X radial"]);
    let parallel = shown(&mut ui, json!({ "x_type": "parallel" }));
    assert_eq!(parallel, ["Always", "While X", "While Y or X parallel"]);
    let y = json!({ "x": false, "x_type": "mirror", "y": true });
    assert_eq!(shown(&mut ui, y), ["Always", "While Y or X parallel"]);
    let unknown = r#"
[[set]]
name = "while-unknown"
title = "While unknown"
controls = [{ label = "Never", command = "view.level", while = "symmetry.w" }]
"#;
    let refused = with_settings(&mut ui, unknown);
    assert!(
        refused.is_err_and(|e| e.contains("symmetry.w")),
        "refused naming it"
    );
}

/// Rule 8.1 of decision 0025, one home per control: no two controls in the shipped sets run the
/// same command with the same arguments. The quick menu, the wheel and the palette reach
/// controls through their sets, so they are not homes of their own.
#[test]
fn every_control_has_one_home() {
    let all = Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    let mut homes: Vec<(&str, &toml::Table, &str)> = Vec::new();
    for set in &all.controls.sets {
        for c in &set.controls {
            let Some(command) = c.command.as_deref() else {
                continue;
            };
            let same = |(other, args, _): &&(&str, &toml::Table, &str)| {
                *other == command && **args == c.args
            };
            if let Some((_, _, home)) = homes.iter().find(same) {
                panic!("{} ({command}) is in both {home} and {}", c.label, set.name);
            }
            homes.push((command, &c.args, &set.name));
        }
    }
}

/// A piece `set/` names the set's controls with no group, as the quick menu's `mask/` names the
/// Mask menu's first three rows and not its Filter and Make groups (decision 0060, amended for
/// H3).
#[test]
fn a_piece_with_an_empty_group_names_the_controls_with_no_group() {
    let all = Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    let picked = all.controls.piece("mask/");
    let labels = picked.map(|(_, cs)| cs.iter().map(|c| c.label.clone()).collect::<Vec<_>>());
    let want = ["Mask all", "Invert mask", "Clear mask"].map(String::from);
    assert_eq!(labels.as_deref(), Some(want.as_slice()));
}
