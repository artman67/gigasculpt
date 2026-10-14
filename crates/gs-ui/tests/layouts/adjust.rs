//! The adjust panel (decision 0060, section 5, as amended 2026-10-13, step H2): a command run
//! from the palette or a key that made one undo step opens a panel in the 3D view's corner away
//! from the pen, titled with the command and filled with the step's arguments. A change undoes
//! the step and runs it again as one step; a refused one puts the old step back and says why
//! under its field. A new step, an undo, its × and Escape over it close it; F9 opens it again
//! while its step is the newest, else says there is nothing to adjust. A tray's button not
//! marked `adjust` and a view setting never open it. Its fields list the step's own arguments
//! first, in the command's order, and hide those of the command's other form (`A1`, Add's
//! primitives).

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, held};

/// The adjust panel as `ui.describe` reports it, while it shows.
fn panel(ui: &mut Ui) -> Option<Value> {
    crate::header::panel(ui, "adjust")
}

/// The panel's widget labelled `label`.
fn widget(ui: &mut Ui, label: &str) -> Value {
    crate::pen::widget(ui, "adjust", label)
}

/// What the panel's Resolution slider shows.
fn resolution(ui: &mut Ui) -> Value {
    widget(ui, "Resolution")["command"]["args"]["resolution"].clone()
}

/// The newest undo step's resolution, as the engine's history keeps it.
fn newest_resolution(ui: &mut Ui) -> Value {
    let list = ui.ok("history.list", json!({ "limit": 1 }));
    list["steps"][0]["args"]["resolution"].clone()
}

/// A rectangle `ui.describe` reports.
fn rect(of: &Value) -> [f64; 4] {
    std::array::from_fn(|i| of["rect"][i].as_f64().unwrap_or_default())
}

/// A small sphere named ball, quick to remesh.
fn ball(ui: &mut Ui) {
    let ball = json!({ "name": "ball", "subdivisions": 3 });
    ui.ok("primitive.sphere", ball);
}

/// Voxel Remesh at `resolution`, run from the palette's form.
fn remesh_from_palette(ui: &mut Ui, resolution: u32) {
    let args = json!({ "resolution": resolution });
    let form = json!({ "form": "mesh.voxel_remesh", "args": args });
    ui.ok("ui.palette", form);
    ui.press("palette > Voxel remesh");
}

/// Types `text` into the panel's field `name` (a slider's number when `slider`) and presses
/// Enter.
fn type_into(ui: &mut Ui, name: &str, text: &str, slider: bool) {
    let r = ui.rect(&format!("adjust > {name}"));
    let x = if slider { r[2] - 4.0 } else { middle(r)[0] };
    ui.tap_at(x, middle(r)[1]);
    let none = Modifiers::default();
    ui.input(&[Input::Text(text.into())], none);
    ui.tap("Return", none);
}

/// Add > `primitive`, which opens the panel with its settings: the panel as `ui.describe`
/// reports it.
fn add(ui: &mut Ui, primitive: &str) -> Value {
    crate::header::open(ui, "Add");
    ui.press(&format!("menu > {primitive}"));
    panel(ui).unwrap_or_else(|| panic!("Add > {primitive} opens the panel"))
}

/// A stroke dragged across the middle of the 3D view.
fn stroke(ui: &mut Ui) {
    let [x, y] = ui.middle();
    ui.touch(x - 30.0, y, false);
    for dx in [0.0, 20.0, 40.0, 60.0] {
        ui.touch(x - 30.0 + dx, y, true);
    }
    ui.touch(x + 30.0, y, false);
}

/// The problem the status line shows, if any.
fn problem(ui: &mut Ui) -> Option<String> {
    let status = ui.labels("status");
    status.into_iter().find(|l| l.starts_with("Problem:"))
}

/// Voxel Remesh run from the palette opens the panel in the view's lower left (the right
/// hand's), titled with the command and filled with the step's arguments: Resolution's slider
/// runs the step with its part; the part, pinned, is not shown.
#[test]
fn voxel_remesh_from_the_palette_opens_the_panel_with_its_step() {
    let mut ui = Ui::new();
    ball(&mut ui);
    assert!(panel(&mut ui).is_none(), "no panel before");
    remesh_from_palette(&mut ui, 32);
    let shown = panel(&mut ui).expect("the panel opens");
    assert_eq!(shown["title"], "Voxel remesh");
    let slider = widget(&mut ui, "Resolution");
    assert_eq!(slider["role"], "slider");
    let step = &slider["command"];
    assert_eq!(step["command"], "mesh.voxel_remesh");
    assert_eq!(
        (&step["args"]["resolution"], &step["args"]["part"]),
        (&json!(32), &json!("ball"))
    );
    let labels = ui.labels("adjust");
    assert!(!labels.contains(&"Part".to_string()), "pinned: {labels:?}");
    let (view, r) = (ui.rect("viewport"), rect(&shown));
    let near = |inside: f64, edge: f64| (0.0..40.0).contains(&(inside - edge).abs());
    assert!(
        near(r[0], view[0]) && r[2] < middle(view)[0],
        "on the left: {r:?} in {view:?}"
    );
    assert!(near(r[3], view[3]), "at the bottom: {r:?} in {view:?}");
}

/// Ctrl+R, the key bound to Voxel Remesh, opens the same panel for its step.
#[test]
fn the_key_bound_to_voxel_remesh_opens_the_panel_too() {
    let mut ui = Ui::new();
    ball(&mut ui);
    // The part keeps this cell size, so the key's run, which gives no resolution, is quick.
    ui.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    assert!(
        panel(&mut ui).is_none(),
        "a run not from the window's doors"
    );
    ui.tap("R", held("Ctrl"));
    assert_eq!(ui.logged("mesh.voxel_remesh").len(), 2, "the key ran it");
    let shown = panel(&mut ui).expect("the panel opens");
    assert_eq!(shown["title"], "Voxel remesh");
    let step = widget(&mut ui, "Resolution")["command"].clone();
    assert_eq!(step["args"]["part"], "ball", "the step's own part");
}

/// A resolution typed into the panel undoes the remesh and runs it again: still one step to
/// undo, the panel showing the new step, and one undo gives back the ball from before the
/// first remesh.
#[test]
fn a_changed_resolution_runs_again_as_one_undo_step() {
    let mut ui = Ui::new();
    ball(&mut ui);
    let before = ui.session.mesh("ball").positions;
    remesh_from_palette(&mut ui, 32);
    let undos = ui.state()["history.undos"].clone();
    type_into(&mut ui, "Resolution", "24", true);
    let runs = ui.logged("mesh.voxel_remesh");
    assert_eq!(runs.len(), 2, "run again: {runs:?}");
    assert_eq!(newest_resolution(&mut ui), 24, "the step run again");
    assert_eq!(ui.state()["history.undos"], undos, "one step");
    assert_eq!(resolution(&mut ui), 24, "the panel shows the new step");
    ui.ok("history.undo", json!({}));
    let back = ui.session.mesh("ball").positions;
    assert_eq!(back, before, "one undo: the ball before the first remesh");
}

/// The fields list the step's own arguments first, in the command's order, then the rest by
/// name (decision 0062, item 749): a remesh reads Resolution first, where by name Blur was.
#[test]
fn the_fields_list_the_step_s_own_arguments_in_the_command_s_order_then_the_rest() {
    let mut ui = Ui::new();
    ball(&mut ui);
    remesh_from_palette(&mut ui, 32);
    let fields = [
        "Resolution",
        "Keep groups",
        "Project",
        "Blur",
        "Close holes",
        "Voxel size",
    ];
    let labels = ui.labels("adjust");
    let shown: Vec<&str> = (labels.iter().map(String::as_str))
        .filter(|l| fields.contains(l))
        .collect();
    assert_eq!(shown, fields, "{labels:?}");
}

/// Add > Sphere passes no arguments and opens the panel with the sphere's settings (`A1`): its
/// form's density, Subdivisions, and Levels, not the other form's Segments and Rings (the
/// schema's `x-when`). Levels changed makes the sphere again as one undo step.
#[test]
fn a_sphere_from_add_shows_its_levels_and_density_and_a_change_makes_it_again_as_one_step() {
    let mut ui = Ui::new();
    let shown = add(&mut ui, "Sphere");
    assert_eq!(shown["title"], "Add sphere");
    let labels = ui.labels("adjust");
    let has = |label: &str| labels.iter().any(|l| l == label);
    assert!(has("Levels") && has("Subdivisions"), "{labels:?}");
    assert!(
        !has("Segments") && !has("Rings"),
        "the uv form's: {labels:?}"
    );
    let undos = ui.state()["history.undos"].clone();
    type_into(&mut ui, "Levels", "2", true);
    assert_eq!(ui.logged("primitive.sphere").len(), 2, "made again");
    assert_eq!(ui.state()["history.undos"], undos, "one step");
    let list = ui.ok("history.list", json!({ "limit": 1 }));
    assert_eq!(list["steps"][0]["args"]["levels"], 2, "{list}");
}

/// Add sphere's Form row reads its words as they are written: Cube and UV, an initialism in
/// capitals, not Uv; UV still chooses the word `uv`.
#[test]
fn a_spheres_form_row_reads_cube_and_uv() {
    let mut ui = Ui::new();
    add(&mut ui, "Sphere");
    let labels = ui.labels("adjust");
    let has = |label: &str| labels.iter().any(|l| l == label);
    assert!(has("Cube") && has("UV") && !has("Uv"), "{labels:?}");
    let uv = widget(&mut ui, "UV")["command"]["args"]["form"].clone();
    assert_eq!(uv, "uv", "the word sent");
}

/// The hints under Add sphere's Form and Axis rows quote none of the words their buttons show:
/// Form's (`"cube" or "uv".`) says nothing, as its next sentence is about leaving it out, and
/// Axis's says what the axis is, its initialism in capitals as a button's is (UV, decision 0025,
/// rule 13, amended for UI-finish). Its X, Y and Z, which have no doc of their own, say that
/// hint on hover.
#[test]
fn a_spheres_form_and_axis_rows_show_no_quoted_values() {
    let mut ui = Ui::new();
    add(&mut ui, "Sphere");
    let labels = ui.labels("adjust");
    let quoted: Vec<&String> = labels.iter().filter(|l| l.contains('"')).collect();
    assert!(quoted.is_empty(), "{quoted:?}");
    let left_out = labels.iter().filter(|l| l.starts_with("Left out"));
    assert_eq!(left_out.count(), 0, "{labels:?}");
    let axis = "The world axis through the UV form's poles.";
    assert!(labels.iter().any(|l| l == axis), "{labels:?}");
    assert_eq!(widget(&mut ui, "Y")["hint"], axis);
}

/// Add sphere's Center, a point, is three number fields side by side, X, Y and Z, not one
/// field typed as a list; a change of Y makes the sphere again with the whole point.
#[test]
fn a_spheres_center_is_three_fields_and_a_change_of_y_sends_the_whole_point() {
    let mut ui = Ui::new();
    add(&mut ui, "Sphere");
    let fields = ["X", "Y", "Z"].map(|axis| widget(&mut ui, &format!("Center {axis}")));
    assert!(
        fields.iter().all(|f| f["role"] == "text field"),
        "{fields:?}"
    );
    let [x, y, z] = fields.each_ref().map(rect);
    let side_by_side = x[2] <= y[0] && y[2] <= z[0] && (x[1] - z[1]).abs() < 1.0;
    assert!(side_by_side, "{x:?} {y:?} {z:?}");
    assert!(!ui.labels("adjust").contains(&"Center".to_string()));
    let r = ui.rect("adjust > Center Y");
    ui.tap_at(r[2] - 4.0, middle(r)[1]);
    let none = Modifiers::default();
    // Y shows 0.0 as the step has it.
    for _ in 0..3 {
        ui.tap("Backspace", none);
    }
    ui.input(&[Input::Text("0.5".into())], none);
    ui.tap("Return", none);
    assert_eq!(ui.logged("primitive.sphere").len(), 2, "made again");
    let list = ui.ok("history.list", json!({ "limit": 1 }));
    let center = list["steps"][0]["args"]["center"].as_array().cloned();
    let center: Vec<f64> = (center.unwrap_or_default().iter())
        .filter_map(Value::as_f64)
        .collect();
    assert_eq!(center, [0.0, 0.5, 0.0], "{list}");
}

/// Every slider of the form has its label beside its rail, whatever the label's length: Add
/// sphere's Subdivisions, the longer label, sits on one row as Levels does.
#[test]
fn a_spheres_subdivisions_and_levels_labels_sit_on_their_sliders_rows() {
    let mut ui = Ui::new();
    add(&mut ui, "Sphere");
    let button = rect(&widget(&mut ui, "Cube"));
    let one_row = 1.5 * (button[3] - button[1]);
    for name in ["Subdivisions", "Levels"] {
        let r = rect(&widget(&mut ui, name));
        assert!(r[3] - r[1] < one_row, "{name} on one row: {r:?}");
    }
}

/// A slider runs the step again when let go, not while it is dragged.
#[test]
fn a_slider_runs_again_when_let_go_only() {
    let mut ui = Ui::new();
    ball(&mut ui);
    remesh_from_palette(&mut ui, 40);
    let r = ui.rect("adjust > Resolution");
    let [x, y] = [r[0] + 20.0, middle(r)[1]];
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x - 30.0, y, true);
    assert_eq!(ui.logged("mesh.voxel_remesh").len(), 1, "not while dragged");
    ui.touch(x - 30.0, y, false);
    let runs = ui.logged("mesh.voxel_remesh");
    assert_eq!(runs.len(), 2, "let go: run again");
    assert_eq!(newest_resolution(&mut ui), 16, "at the rail's low end");
    assert_eq!(resolution(&mut ui), 16);
}

/// A change the engine refuses (a cell size too small for the grid) puts the old step back
/// with `history.redo`: the remesh as it was, the panel filled from it and the engine's reason
/// under the field, the status line left alone.
#[test]
fn a_refused_change_puts_the_old_step_back_and_says_why_under_its_field() {
    let mut ui = Ui::new();
    ball(&mut ui);
    remesh_from_palette(&mut ui, 32);
    let made = ui.session.mesh("ball").positions;
    let undos = ui.state()["history.undos"].clone();
    type_into(&mut ui, "Voxel size", "0.0001", false);
    assert_eq!(
        ui.logged("mesh.voxel_remesh").len(),
        1,
        "refused, not logged"
    );
    assert_eq!(ui.logged("history.redo").len(), 1, "the old step put back");
    assert_eq!(ui.session.mesh("ball").positions, made, "as it was");
    assert_eq!(ui.state()["history.undos"], undos);
    assert!(panel(&mut ui).is_some(), "the panel stays");
    assert_eq!(resolution(&mut ui), 32, "filled from the old step");
    let widgets = ui.widgets("adjust");
    let says = |w: &&Value| w["label"].as_str().is_some_and(|l| l.contains("2048"));
    // The last that says it: Resolution's hint, above, says 2048 too.
    let reason = widgets.iter().rev().find(says).expect("the reason shows");
    let field = widget(&mut ui, "Voxel size");
    assert!(rect(reason)[1] >= rect(&field)[3], "under its field");
    assert_eq!(problem(&mut ui), None, "the status line is left alone");
}

/// × closes the panel and F9 opens it again; Escape closes it only with the pen over it; an
/// undo closes it, and so does a stroke.
#[test]
fn a_stroke_an_undo_close_and_escape_over_it_close_the_panel() {
    let mut ui = Ui::new();
    ball(&mut ui);
    remesh_from_palette(&mut ui, 32);
    ui.press("adjust > Close");
    assert!(panel(&mut ui).is_none(), "× closes it");
    ui.tap("F9", Modifiers::default());
    assert!(panel(&mut ui).is_some(), "F9 opens it again");
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.tap("Escape", Modifiers::default());
    assert!(panel(&mut ui).is_some(), "Escape over the view leaves it");
    let [x, y] = middle(ui.rect("adjust"));
    ui.pen(x, y);
    ui.tap("Escape", Modifiers::default());
    assert!(panel(&mut ui).is_none(), "Escape over it closes it");
    ui.tap("F9", Modifiers::default());
    ui.tap("Z", held("Ctrl"));
    assert!(panel(&mut ui).is_none(), "an undo closes it");
    ui.tap("Y", held("Ctrl"));
    ui.tap("F9", Modifiers::default());
    assert!(
        panel(&mut ui).is_some(),
        "redone, its step is the newest again"
    );
    let undos = ui.state()["history.undos"].clone();
    stroke(&mut ui);
    assert_ne!(ui.state()["history.undos"], undos, "the stroke landed");
    assert!(panel(&mut ui).is_none(), "a stroke closes it");
}

/// F9 opens the panel again for the newest step; once a stroke is the newest, it opens nothing
/// and the status line says there is nothing to adjust.
#[test]
fn f9_opens_the_panel_again_or_says_nothing_to_adjust() {
    let mut ui = Ui::new();
    ball(&mut ui);
    remesh_from_palette(&mut ui, 32);
    ui.press("adjust > Close");
    ui.tap("F9", Modifiers::default());
    let shown = panel(&mut ui).expect("F9 opens it again");
    assert_eq!(shown["title"], "Voxel remesh");
    assert_eq!(resolution(&mut ui), 32);
    assert_eq!(problem(&mut ui), None);
    stroke(&mut ui);
    ui.tap("F9", Modifiers::default());
    assert!(panel(&mut ui).is_none(), "the stroke is the newest step");
    let none = Some("Problem: Nothing to adjust".to_string());
    assert_eq!(problem(&mut ui), none);
}

/// A tray's button not marked `adjust` and a view setting never open the panel: the scene
/// tray's level stepper runs an edit with a field (By), and Shift+P sets the floor. F9 then has
/// nothing to adjust. (Subdivide, marked, opens it: tests/layouts/detail.rs.)
#[test]
fn a_tray_stepper_and_a_view_setting_never_open_the_panel() {
    let mut ui = Ui::new();
    ball(&mut ui);
    ui.ok("level.subdivide", json!({}));
    ui.open_only(&["detail"]);
    ui.press("tray > ‹");
    assert_eq!(ui.logged("level.step").len(), 1, "the tray ran it");
    assert!(panel(&mut ui).is_none(), "a tray's run");
    let floor = ui.state()["view.floor"].clone();
    ui.tap("P", held("Shift"));
    assert_ne!(ui.state()["view.floor"], floor, "the key set the floor");
    assert!(panel(&mut ui).is_none(), "a view setting");
    ui.tap("F9", Modifiers::default());
    assert!(panel(&mut ui).is_none(), "nor F9 after a tray's run");
    let none = Some("Problem: Nothing to adjust".to_string());
    assert_eq!(problem(&mut ui), none);
}

/// Rule 8 (decision 0060, section 11): the panel's ×, its title (the fold) and the line saying
/// the step used the view each say what they do on hover. Assigning the visible faces a group
/// pins the view's camera, so its panel has that line; a tap on the title folds the panel to
/// it, kept by the layout, and another opens it.
#[test]
fn the_close_the_fold_and_the_view_line_say_what_they_do() {
    let mut ui = Ui::new();
    ball(&mut ui);
    let args = json!({ "select": { "by": "visible" } });
    ui.ok(
        "ui.palette",
        json!({ "form": "group.assign", "args": args }),
    );
    ui.press("palette > Assign face group");
    assert!(panel(&mut ui).is_some(), "the panel opens");
    let line = "Uses the view as it was when it ran";
    for label in ["Close", "Assign face group", line] {
        let hint = widget(&mut ui, label)["hint"].clone();
        let hint = hint.as_str().unwrap_or_default();
        assert!(hint.len() > 20, "{label} says what it does: {hint:?}");
    }
    ui.press("adjust > adjust");
    let folded = ui.host.lock().shell.layout.layout.folded.clone();
    assert!(folded.contains(&"adjust".to_string()), "{folded:?}");
    assert!(!ui.labels("adjust").contains(&line.to_string()), "folded");
    ui.press("adjust > adjust");
    assert!(
        ui.labels("adjust").contains(&line.to_string()),
        "open again"
    );
}
