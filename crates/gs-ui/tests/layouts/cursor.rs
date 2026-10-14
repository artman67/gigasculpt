//! The cursor is where the eye is: it shows what a held modifier key does
//! (and holding one never moves the view), the brush's size, strength, Softness and symmetry,
//! and the Softness ring moves as Softness does (the falloff, named Softness everywhere:
//! decision 0060, amended for UI-finish).
//! Holding O and moving sets Softness, as S and U set size and strength. Its lines stay inside
//! the 3D view at each of its edges.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::rig::{Ui, held, key};

/// The labels beside the pen.
fn cursor(ui: &mut Ui) -> Vec<String> {
    ui.labels("viewport")
}

#[test]
fn held_modifiers_show_on_the_cursor() {
    let mut ui = Ui::new();
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let view = ui.describe()["viewport"]["rect"].clone();
    for (modifier, says) in [
        ("Shift", "Shift: Smooth"),
        ("Ctrl", "Ctrl: Mask Pen"),
        ("Alt", "Alt: Subtract"),
    ] {
        ui.input(&[], held(modifier));
        let labels = cursor(&mut ui);
        assert!(labels.iter().any(|l| l == says), "{modifier}: {labels:?}");
        let moved = ui.describe()["viewport"]["rect"].clone();
        assert_eq!(moved, view, "holding {modifier} never moves the view");
        let status = ui.labels("status");
        assert!(
            status.iter().any(|l| l == says),
            "the status line too: {status:?}"
        );
    }
    ui.input(&[], Modifiers::default());
    let labels = cursor(&mut ui);
    assert!(
        !labels.iter().any(|l| l.contains(": ")),
        "nothing held, nothing said: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("Draw  64 px  40%")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.starts_with("Softness +0  No symmetry")),
        "{labels:?}"
    );
}

/// On a brush whose stored mode is Subtract a stroke carves and Alt flips it back to adding
/// (gs-core `stroke/mod.rs`), so the cursor and the status line say Add while Alt is held. The
/// Brush tray's Add and Subtract set the stored mode through `brush.set`, for the brush only.
#[test]
fn alt_on_a_subtract_brush_says_add() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "settings": { "mode": "subtract" } }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.input(&[], held("Alt"));
    let (labels, status) = (cursor(&mut ui), ui.labels("status"));
    assert!(labels.iter().any(|l| l == "Alt: Add"), "{labels:?}");
    assert!(status.iter().any(|l| l == "Alt: Add"), "{status:?}");
    ui.input(&[], Modifiers::default());
    ui.press("brush > Add");
    assert_eq!(ui.state()["brush.mode"], "add");
    let last = ui.logged("brush.set").pop().unwrap_or_default();
    assert_eq!(
        last["role"], "human",
        "the press is a logged brush.set: {last}"
    );
    ui.pen(x, y);
    ui.input(&[], held("Alt"));
    let labels = cursor(&mut ui);
    assert!(labels.iter().any(|l| l == "Alt: Subtract"), "{labels:?}");
    ui.input(&[], Modifiers::default());
    ui.press("brush > Subtract");
    assert_eq!(ui.state()["brush.mode"], "subtract");
    ui.ok("brush.set", json!({ "brush": "Clay" }));
    assert_eq!(
        ui.state()["brush.mode"],
        "add",
        "the mode is the brush's own"
    );
}

/// On a color brush that does not sculpt, Alt paints the Secondary color instead of the Primary
/// (decision 0039, section 7), so the cursor and the status line say Secondary while Alt is held.
#[test]
fn alt_on_a_color_brush_says_secondary() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Paint" }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.input(&[], held("Alt"));
    let (labels, status) = (cursor(&mut ui), ui.labels("status"));
    assert!(labels.iter().any(|l| l == "Alt: Secondary"), "{labels:?}");
    assert!(status.iter().any(|l| l == "Alt: Secondary"), "{status:?}");
}

/// With the gizmo tool on, a press works the gizmo, never a stroke (decision 0027, sections 11
/// and 14), so the keys say what they do to its drag: Ctrl copies, Alt moves only the gizmo
/// (and wins over Ctrl), Shift snaps moves and turns. With no part there is no handle under
/// the pen; gs-ui `tests/sculpt/gizmo.rs` has what they say on each kind of handle.
#[test]
fn with_the_gizmo_tool_on_held_keys_say_what_they_do_to_a_drag() {
    let mut ui = Ui::new();
    ui.ok("ui.gizmo", json!({ "tool": true }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let both = |a: &str, b: &str| Modifiers {
        ctrl: true,
        shift: b == "Shift",
        alt: a == "Alt" || b == "Alt",
        ..Modifiers::default()
    };
    for (keys, says) in [
        (held("Ctrl"), "Ctrl: Copy"),
        (held("Alt"), "Alt: Gizmo only"),
        (held("Shift"), "Shift: Snap moves and turns"),
        (
            both("Ctrl", "Shift"),
            "Ctrl+Shift: Copy, snap moves and turns",
        ),
        (both("Ctrl", "Alt"), "Ctrl+Alt: Gizmo only"),
    ] {
        ui.input(&[], keys);
        let (labels, status) = (cursor(&mut ui), ui.labels("status"));
        assert!(labels.iter().any(|l| l == says), "{says}: {labels:?}");
        assert!(status.iter().any(|l| l == says), "{says}: {status:?}");
    }
}

/// The cursor's lines stay inside the 3D view, never over the status line, a tray or the
/// header: in a 1400 by 860 window, with the pen a few points inside each of the view's edges,
/// both lines (the brush's, its Softness and symmetry) lie inside it, and near its foot they end
/// above the status line.
#[test]
fn the_cursors_lines_stay_inside_the_view_at_its_edges() {
    let mut ui = Ui::new();
    ui.host.lock().resize([1400, 860], 1.0);
    ui.settle();
    let view = ui.rect("viewport");
    let [x0, y0, x1, y1] = view;
    let [mx, my] = [f64::midpoint(x0, x1), f64::midpoint(y0, y1)];
    let status = ui.rect("status");
    for (edge, x, y) in [
        ("foot", mx, y1 - 4.0),
        ("right", x1 - 4.0, my),
        ("top", mx, y0 + 4.0),
        ("left", x0 + 4.0, my),
    ] {
        ui.pen(x, y);
        let widgets = ui.widgets("viewport");
        let line = |w: &&serde_json::Value| {
            let label = w["label"].as_str().unwrap_or_default();
            label.starts_with("Draw  ") || label.starts_with("Softness +")
        };
        let rects: Vec<[f64; 4]> = (widgets.iter().filter(line))
            .map(|w| std::array::from_fn(|i| w["rect"][i].as_f64().unwrap_or_default()))
            .collect();
        assert_eq!(rects.len(), 2, "{edge}: the brush's two lines");
        for r in rects {
            let inside = r[0] >= x0 && r[1] >= y0 && r[2] <= x1 && r[3] <= y1;
            assert!(inside, "{edge}: {r:?} inside the view {view:?}");
            if edge == "foot" {
                assert!(r[3] <= status[1], "{r:?} ends above {status:?}");
            }
        }
    }
}

#[test]
fn the_falloff_ring_moves_in_as_falloff_rises_and_o_sets_it() {
    let mut ui = Ui::new();
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let ring = |ui: &mut Ui, name: &str| {
        let widgets = ui.widgets("viewport");
        let w = widgets
            .iter()
            .find(|w| w["label"] == name)
            .cloned()
            .unwrap_or_default();
        w["rect"][2].as_f64().unwrap_or_default() - w["rect"][0].as_f64().unwrap_or_default()
    };
    // Where the bent distance d^(2^(-f/50)) reaches one half: 0.5, 0.25, 0.707 of the size.
    for (falloff, ratio) in [
        (0.0, 0.5),
        (50.0, 0.25),
        (-50.0, std::f64::consts::FRAC_1_SQRT_2),
    ] {
        ui.ok("brush.set", json!({ "falloff": falloff }));
        let (size, inner) = (ring(&mut ui, "Size ring"), ring(&mut ui, "Softness ring"));
        assert!((size - 128.0).abs() < 0.01, "64 pixels out: {size}");
        assert!(
            (inner / size - ratio).abs() < 0.01,
            "Softness {falloff}: {inner} of {size}"
        );
    }
    // O held past the activation time, then moved 100 points right: a quarter of the range
    // (200 in 400 points).
    let at = |x: f64| Input::MouseMove { x, y: 300.0 };
    ui.input(&[at(400.0)], Modifiers::default());
    ui.input(
        &[key("O", true, Modifiers::default())],
        Modifiers::default(),
    );
    ui.wait(0.3);
    ui.input(&[at(500.0)], Modifiers::default());
    let shown = cursor(&mut ui);
    assert!(
        shown.iter().any(|l| l.starts_with("Softness +0")),
        "shown while held: {shown:?}"
    );
    ui.input(
        &[key("O", false, Modifiers::default())],
        Modifiers::default(),
    );
    let falloff = ui.state()["brush.falloff"].as_f64().unwrap_or_default();
    assert!((falloff - 0.0).abs() < 1e-3, "-50 and 50 more: {falloff}");
}
