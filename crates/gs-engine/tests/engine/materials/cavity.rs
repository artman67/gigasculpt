//! The cavity keys (decision 0043, step (c)): a standard material's alone.

use gs_testkit::code;
use serde_json::{Value, json};

use super::values;
use crate::support::agent;

/// Each cavity key `material.set` takes (decision 0043, step (c)), a value other than its
/// default, and how `material.list` reports it.
pub(super) fn cavity_edits() -> Vec<(&'static str, Value, Value)> {
    vec![
        ("cavity_intensity", json!(0.5), json!(0.5)),
        ("cavity_radius", json!(2.0), json!(2.0)),
        ("cavity_colorize", json!(0.25), json!(0.25)),
        ("cavity_color", json!("#ff0000"), json!([1.0, 0.0, 0.0])),
    ]
}

/// Basic Material's place: after the thirteen MatCaps, Flat Color and Fast Shader. A wrong
/// place shows as values that are not the ones `material.set` on Basic Material reports.
const BASIC: usize = 15;

/// The cavity settings are a standard material's alone (decision 0043, step (c)):
/// `material.list` reports the four keys for every material, neutral unless set. On Basic
/// Material each key changes alone, one palette step a call, a value out of range is refused at
/// its key, and `shipped` puts the neutral settings back. On a MatCap look each key is refused
/// with `E_ARGS_INVALID` at its key and changes nothing (Flat Color refuses them with every
/// other key).
#[test]
fn cavity_keys_are_a_standard_materials_alone() {
    let mut s = agent();
    let neutral = [
        ("cavity_intensity", json!(1.0)),
        ("cavity_radius", json!(1.0)),
        ("cavity_colorize", json!(0.0)),
        ("cavity_color", json!([0.0, 0.0, 0.0])),
    ];
    let list = s.query("material.list", json!({}));
    for m in list["looks"].as_array().into_iter().flatten() {
        for (key, value) in &neutral {
            assert_eq!(&m["values"][key], value, "{}: {key}", m["name"]);
        }
    }
    for (key, given, reported) in cavity_edits() {
        let mut want = values(&mut s, BASIC);
        want[key] = reported;
        let args = json!({ "material": "basic-material", key: given });
        let set = s.query("material.set", args);
        assert_eq!(set["material"]["values"], want, "{key}: only it changed");
    }
    for (key, wrong) in [
        ("cavity_intensity", 3.0),
        ("cavity_radius", 0.125),
        ("cavity_colorize", -0.5),
    ] {
        let e = s.err(
            "material.set",
            json!({ "material": "basic-material", key: wrong }),
        );
        let at = e.path.clone().unwrap_or_default();
        assert_eq!(
            (code(e.code), at),
            ("E_ARGS_INVALID".into(), format!("/{key}"))
        );
    }
    let before = s.query("material.list", json!({}));
    for (key, given, _) in cavity_edits() {
        let e = s.err("material.set", json!({ "material": "red-wax", key: given }));
        let said = format!("{} {}", e.path.unwrap_or_default(), e.message);
        assert_eq!(code(e.code), "E_ARGS_INVALID", "{said}");
        assert!(said.contains(key) && said.contains("standard"), "{said}");
    }
    assert_eq!(
        s.query("material.list", json!({})),
        before,
        "a refusal changes nothing"
    );
    let shipped = json!({ "material": "basic-material", "shipped": true });
    let back = s.query("material.set", shipped)["material"]["values"].clone();
    for (key, value) in &neutral {
        assert_eq!(&back[key], value, "shipped: {key}");
    }
    let mut steps = 0;
    while s.run("material.undo", json!({})).is_ok() {
        steps += 1;
    }
    assert_eq!(steps, 5, "one palette step a call");
}
