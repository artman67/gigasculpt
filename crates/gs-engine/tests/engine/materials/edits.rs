//! Copies and edits (decision 0043, steps (a) and (b)): copying, renaming and deleting with
//! their results and refusals, and `material.set`: each key, what it refuses, a built-in's
//! shipped settings, and the palette's undo of an edit leaving the sculpt.

use gs_protocol::{Outcome, Request, RunParams};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use super::{look, names, refused, values};
use crate::support::{agent, stroke};

#[test]
fn copy_rename_and_delete_report_and_refuse() {
    let mut s = agent();
    let copy = s.query("material.copy", json!({}));
    let made = &copy["material"];
    assert_eq!(
        (
            &made["id"],
            &made["name"],
            &made["title"],
            &made["built_in"]
        ),
        (
            &json!(14),
            &json!("gray-copy"),
            &json!("MatCap Gray copy"),
            &json!(false)
        ),
        "a copy of the active material, after the user materials: {copy}"
    );
    assert_eq!(
        copy["active"],
        json!("gray"),
        "copying does not choose the copy"
    );
    let second = s.query("material.copy", json!({ "material": "gray" }));
    assert_eq!(second["material"]["name"], json!("gray-copy-2"));
    let own = json!({ "material": "#2", "name": "my-wax", "title": "My Wax" });
    assert_eq!(s.query("material.copy", own)["material"]["id"], json!(16));
    assert_eq!(
        names(&mut s)[13..16],
        ["gray-copy", "gray-copy-2", "my-wax"]
    );

    for (name, wanted) in [
        ("red-wax", "E_PRECONDITION"),
        ("#7", "E_ARGS_INVALID"),
        ("@x", "E_ARGS_INVALID"),
        ("main", "E_ARGS_INVALID"),
        (" wax", "E_ARGS_INVALID"),
    ] {
        let (got, message) = refused(&mut s, "material.copy", json!({ "name": name }));
        assert_eq!(got, wanted, "copy named {name:?}: {message}");
    }
    let (got, _) = refused(&mut s, "material.rename", json!({ "material": "my-wax" }));
    assert_eq!(got, "E_ARGS_INVALID", "neither a name nor a title");
    let taken = json!({ "material": "my-wax", "name": "bone" });
    assert_eq!(
        refused(&mut s, "material.rename", taken).0,
        "E_PRECONDITION"
    );
    let renamed = s.query(
        "material.rename",
        json!({ "material": "#16", "name": "wax", "title": "Wax" }),
    );
    assert_eq!(
        (&renamed["material"]["id"], &renamed["material"]["name"]),
        (&json!(16), &json!("wax"))
    );
    let built_in = json!({ "material": "red-wax", "title": "Red" });
    assert_eq!(
        s.query("material.rename", built_in)["material"]["title"],
        json!("Red")
    );

    let (got, message) = refused(&mut s, "material.delete", json!({ "material": "red-wax" }));
    assert_eq!(got, "E_PRECONDITION", "built-in materials stay: {message}");
    assert_eq!(
        refused(&mut s, "material.delete", json!({ "material": "wox" })).0,
        "E_NOT_FOUND"
    );
    s.ok("material.select", json!({ "name": "wax" }));
    let deleted = s.query("material.delete", json!({ "material": "wax" }));
    assert_eq!(deleted["material"]["name"], json!("wax"), "{deleted}");
    assert_eq!(
        deleted["active"],
        json!("gray"),
        "the first material is active again"
    );
    assert_eq!(look(&mut s), json!("gray"));
    assert!(!names(&mut s).contains(&"wax".to_string()));
}

/// `material.rename` and `material.delete` take the active material when `material` is left
/// out, as `material.copy` and `material.set` do, so the window's Rename and Delete rows run
/// them with no arguments (decision 0043, step (d)). Delete's dry run is refused on a built-in,
/// with why, so the window greys it there.
#[test]
fn rename_and_delete_take_the_active_material() {
    let mut s = agent();
    let dry = |s: &mut Session, args: Value| {
        s.request(Request::Run(RunParams::new("material.delete", args).dry()))
            .outcome
    };
    let greyed = dry(&mut s, json!({}));
    let Outcome::Error(e) = greyed else {
        panic!("a dry run of Delete on MatCap Gray is refused: {greyed:?}");
    };
    assert_eq!(code(e.code), "E_PRECONDITION", "{}", e.message);
    assert!(e.message.contains("built in"), "{}", e.message);

    s.ok("material.copy", json!({ "name": "mine" }));
    s.ok("material.select", json!({ "name": "mine" }));
    assert!(
        matches!(dry(&mut s, json!({})), Outcome::Ok(_)),
        "a copy can go"
    );
    let renamed = s.query(
        "material.rename",
        json!({ "name": "ours", "title": "Ours" }),
    );
    assert_eq!(
        (&renamed["material"]["id"], &renamed["material"]["title"]),
        (&json!(14), &json!("Ours")),
        "{renamed}"
    );
    let deleted = s.query("material.delete", json!({}));
    assert_eq!(deleted["material"]["name"], json!("ours"), "{deleted}");
    assert_eq!(deleted["active"], json!("gray"));
    assert!(!names(&mut s).contains(&"ours".to_string()));
}

/// Every key `material.set` takes, a value for it that MatCap Red Wax does not have, and how
/// `material.list` reports it: colors linear, a `"#rrggbb"` one decoded from sRGB. The values
/// are exact in 32 bits, so the report reads them back unchanged.
pub(super) fn edits() -> Vec<(&'static str, Value, Value)> {
    let same = |v: Value| (v.clone(), v);
    let pairs = [
        ("base_color", (json!("#000000"), json!([0.0, 0.0, 0.0]))),
        ("base_metalness", same(json!(0.5))),
        ("specular_weight", same(json!(0.25))),
        ("specular_color", same(json!([0.5, 0.25, 0.125]))),
        ("specular_roughness", same(json!(0.75))),
        ("specular_ior", same(json!(2.5))),
        ("coat_weight", same(json!(0.5))),
        ("coat_color", same(json!([0.5, 0.75, 1.0]))),
        ("coat_roughness", same(json!(0.125))),
        ("coat_ior", same(json!(1.25))),
        ("subsurface_weight", same(json!(0.375))),
        ("subsurface_radius", same(json!(2.5))),
        ("subsurface_radius_scale", same(json!([1.0, 0.75, 0.5]))),
    ];
    pairs.into_iter().map(|(k, (a, b))| (k, a, b)).collect()
}

#[test]
fn material_set_changes_each_key_and_reports_it() {
    let mut s = agent();
    let start = s.query("material.list", json!({}));
    for (key, given, reported) in edits() {
        let mut want = values(&mut s, 1);
        assert_ne!(want[key], reported, "{key}: red wax has another value");
        want[key] = reported;
        let set = s.query("material.set", json!({ "material": "red-wax", key: given }));
        assert_eq!(set["material"]["values"], want, "{key}: only it changed");
        assert_eq!(
            (&set["material"]["id"], &set["active"]),
            (&json!(2), &json!("gray"))
        );
        assert_eq!(values(&mut s, 1), want, "{key}: material.list reports it");
    }
    let lit = s.query(
        "material.set",
        json!({ "material": "#2", "environment": "soft-studio" }),
    );
    assert_eq!(lit["material"]["environment"], json!("soft-studio"));
    let active = s.query("material.set", json!({ "specular_weight": 0.5 }));
    assert_eq!(
        active["material"]["name"],
        json!("gray"),
        "left out, the active one"
    );
    let mut steps = 0;
    while s.run("material.undo", json!({})).is_ok() {
        steps += 1;
    }
    assert_eq!(steps, 15, "one palette step a call");
    assert_eq!(
        s.query("material.list", json!({})),
        start,
        "undone, as shipped"
    );
}

#[test]
fn material_set_refuses_what_a_look_file_refuses() {
    let mut s = agent();
    s.ok("material.copy", json!({ "name": "mine" }));
    let before = s.query("material.list", json!({}));
    for (args, wanted, names) in [
        (
            json!({ "material": "#2", "specular": 0.5 }),
            "E_ARGS_INVALID",
            "specular",
        ),
        (
            json!({ "material": "#2", "specular_ior": 0.5 }),
            "E_ARGS_INVALID",
            "specular_ior",
        ),
        (
            json!({ "material": "#2", "base_metalness": 2 }),
            "E_ARGS_INVALID",
            "base_metalness",
        ),
        (
            json!({ "material": "#2", "base_color": [2, 0, 0] }),
            "E_ARGS_INVALID",
            "base_color",
        ),
        (
            json!({ "material": "#2", "coat_color": "#12" }),
            "E_ARGS_INVALID",
            "coat_color",
        ),
        (
            json!({ "material": "#2", "subsurface_radius": -1 }),
            "E_ARGS_INVALID",
            "subsurface_radius",
        ),
        (
            json!({ "material": "#2", "environment": "moon" }),
            "E_ARGS_INVALID",
            "environment",
        ),
        (json!({ "material": "#2" }), "E_ARGS_INVALID", "key"),
        (
            json!({ "material": "#99", "coat_weight": 1 }),
            "E_NOT_FOUND",
            "#99",
        ),
        (
            json!({ "material": "mine", "shipped": true }),
            "E_PRECONDITION",
            "built in",
        ),
    ] {
        // The argument's path or the message names what was wrong.
        let e = s.err("material.set", args.clone());
        let said = format!("{} {}", e.path.unwrap_or_default(), e.message);
        assert_eq!(code(e.code), wanted, "{args}: {said}");
        assert!(said.contains(names), "{args}: {said}");
    }
    assert_eq!(
        s.query("material.list", json!({})),
        before,
        "a refusal changes nothing"
    );
    // A value read from material.list and written back is the value it has: no step.
    let ior = values(&mut s, 1)["specular_ior"].clone();
    s.ok(
        "material.set",
        json!({ "material": "red-wax", "specular_ior": ior }),
    );
    s.ok("material.undo", json!({}));
    assert!(
        !names(&mut s).contains(&"mine".to_string()),
        "no step: undo takes the copy back"
    );
}

/// A built-in material keeps an edit until `shipped` puts its shipped settings back, with
/// its name and title as they are now; a key given with it applies on top.
#[test]
fn shipped_puts_a_built_in_back_as_it_shipped() {
    let mut s = agent();
    let shipped = values(&mut s, 1);
    s.ok(
        "material.set",
        json!({ "material": "red-wax", "specular_weight": 0.25, "environment": "soft-studio" }),
    );
    s.ok(
        "material.rename",
        json!({ "material": "red-wax", "name": "wax", "title": "Wax" }),
    );
    let back = s.query(
        "material.set",
        json!({ "material": "wax", "shipped": true }),
    );
    let m = &back["material"];
    assert_eq!(m["values"], shipped);
    assert_eq!(
        (&m["name"], &m["title"], &m["environment"]),
        (&json!("wax"), &json!("Wax"), &json!("studio"))
    );
    let on_top = json!({ "material": "wax", "specular_weight": 0.5, "shipped": true });
    let mut want = shipped.clone();
    want["specular_weight"] = json!(0.5);
    assert_eq!(s.query("material.set", on_top)["material"]["values"], want);
    s.ok("material.undo", json!({}));
    assert_eq!(
        values(&mut s, 1),
        shipped,
        "undo steps back to the shipped settings"
    );
}

/// The Check clause "changing a material's specular and pressing the palette Undo restores it
/// and leaves the sculpt" (decision 0043, section 9): the scene's hash and history stay; Redo
/// puts the change back.
#[test]
fn the_palette_undo_restores_a_material_and_leaves_the_sculpt() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    let (hash, history) = (s.hash(), s.history());
    let shipped = values(&mut s, 0);
    let set = s.query("material.set", json!({ "specular_weight": 0.25 }));
    assert_eq!(set["material"]["values"]["specular_weight"], json!(0.25));
    assert_eq!((s.hash(), s.history()), (hash.clone(), history.clone()));

    let undone = s.query("material.undo", json!({}));
    assert_eq!(undone["command"], json!("material.set"));
    assert_eq!(values(&mut s, 0), shipped, "the specular is back");
    assert_eq!(s.hash(), hash, "the sculpt stays");
    assert_eq!(s.history(), history, "and so does its undo");
    s.ok("material.redo", json!({}));
    assert_eq!(values(&mut s, 0)["specular_weight"], json!(0.25));
}
