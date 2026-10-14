//! A control's slider whose argument sits inside a table, named by a dotted path in the
//! controls file (`args = { subsurface.strength = "drag" }`), sets that argument alone; one
//! whose value can be a word ("auto") sets a number when dragged. Every slider also takes a
//! typed value and the wheel (decision 0025, section 4), moves from where it is when dragged,
//! rounds with Shift, snaps as Settings > Sliders says, and opens a fine handle on a
//! double-click (`interface.sliders`).

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, held};

/// Drags the slider `name` with the pen from its rail, 20 points in from its left end, to `x`,
/// and lifts there. A drag moves the value from where it is, by the rail's width end to end.
fn drag_to(ui: &mut Ui, name: &str, x: f64) {
    let r = ui.rect(name);
    let [from, y] = [r[0] + 20.0, middle(r)[1]];
    ui.touch(from, y, false);
    ui.touch(from, y, true);
    ui.touch(x, y, true);
    ui.touch(x, y, false);
}

#[test]
fn a_slider_on_a_dotted_path_sets_its_nested_argument() {
    let mut ui = Ui::with_looks();
    // Subsurface shows in Lit mode (decision 0060 as amended for R1).
    ui.ok("view.display", json!({ "shading": "lit" }));
    ui.open_look();
    let before = ui.state();
    assert_eq!(before["view.subsurface.strength"], json!(1.0));
    let r = ui.rect("look > Subsurface strength");
    drag_to(&mut ui, "look > Subsurface strength", r[0] - 200.0);
    let after = ui.state();
    let strength = after["view.subsurface.strength"].as_f64().unwrap_or(1.0);
    assert!(strength < 0.01, "dragged to its left end: {strength}");
    for other in ["mode", "radius", "unit_mm"] {
        let name = format!("view.subsurface.{other}");
        assert_eq!(after[&name], before[&name], "{name} is left alone");
    }
}

/// The occlusion radius is "auto" until set (its slider reads so, tests/ui/sliders.rs): a drag
/// sets a number, and Automatic radius turns it back.
#[test]
fn the_occlusion_radius_slider_sets_a_number_and_automatic_radius_turns_it_back() {
    let mut ui = Ui::with_looks();
    ui.open_look();
    let radius = |ui: &mut Ui| ui.state()["view.occlusion_radius"].clone();
    assert_eq!(radius(&mut ui), json!("auto"));
    let r = ui.rect("look > Occlusion radius");
    drag_to(&mut ui, "look > Occlusion radius", r[2] + 40.0);
    assert_eq!(radius(&mut ui), json!(1.0), "dragged to its right end");
    let [x, y] = middle(ui.rect("look > Auto"));
    ui.tap_at(x, y);
    assert_eq!(
        radius(&mut ui),
        json!("auto"),
        "Automatic radius turns it back"
    );
}

/// Taps the value of the slider `name`: the end of its label, right of its rail.
fn tap_value(ui: &mut Ui, name: &str) {
    let r = ui.rect(name);
    ui.tap_at(r[2] - 4.0, middle(r)[1]);
}

/// Types `text` in the field a tap opened, then presses `key` (`Return` or `Escape`).
fn type_in(ui: &mut Ui, text: &str, key: &str) {
    let none = Modifiers::default();
    ui.input(&[Input::Text(text.into())], none);
    ui.tap(key, none);
}

/// A brush value from `session.state`.
fn brush(ui: &mut Ui, name: &str) -> f64 {
    ui.state()[name].as_f64().unwrap_or_default()
}

/// Opens the quick menu over the 3D view, with the pen there.
fn open_quick(ui: &mut Ui) {
    let [x0, y0, x1, y1] = ui.rect("viewport");
    ui.pen(x0 + (x1 - x0) * 0.3, y0 + (y1 - y0) * 0.3);
    ui.tap("Space", held("Shift"));
    assert!(!ui.widgets("quick").is_empty(), "Shift+Space opens it");
    // Shift let go: while it is held the Smooth slot's brush shows.
    ui.input(&[], Modifiers::default());
}

/// A tap on a slider's value opens a field; Enter sets what was typed, in the units the slider
/// shows (Strength's percent), with them or without, by the command a drag runs, once.
#[test]
fn a_value_typed_into_a_slider_sets_it_once() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    tap_value(&mut ui, "brush > Strength");
    type_in(&mut ui, "42", "Return");
    let strength = brush(&mut ui, "brush.strength");
    assert!((strength - 0.42).abs() < 1e-6, "42% is 0.42: {strength}");
    tap_value(&mut ui, "brush > Size");
    type_in(&mut ui, "120 px", "Return");
    assert_eq!(brush(&mut ui, "brush.size_px"), 120.0, "with its units");
    assert_eq!(ui.logged("brush.set").len(), 2, "one log line a value");
}

/// Escape closes the field setting nothing, and leaves the quick menu it is in open; the next
/// Escape closes the menu.
#[test]
fn escape_leaves_a_typed_value_and_the_menu_open() {
    let mut ui = Ui::new();
    open_quick(&mut ui);
    let size = brush(&mut ui, "brush.size_px");
    tap_value(&mut ui, "quick > Size");
    type_in(&mut ui, "300", "Escape");
    assert_eq!(brush(&mut ui, "brush.size_px"), size, "nothing set");
    assert!(ui.logged("brush.set").is_empty(), "nothing run");
    assert!(!ui.widgets("quick").is_empty(), "the menu stays open");
    ui.tap("Escape", Modifiers::default());
    assert!(ui.widgets("quick").is_empty(), "the next Escape closes it");
}

/// A typed value may go past the rail's end up to what the command takes, as Blender's soft
/// and hard limits; past that the command refuses it with its reason and nothing changes.
#[test]
fn a_typed_value_goes_past_the_rail_up_to_what_the_command_takes() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    tap_value(&mut ui, "brush > Size");
    type_in(&mut ui, "3000", "Return");
    assert_eq!(
        brush(&mut ui, "brush.size_px"),
        3000.0,
        "past the rail's 2000"
    );
    tap_value(&mut ui, "brush > Size");
    type_in(&mut ui, "20000", "Return");
    assert_eq!(brush(&mut ui, "brush.size_px"), 3000.0, "refused");
    let problem = ui.host.lock().shell.status.problem.clone();
    assert!(
        problem.is_some_and(|p| p.contains("maximum")),
        "the status line says why"
    );
    // A control's slider the same: orbit speed's rail stops at 2, `ui.navigation` takes 10.
    ui.ok("ui.settings", json!({ "open": true, "page": "Navigation" }));
    ui.settle();
    tap_value(&mut ui, "settings > Orbit speed");
    type_in(&mut ui, "3", "Return");
    assert_eq!(ui.state()["navigation.orbit_speed"], json!(3.0));
}

/// The wheel over a slider steps it a hundredth of its rail a notch, a thousandth with Shift,
/// and the 3D view under the menu does not dolly; with the menu shut, the wheel dollies the view
/// (off the menu while it shows, it sizes the brush: `quick_wheel.rs`).
#[test]
fn the_wheel_steps_the_slider_under_it_and_not_the_view() {
    let mut ui = Ui::new();
    open_quick(&mut ui);
    let [x, y] = middle(ui.rect("quick > Strength"));
    ui.pen(x, y);
    let before = brush(&mut ui, "brush.strength");
    let wheel = [Input::Wheel { dx: 0.0, dy: 1.0 }];
    ui.input(&wheel, Modifiers::default());
    let after = brush(&mut ui, "brush.strength");
    assert!((after - before - 0.01).abs() < 1e-6, "{before} to {after}");
    // Shift holds the Smooth slot, whose brush the menu shows and the fine step moves
    // (brush-settings.modifier-key-slots).
    ui.input(&[], held("Shift"));
    let smooth = brush(&mut ui, "brush.strength");
    ui.input(&[Input::Wheel { dx: 0.0, dy: -1.0 }], held("Shift"));
    let fine = brush(&mut ui, "brush.strength");
    assert!((smooth - fine - 0.001).abs() < 1e-6, "{smooth} to {fine}");
    assert_eq!(ui.logged("brush.set").len(), 2, "one log line a notch");
    assert!(ui.logged("view.dolly").is_empty(), "the view stays");
    let menu = ui.rect("quick");
    let [_, _, x1, y1] = ui.rect("viewport");
    ui.pen(f64::midpoint(menu[2], x1), f64::midpoint(menu[3], y1));
    ui.tap("Escape", Modifiers::default());
    ui.input(&wheel, Modifiers::default());
    assert_eq!(
        ui.logged("view.dolly").len(),
        1,
        "with the menu shut it dollies"
    );
}

/// The brush tray's slider `label` as `ui.describe` reports it.
fn slider(ui: &mut Ui, label: &str) -> Option<Value> {
    (ui.widgets("brush").into_iter()).find(|w| w["label"] == label)
}

/// The value the brush tray's slider `label` shows now: what its `brush.set` would set.
fn shown(ui: &mut Ui, label: &str, arg: &str) -> f64 {
    let found = slider(ui, label).unwrap_or_else(|| panic!("no {label} in the brush tray"));
    found["command"]["args"][arg].as_f64().unwrap_or(f64::NAN)
}

/// Where the brush tray's slider `label` is pressed: 20 points into its rail, at its middle.
fn on_rail(ui: &mut Ui, label: &str) -> [f64; 2] {
    let r = ui.rect(&format!("brush > {label}"));
    [r[0] + 20.0, middle(r)[1]]
}

/// Where Brush tray > Tip's Softness (the brush's Falloff, decision 0060 section 6) is pressed,
/// Tip opened first: it starts shut.
fn on_softness(ui: &mut Ui) -> [f64; 2] {
    let tip = ui.rect("brush > curve");
    ui.tap_at(middle(tip)[0], tip[1] + 10.0);
    on_rail(ui, "Softness")
}

/// A drag with the pen from `[x, y]`, `dx` points along, lifted there.
fn drag(ui: &mut Ui, [x, y]: [f64; 2], dx: f64) {
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x + dx, y, true);
    ui.touch(x + dx, y, false);
}

/// Waits longer than egui counts clicks together (a triple click's 0.6 seconds), so the next
/// two taps are a double-click and not a triple.
fn pause(ui: &mut Ui) {
    ui.wait(0.7);
}

/// A press on a slider leaves it where it is (egui's slider jumped to the pen); the drag then
/// moves it from there by the rail's width end to end.
#[test]
fn a_drag_moves_a_slider_from_where_it_is_and_a_press_does_not_jump_it() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let [x, y] = on_rail(&mut ui, "Strength");
    ui.tap_at(x, y);
    assert!(ui.logged("brush.set").is_empty(), "a tap sets nothing");
    pause(&mut ui);
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    let pressed = shown(&mut ui, "Strength", "strength");
    assert!(
        (pressed - 0.4).abs() < 1e-6,
        "the press leaves 40%: {pressed}"
    );
    ui.touch(x + 20.0, y, true);
    let moved = shown(&mut ui, "Strength", "strength");
    assert!(
        moved > 0.45 && moved < 0.7,
        "20 points right of 40%, though the pen is near the rail's left end: {moved}"
    );
    ui.touch(x + 20.0, y, false);
    let set = brush(&mut ui, "brush.strength");
    assert!((set - moved).abs() < 1e-6, "set where it let go: {set}");
}

/// Shift held while dragging rounds the drag to whole numbers in the units shown: Strength's
/// percent. Shift pressed after the press leaves the drag on the brush it started on, as it
/// leaves a stroke (`brush-settings.modifier-key-slots` holds the Smooth slot on Shift only
/// while the pen is up), so Draw's Strength is rounded and the Smooth slot's left alone.
#[test]
fn shift_rounds_a_drag_to_whole_numbers_in_the_units_shown() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let smooth = |ui: &mut Ui| {
        ui.input(&[], held("Shift"));
        let strength = brush(ui, "brush.strength");
        ui.input(&[], Modifiers::default());
        strength
    };
    let smooth_before = smooth(&mut ui);
    let [x, y] = on_rail(&mut ui, "Strength");
    drag(&mut ui, [x, y], -23.0);
    let free = brush(&mut ui, "brush.strength") * 100.0;
    assert!(
        (free - free.round()).abs() > 0.01,
        "without Shift, between whole percents: {free}"
    );
    ui.ok("brush.set", json!({ "strength": 0.4 }));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x - 10.0, y, true);
    ui.input(&[], held("Shift"));
    ui.touch(x - 23.0, y, true);
    ui.touch(x - 23.0, y, false);
    ui.input(&[], Modifiers::default());
    let rounded = brush(&mut ui, "brush.strength") * 100.0;
    assert!(
        (rounded - free.round()).abs() < 1e-3,
        "{free}% with Shift reads {rounded}% on Draw"
    );
    assert_eq!(
        smooth(&mut ui),
        smooth_before,
        "the Smooth slot's is left alone"
    );
}

/// With Settings > Sliders' Falloff snap on, Shift held as a drag of Softness starts takes it to
/// -100, 0 and 100 only, through the middle; letting go of Shift gives the drag back its every
/// value. Shift held before the press holds the Smooth slot (`brush-settings.modifier-key-slots`),
/// so the slider shows and sets the Smooth slot brush's Falloff, and letting go of Shift during
/// the drag leaves the drag on that brush: Draw's Falloff is left alone.
#[test]
fn with_the_falloff_snap_on_a_shift_drag_takes_only_its_stops() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.ok("ui.settings", json!({ "falloff_snap": true }));
    assert_eq!(ui.state()["settings.falloff_snap"], json!(true));
    let draw = brush(&mut ui, "brush.falloff");
    let [x, y] = on_softness(&mut ui);
    ui.input(&[], held("Shift"));
    ui.ok("brush.set", json!({ "falloff": -100.0 }));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    // Five points at a time until it takes 100, so the pen stops short of the rail's end
    // (Softness shares its row with the tip's picture).
    let (mut seen, mut dx) = (Vec::new(), 0.0);
    while !seen.contains(&100.0) && dx < 400.0 {
        dx += 5.0;
        ui.touch(x + dx, y, true);
        seen.push(shown(&mut ui, "Softness", "falloff"));
    }
    let stops = [-100.0, 0.0, 100.0];
    assert!(seen.iter().all(|v| stops.contains(v)), "{seen:?}");
    assert!(seen.contains(&0.0) && seen.contains(&100.0), "{seen:?}");
    ui.input(&[], Modifiers::default());
    let free = shown(&mut ui, "Softness", "falloff");
    assert!(
        !stops.contains(&free),
        "Shift let go, the drag is free: {free}"
    );
    ui.touch(x + dx, y, false);
    assert_eq!(
        brush(&mut ui, "brush.falloff"),
        draw,
        "Draw's is left alone"
    );
    ui.input(&[], held("Shift"));
    let set = brush(&mut ui, "brush.falloff");
    assert!((set - free).abs() < 1e-3, "the Smooth slot's: {set}");
    ui.input(&[], Modifiers::default());
}

/// Drags the brush's Falloff (Tip's Softness, open) from -20 a point at a time, without
/// lifting: what it shows on the way.
fn falloff_through_the_middle(ui: &mut Ui) -> Vec<f64> {
    ui.ok("brush.set", json!({ "falloff": -20.0 }));
    let [x, y] = on_rail(ui, "Softness");
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    let mut seen = Vec::new();
    for step in 1..=25 {
        ui.touch(x + f64::from(step), y, true);
        seen.push(shown(ui, "Softness", "falloff"));
    }
    ui.touch(x + 25.0, y, false);
    seen
}

/// The center snap (on at first) holds a rail through 0 at 0 in a small dead zone; off, the
/// drag passes through every value.
#[test]
fn the_center_snap_holds_falloff_at_zero_until_turned_off() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    assert_eq!(ui.state()["settings.center_snap"], json!(true));
    let near = |v: &&f64| v.abs() > 0.0 && v.abs() < 4.0;
    on_softness(&mut ui);
    let snapped = falloff_through_the_middle(&mut ui);
    assert!(snapped.contains(&0.0), "{snapped:?}");
    assert!(!snapped.iter().any(|v| near(&v)), "{snapped:?}");
    ui.ok("ui.settings", json!({ "center_snap": false }));
    let free = falloff_through_the_middle(&mut ui);
    assert!(free.iter().any(|v| near(&v)), "{free:?}");
}

/// A double-click on a slider opens its typing field and a fine handle under it, which moves the
/// value a tenth as far as the rail does for the same drag; a press off them shuts it.
#[test]
fn a_double_click_shows_the_fine_handle_and_the_typing_box() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let [x, y] = on_rail(&mut ui, "Strength");
    ui.tap_at(x, y);
    ui.tap_at(x, y);
    let labels = ui.labels("brush");
    for open in ["Fine Strength", "Type Strength"] {
        assert!(labels.iter().any(|l| l == open), "{open}: {labels:?}");
    }
    let fine = ui.rect("brush > Fine Strength");
    assert!(fine[3] - fine[1] >= 30.0, "a pen target: {fine:?}");
    // 20 points: Strength's rail in Basics, beside its pen toggle, takes that far from 40% to
    // under its end.
    let before = brush(&mut ui, "brush.strength");
    drag(&mut ui, middle(fine), 20.0);
    let after_fine = brush(&mut ui, "brush.strength");
    assert!(
        slider(&mut ui, "Fine Strength").is_some(),
        "a drag on it leaves it open"
    );
    drag(&mut ui, [x, y], 20.0);
    let after_rail = brush(&mut ui, "brush.strength");
    let (fine, rail) = (after_fine - before, after_rail - after_fine);
    assert!(
        fine > 0.0 && (rail - 10.0 * fine).abs() < 1e-3,
        "{fine} and {rail}"
    );
    let size = on_rail(&mut ui, "Size");
    ui.tap_at(size[0], size[1]);
    assert!(
        slider(&mut ui, "Fine Strength").is_none(),
        "a press elsewhere shuts it"
    );
}

/// Ctrl+double-click puts the default back: the preset's own Strength, through the `brush.set` a
/// drag runs, and the window's own orbit speed. Ctrl holds the Mask slot
/// (`brush-settings.modifier-key-slots`), so the brush sliders show and reset the Mask slot
/// brush's: Draw keeps its own change.
#[test]
fn ctrl_double_click_puts_the_default_back() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.ok("brush.set", json!({ "strength": 0.9 }));
    let [x, y] = on_rail(&mut ui, "Strength");
    ui.input(&[], held("Ctrl"));
    let own = brush(&mut ui, "brush.strength");
    ui.ok("brush.set", json!({ "strength": 0.9 }));
    assert!((own - 0.9).abs() > 0.01, "Mask Pen's own is not 90%: {own}");
    ui.tap_at(x, y);
    ui.tap_at(x, y);
    let strength = brush(&mut ui, "brush.strength");
    assert!((strength - own).abs() < 1e-6, "Mask Pen's own: {strength}");
    ui.input(&[], Modifiers::default());
    let draw = brush(&mut ui, "brush.strength");
    assert!((draw - 0.9).abs() < 1e-6, "Draw keeps its 90%: {draw}");
    assert!(
        slider(&mut ui, "Fine Strength").is_none(),
        "nothing left open"
    );
    let speed = ui.state()["navigation.orbit_speed"].clone();
    ui.ok("ui.navigation", json!({ "orbit_speed": 1.5 }));
    ui.ok("ui.settings", json!({ "open": true, "page": "Navigation" }));
    ui.settle();
    pause(&mut ui);
    let r = ui.rect("settings > Orbit speed");
    ui.input(&[], held("Ctrl"));
    ui.tap_at(r[0] + 20.0, middle(r)[1]);
    ui.tap_at(r[0] + 20.0, middle(r)[1]);
    ui.input(&[], Modifiers::default());
    assert_eq!(ui.state()["navigation.orbit_speed"], speed);
}

/// With Settings > Sliders' Show slider values off, a slider reads its title alone until the pen
/// is over it.
#[test]
fn with_show_values_off_a_slider_reads_its_value_only_under_the_pen() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let shown = ui.rect("brush > Strength");
    ui.ok("ui.settings", json!({ "open": true, "page": "Interface" }));
    ui.settle();
    ui.press("settings > Show slider values");
    assert_eq!(ui.state()["settings.show_values"], json!(false));
    ui.pen(x, y);
    let hidden = ui.rect("brush > Strength");
    assert!(hidden[2] < shown[2], "{hidden:?} against {shown:?}");
    let [x, y] = on_rail(&mut ui, "Strength");
    ui.pen(x, y);
    let under = ui.rect("brush > Strength");
    assert!(
        (under[2] - shown[2]).abs() < 0.5,
        "{under:?} against {shown:?}"
    );
}

/// A slider says its hint on hover as a button does (decision 0060, section 11): `ui.describe`
/// gives it, and with Ctrl held over it the description box says it with what it runs. Size is
/// a brush value's slider, Spacing a controls file's.
#[test]
fn hovering_a_slider_shows_its_hint_as_a_buttons_does() {
    let mut ui = Ui::new();
    let hints = [
        (
            "Size",
            "How big the brush is, in screen pixels. Keys: S held, [ and ]. (Blender: Radius.)",
        ),
        (
            "Spacing",
            "The gap between dabs, as a share of the size. Lower is smoother and slower. (Was Step.)",
        ),
    ];
    for (label, says) in hints {
        let found = slider(&mut ui, label).unwrap_or_else(|| panic!("no {label}"));
        assert_eq!(found["hint"], says, "{label} in ui.describe");
        let [x, y] = middle(ui.rect(&format!("brush > {label}")));
        ui.pen(x, y);
        ui.input(&[], held("Ctrl"));
        let shown = ui.labels("description");
        assert!(shown.iter().any(|l| l == says), "{label}: {shown:?}");
        assert!(shown.iter().any(|l| l.contains("brush.set")), "{shown:?}");
        ui.input(&[], Modifiers::default());
    }
}
