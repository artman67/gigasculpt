//! Voxel mode in the window (decision 0002, amended 2026-10-12): the Mesh menu's Voxel mode row
//! (decision 0060) shows the active part's switch and flips it. With it on and no mask, Ctrl alone
//! dragged on empty canvas runs `mesh.voxel_update`, a logged call that replays; with the Gizmo
//! tool on the same drag is the gizmo's and remeshes nothing; with the switch off the engine
//! refuses the update, nothing is logged, and the status line says why.

use gs_platform::Modifiers;
use gs_protocol::{Outcome, Role, ScriptLine};
use gs_testkit::Session;
use gs_ui::Keymap;
use serde_json::{Value, json};

use crate::background::{problem, ran_since};
use crate::region;
use crate::rig::{Window, log_path, pen_at};

const CTRL: Modifiers = Modifiers {
    ctrl: true,
    shift: false,
    alt: false,
    logo: false,
};

/// One frame of the window with `held` held, after the pen sample `sample` if any, running
/// what the interface asked for as the window does: a refused call shows on the status line
/// and is not logged.
fn frame(w: &mut Window, sample: Option<gs_protocol::PenSample>, held: Modifiers) {
    if let Some(s) = sample {
        w.pen(s);
    }
    let steps = {
        let mut studio = w.host.lock();
        studio.input(&[], held);
        studio.frame().unwrap_or_else(|e| panic!("{e}"));
        studio.shell.take_commands()
    };
    for step in steps {
        let done = w.call_as(Role::Human, &step.command, step.args.clone());
        if let Outcome::Error(e) = done.outcome {
            w.host.lock().shell.refused(Some(step), &e);
        }
    }
}

/// A drag of the tip from `from` to `to` (a tap when they are the same) with `held` held, one
/// frame per sample, then lifted.
fn drag(w: &mut Window, from: [f32; 2], to: [f32; 2], held: Modifiers) {
    frame(w, None, held);
    frame(w, Some(pen_at(0, from[0], from[1], 0.0)), held);
    for k in 0..5u8 {
        let f = f32::from(k) / 4.0;
        let [x, y] = [0, 1].map(|i| from[i] + (to[i] - from[i]) * f);
        frame(w, Some(pen_at(4 * u64::from(k) + 4, x, y, 0.5)), held);
    }
    frame(w, Some(pen_at(24, to[0], to[1], 0.0)), held);
}

/// The header's Mesh menu opened, or shut with `open` false, and drawn.
fn mesh_menu(w: &mut Window, open: bool) {
    let open = if open { json!("mesh") } else { json!(false) };
    w.ok("ui.menu", json!({ "open": open }));
    frame(w, None, Modifiers::default());
    frame(w, None, Modifiers::default());
}

/// The Mesh menu's Voxel mode row as `ui.describe` reports it; the menu is shut again after.
fn button(w: &mut Window) -> Value {
    mesh_menu(w, true);
    let d = w.ok("ui.describe", json!({}));
    mesh_menu(w, false);
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let menu = panels.iter().find(|p| p["id"] == "menu");
    let widgets = menu.and_then(|p| p["widgets"].as_array().cloned());
    let found = (widgets.unwrap_or_default().into_iter()).find(|x| x["label"] == "Voxel mode");
    found.unwrap_or_else(|| panic!("the Mesh menu shows Voxel mode"))
}

/// A tap of the pen on the Mesh menu's Voxel mode row: the calls it logged.
fn tap(w: &mut Window) -> Vec<(String, Value)> {
    mesh_menu(w, true);
    let r = w.rect("menu > Voxel mode");
    let at = region::middle(r).map(|c| c as f32);
    let from = w.logged().len();
    drag(w, at, at, Modifiers::default());
    ran_since(w, from)
}

#[test]
fn the_voxel_mode_button_flips_the_switch_and_a_ctrl_drag_updates_unless_the_gizmo_has_it() {
    let log = log_path("voxel-mode");
    let mut w = Window::sized(Some(log.clone()), Keymap::default(), [1800, 1100], 1.0);
    w.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    w.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    let rect = w.view_rect();
    let corner = [rect[0] + 30.0, rect[3] - 30.0];
    let across = [corner[0] + 80.0, corner[1]];
    let off = button(&mut w);
    assert_eq!(off["selected"], json!(false), "off: {off}");
    assert_eq!(off["command"]["command"], "mesh.voxel_mode", "{off}");

    // Off: the update is refused, nothing runs, and the status line says why.
    let (hash, start) = (w.hash(), w.logged().len());
    drag(&mut w, corner, across, CTRL);
    let ran = ran_since(&w, start);
    assert!(ran.is_empty(), "nothing runs: {ran:?}");
    assert_eq!(w.hash(), hash, "nothing changed");
    let said = problem(&w);
    assert!(said.contains("'sphere' is not in Voxel mode"), "{said}");

    // A tap turns it on, and it lights. The log pins the part, as for every part command, and
    // the hole closing it used (decision 0052 as amended).
    let ran = tap(&mut w);
    assert_eq!(
        ran,
        [(
            "mesh.voxel_mode".to_string(),
            json!({ "on": true, "part": "sphere", "close_holes": 1 })
        )],
        "the tap"
    );
    assert_eq!(button(&mut w)["selected"], json!(true), "lit while on");
    // A Mesh menu run opens the adjust panel in the view's lower left, where the drag starts:
    // closed, so the drag lands on the canvas.
    w.ok("ui.adjust", json!({ "open": false }));

    // On: the drag updates the part, a logged call.
    let (hash, start) = (w.hash(), w.logged().len());
    drag(&mut w, corner, across, CTRL);
    let ran = ran_since(&w, start);
    let names: Vec<&str> = ran.iter().map(|l| l.0.as_str()).collect();
    assert_eq!(names, ["mesh.voxel_update"], "it remeshes");
    assert_ne!(w.hash(), hash, "a new surface");

    // With the Gizmo tool on, the same drag is the gizmo's: it remeshes nothing.
    w.ok("ui.gizmo", json!({ "tool": true }));
    let (hash, start) = (w.hash(), w.logged().len());
    drag(&mut w, corner, across, CTRL);
    let ran = ran_since(&w, start);
    assert!(
        !ran.iter()
            .any(|l| l.0.starts_with("mesh.") || l.0.starts_with("mask.")),
        "the gizmo has it: {ran:?}"
    );
    assert_eq!(w.hash(), hash, "nothing remeshed");
    w.ok("ui.gizmo", json!({ "tool": false }));

    // A second tap turns it off.
    let ran = tap(&mut w);
    assert_eq!(
        ran,
        [(
            "mesh.voxel_mode".to_string(),
            json!({ "on": false, "part": "sphere" })
        )],
        "the tap"
    );
    assert_eq!(button(&mut w)["selected"], json!(false), "dark while off");

    // The log replays to the same scene without the window.
    let scene = w.hash();
    let mut replay = Session::new();
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("{e}"));
    for line in text.lines() {
        if let Ok(ScriptLine::Record(r)) = ScriptLine::parse(line) {
            replay.request(r.request().unwrap_or_else(|e| panic!("{e}")));
        }
    }
    let replayed = replay.query("mesh.hash", json!({}))["hash"].clone();
    assert_eq!(replayed, json!(scene), "the log replays to the same scene");
}
