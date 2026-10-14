//! Descriptions on hover (interface.gesture-map-widgets, its first row): with Ctrl held over a
//! button, a box at the pointer says what the button's command does, the command's address and
//! the key bound to it; with Ctrl+Alt, the command's whole docs. Without them nothing shows,
//! but over a brush's button, whose box leads with the keys that pick the brush.

use gs_platform::Modifiers;
use serde_json::json;

use crate::region::middle;
use crate::rig::{Ui, center, held};

#[test]
fn ctrl_over_a_button_describes_it_and_ctrl_alt_shows_the_whole_docs() {
    let mut ui = Ui::new();
    ui.open_only(&["detail"]);
    let button = ui
        .widgets("tray")
        .into_iter()
        .find(|w| w["label"] == "Subdivide");
    let button = button.expect("the scene tray has Subdivide");
    let command = button["command"]["command"].as_str().unwrap_or_default();
    let key = button["key"].as_str().unwrap_or_default();
    assert_eq!((command, key), ("level.subdivide", "Ctrl+D"));
    let catalog = ui.session.engine.catalog().clone();
    let info = catalog.commands.iter().find(|c| c.name == command);
    let info = info.expect("a registered command");
    assert_ne!(info.docs, info.summary, "docs longer than their first line");

    let [x, y] = middle(ui.rect("tray > Subdivide"));
    ui.pen(x, y);
    assert!(ui.labels("description").is_empty(), "nothing without Ctrl");
    ui.input(&[], held("Ctrl"));
    let shown = ui.labels("description");
    assert!(shown.contains(&info.summary), "{shown:?} says what it does");
    assert!(shown.iter().any(|l| l.contains(command)), "its address");
    assert!(shown.iter().any(|l| l.contains(key)), "its key");
    assert!(!shown.contains(&info.docs), "not yet the whole docs");

    let both = Modifiers {
        ctrl: true,
        alt: true,
        ..Modifiers::default()
    };
    ui.input(&[], both);
    let longer = ui.labels("description");
    assert!(longer.contains(&info.docs), "{longer:?} is the whole docs");

    ui.input(&[], Modifiers::default());
    assert!(ui.labels("description").is_empty(), "gone with Ctrl let go");
}

/// What the box says with the pen at (`x`, `y`) in pixels and `keys` held.
fn over(ui: &mut Ui, [x, y]: [f64; 2], keys: Modifiers) -> Vec<String> {
    ui.pen(x, y);
    ui.input(&[], keys);
    ui.labels("description")
}

/// Interface.tooltips: the pointer on a brush's button shows at once, with no key held, the
/// keys that pick the brush first, then its name and its hint; with Ctrl they lead the
/// description in place of its key line. A Smooth brush's button runs `brush.slot` and its key
/// `brush.set`, and Shift held strokes with it; a key you bind shows over the built-in one.
#[test]
fn hovering_a_brush_shows_the_keys_that_pick_it_first() {
    let mut ui = Ui::new();
    let none = Modifiers::default();
    let face = ui.face("Build");
    let hint = face["hint"].as_str().unwrap_or_default().to_string();
    let shown = over(&mut ui, center(&face["rect"]), none);
    assert_eq!(shown, ["B S T", "Draw", &hint], "the shelf's Build button");
    ui.open_family("Smooth");
    let smooth = middle(ui.rect("family > Smooth"));
    let shown = over(&mut ui, smooth, none);
    assert_eq!(shown[..2], ["B S M or Shift held", "Smooth"], "{shown:?}");
    let ctrl = over(&mut ui, smooth, held("Ctrl"));
    assert_eq!(ctrl[0], "B S M or Shift held", "{ctrl:?}");
    assert!(ctrl.iter().any(|l| l.contains("brush.slot")), "{ctrl:?}");
    assert!(!ctrl.iter().any(|l| l.starts_with("Key: ")), "{ctrl:?}");
    let k = json!({ "bind": "K", "command": "brush.set", "args": { "brush": "Smooth" } });
    ui.ok("ui.keymap", k);
    let rebound = over(&mut ui, smooth, none);
    assert_eq!(rebound[0], "K or Shift held", "{rebound:?}");
}

/// The shipped sets whose controls still wait for their hints, each until the build step that
/// passes over it writes them (decision 0060, section 18): the list only shrinks, and is empty
/// after R3.
const WAITING: [&str; 0] = [];

/// Decision 0060, section 8's rule 8 and section 18: every shipped control has a hint, but those
/// of the sets on the waiting list. A name on the list is a shipped set that still lacks one, so
/// a step that writes a set's hints takes it off.
#[test]
fn every_control_has_a_hint() {
    let all = gs_ui::Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    let hinted = |c: &&gs_ui::Control| c.hint.as_deref().is_some_and(|h| !h.trim().is_empty());
    for name in WAITING {
        let set = all.controls.sets.iter().find(|s| s.name == name);
        let set = set.unwrap_or_else(|| panic!("{name} is no set: take it off the waiting list"));
        assert!(
            !set.controls.iter().all(|c| hinted(&c)),
            "{name} has every hint: take it off the waiting list"
        );
    }
    let missing: Vec<String> = (all.controls.sets.iter())
        .filter(|s| !WAITING.contains(&s.name.as_str()))
        .flat_map(|s| {
            let bare = s.controls.iter().filter(|c| !hinted(c));
            bare.map(|c| format!("{} > {}", s.name, c.label))
        })
        .collect();
    assert!(missing.is_empty(), "controls with no hint: {missing:?}");
}

/// Every shipped control's label and short label draw in the window's font, none as the box
/// of a glyph it lacks (the Go font has no ⋯, U+22EF).
#[test]
fn every_label_draws_in_the_font() {
    let ctx = egui::Context::default();
    ctx.set_fonts(gs_ui::fonts());
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    // Where each glyph of `text` sits in the font's picture; every glyph the font lacks sits
    // where the box does (egui's `has_glyphs` says no to every glyph of a one-font family).
    let places = |text: &str| {
        let font = egui::FontId::proportional(14.0);
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.into(), font, egui::Color32::WHITE));
        let glyphs = galley.rows.iter().flat_map(|r| r.row.glyphs.iter());
        glyphs.map(|g| g.uv_rect).collect::<Vec<_>>()
    };
    let lacked = places("\u{E000}")[0];
    let all = gs_ui::Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    let missing: Vec<&String> = (all.controls.sets.iter())
        .flat_map(|s| &s.controls)
        .flat_map(|c| [Some(&c.label), c.short.as_ref()].into_iter().flatten())
        .filter(|l| places(l).contains(&lacked))
        .collect();
    assert!(
        missing.is_empty(),
        "labels the font cannot draw: {missing:?}"
    );
}
