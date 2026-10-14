//! A control's slider whose value can be a word reads that word, not 0: the occlusion Radius
//! slider in the Look options reads "auto" while the radius is automatic, and its number once
//! one is set (decision 0025, section 4). A form's unset optional number reads "auto" too, and
//! the floor's Size at 0 "endless" (decision 0062).

use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::rig::{Rig, harness, window};

#[test]
fn the_occlusion_radius_slider_reads_auto_while_the_radius_is_automatic() {
    let mut h = harness(Rig::new());
    h.run();
    h.get_by_label("Look").click();
    h.run();
    let reads = |h: &egui_kittest::Harness<'_, Rig>, text: &str| {
        h.query_all_by_label(text).next().is_some()
    };
    assert!(reads(&h, "Radius  auto"), "automatic, it reads auto");
    assert!(!reads(&h, "Radius  0.00"), "not 0");
    let set = json!({ "occlusion_radius": 0.5 });
    h.state_mut().ok(&window(), "view.display", set);
    h.run();
    assert!(reads(&h, "Radius  0.50"), "set, it reads the number");
    let auto = json!({ "occlusion_radius": "auto" });
    h.state_mut().ok(&window(), "view.display", auto);
    h.run();
    assert!(reads(&h, "Radius  auto"), "and auto again");
}

/// Whether a widget labelled `text` shows.
fn reads(h: &egui_kittest::Harness<'_, Rig>, text: &str) -> bool {
    h.query_all_by_label(text).next().is_some()
}

/// A form's optional number with no default reads "auto" until moved, and a whole number reads
/// without decimals (decision 0062): the sphere's Subdivisions unset, which the engine settles,
/// and its Levels at 0.
#[test]
fn a_form_s_unset_optional_number_reads_auto_and_a_whole_one_has_no_decimals() {
    let mut h = harness(Rig::new());
    let asked = json!(["subdivisions", "levels"]);
    let form = json!({ "form": "primitive.sphere", "ask": asked });
    h.state_mut().ok(&window(), "ui.palette", form);
    h.run();
    assert!(reads(&h, "Subdivisions  auto"), "unset, it reads auto");
    assert!(reads(&h, "Levels  0"), "a whole number");
}

/// The floor's Size at 0 reads "endless", as its hint says 0 is (decision 0062).
#[test]
fn the_floor_s_size_at_0_reads_endless() {
    let mut h = harness(Rig::new());
    // The view buttons wait for a part.
    h.state_mut().ok(&window(), "primitive.sphere", json!({}));
    let on = json!({ "floor": true });
    h.state_mut().ok(&window(), "view.display", on);
    h.run();
    // The header's pen end is drawn twice, once unseen to measure it; the seen one is last.
    let floor = h
        .query_all_by_label("Floor")
        .last()
        .expect("the Floor button");
    floor.click_secondary();
    h.run();
    assert!(reads(&h, "Size  endless"), "0 reads endless");
    let sized = json!({ "size": 2.0 });
    h.state_mut().ok(&window(), "view.floor", sized);
    h.run();
    assert!(reads(&h, "Size  2.00"), "a size reads its number");
}
