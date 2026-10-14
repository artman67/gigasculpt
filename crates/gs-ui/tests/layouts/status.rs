//! The status line says how big the scene is (decision 0025, W2): its parts, and their points
//! and faces, the same totals `mesh.stats` gives, after every change. It also names the step
//! the last undo or redo took.

use serde_json::json;

use crate::rig::Ui;

/// The scene's size as the status line shows it.
fn scene(ui: &mut Ui) -> String {
    let labels = ui.labels("status");
    let found = labels.into_iter().find(|l| l.contains("part"));
    found.unwrap_or_else(|| panic!("the status line counts the scene"))
}

/// `mesh.stats`' totals over every part, written as the status line writes them.
fn totals(ui: &mut Ui) -> String {
    let stats = ui.ok("mesh.stats", json!({}));
    let parts = stats["parts"].as_array().cloned().unwrap_or_default();
    let sum = |key: &str| {
        parts
            .iter()
            .map(|p| p[key].as_u64().unwrap_or_default())
            .sum::<u64>()
    };
    let grouped = |n: u64| {
        let digits = n.to_string();
        let at = |i: usize| (i > 0 && (digits.len() - i).is_multiple_of(3)).then_some(',');
        (digits.chars().enumerate())
            .flat_map(|(i, d)| at(i).into_iter().chain([d]))
            .collect::<String>()
    };
    let what = if parts.len() == 1 { "part" } else { "parts" };
    format!(
        "{} {what}  {} points  {} faces",
        parts.len(),
        grouped(sum("vertices")),
        grouped(sum("faces"))
    )
}

#[test]
fn the_status_line_counts_the_scenes_parts_points_and_faces() {
    let mut ui = Ui::new();
    ui.settle();
    assert_eq!(scene(&mut ui), "No parts");
    ui.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    ui.settle();
    let one = scene(&mut ui);
    assert_eq!(one, totals(&mut ui));
    ui.ok("primitive.cube", json!({ "subdivisions": 8 }));
    ui.ok("level.subdivide", json!({ "part": "cube" }));
    ui.settle();
    let two = scene(&mut ui);
    assert!(two.starts_with("2 parts"), "{two}");
    assert_eq!(two, totals(&mut ui), "after a second part, subdivided");
    ui.ok("history.undo", json!({}));
    ui.settle();
    assert_eq!(scene(&mut ui), totals(&mut ui), "and after undo");
}

/// What the status line says the last undo or redo took, if anything.
fn moved(ui: &mut Ui) -> Option<String> {
    ui.settle();
    let labels = ui.labels("status");
    (labels.into_iter()).find(|l| l.starts_with("Undid") || l.starts_with("Redid"))
}

/// An undo or redo says on the status line which step it took, by the brush's name for a
/// stroke (session.state's `history.moved`), until the next new step.
#[test]
fn the_status_line_names_the_step_an_undo_or_redo_took() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let stroke = json!({ "brush": "Draw", "size": "20%", "samples": [{ "position": [0, 0, 1] }] });
    ui.ok("brush.stroke", stroke.clone());
    assert_eq!(moved(&mut ui), None, "a new step says nothing");
    ui.ok("history.undo", json!({}));
    assert_eq!(moved(&mut ui).as_deref(), Some("Undid Draw stroke"));
    ui.ok("history.redo", json!({}));
    assert_eq!(moved(&mut ui).as_deref(), Some("Redid Draw stroke"));
    ui.ok("history.undo", json!({}));
    ui.ok("brush.stroke", stroke);
    assert_eq!(moved(&mut ui), None, "a new step clears it");
}
