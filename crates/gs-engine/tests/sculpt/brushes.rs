//! What agents learn about brushes and set for the window: `brush.list` agrees with each
//! brush's own description, names are found in any case, and `brush.set` keeps a size for
//! every brush and a strength for each.

use gs_testkit::{Session, code};
use serde_json::{Value, json};

/// A number as the `f32` it stands for.
fn single(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn describe(s: &mut Session, brush: &str) -> Value {
    s.query("brush.describe", json!({ "brush": brush }))
}

/// The families in the order brushes are listed (decision 0013).
const FAMILIES: [&str; 6] = ["Build", "Carve", "Move", "Smooth", "Mask", "Paint"];

#[test]
fn the_list_agrees_with_each_brush_description() {
    let mut s = Session::new();
    let list = s.query("brush.list", json!({}));
    let brushes = list["brushes"].as_array().cloned().unwrap_or_default();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brushes");
    let files = std::fs::read_dir(dir).map_or(0, Iterator::count);
    assert_eq!(brushes.len(), files, "every preset file in assets/brushes");
    let key = |b: &Value| {
        let family = FAMILIES.iter().position(|f| b["family"] == *f);
        (family, b["name"].as_str().unwrap_or_default().to_string())
    };
    let keys: Vec<_> = brushes.iter().map(key).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "listed by family, then by name");
    for b in &brushes {
        let name = b["name"].as_str().unwrap_or_default();
        let d = describe(&mut s, name);
        let settings = &d["settings"];
        let class = settings["stroke"]["class"].as_str().unwrap_or("dab");
        assert_eq!(b["kernel"], settings["kernel"]["type"], "{name}'s kernel");
        assert_eq!(b["stroke"], json!(class), "{name}'s stroke class");
        assert_eq!(
            single(&b["strength"]),
            single(&settings["strength"]),
            "{name}'s strength"
        );
        assert_eq!(b["id"], settings["id"], "{name}'s id");
        assert_eq!(b["family"], settings["family"], "{name}'s family");
        assert!(key(b).0.is_some(), "{name}'s family is one of {FAMILIES:?}");
        let about = d["about"].as_str().unwrap_or_default();
        assert!(about.starts_with(name), "{name} is described: {about}");
        assert_eq!(
            d["modifier"].is_string(),
            (b["kernel"] == "offset" && settings["mode"] != "off") || b["kernel"] == "hook",
            "{name}'s modifier means something on the brushes that push or pull out"
        );
    }
}

#[test]
fn brushes_are_found_by_name_in_any_case_or_by_id() {
    let mut s = Session::new();
    for asked in [
        "Clay Build",
        "clay build",
        " CLAY BUILD ",
        "brush.clay-build",
    ] {
        assert_eq!(
            describe(&mut s, asked)["name"],
            json!("Clay Build"),
            "{asked:?}"
        );
    }
    let err = s.err("brush.describe", json!({ "brush": "Clay Buildup" }));
    assert_eq!(code(err.code), "E_NOT_FOUND");
    assert!(
        err.suggestions.contains(&"Clay Build".to_string()),
        "{:?}",
        err.suggestions
    );
}

#[test]
fn brush_set_keeps_the_size_for_all_and_a_strength_for_each() {
    let mut s = Session::new();
    let set = |s: &mut Session, args: Value| s.query("brush.set", args);
    let start = set(&mut s, json!({}));
    assert_eq!(
        (start["brush"].clone(), start["size_px"].clone()),
        (json!("Draw"), json!(64.0))
    );
    assert_eq!(start["strength"], json!(0.4_f32), "Draw's own");
    set(&mut s, json!({ "strength": 0.6, "size_px": 30 }));
    let moved = set(&mut s, json!({ "brush": "brush.move" }));
    assert_eq!(moved["brush"], json!("Move"), "by id, reported by name");
    assert_eq!(moved["strength"], json!(1.0), "Move keeps its own strength");
    assert_eq!(moved["size_px"], json!(30.0), "the size is shared");
    let back = set(&mut s, json!({ "brush": "draw" }));
    assert_eq!(single(&back["strength"]), Some(0.6), "Draw's set strength");
    let grown = set(&mut s, json!({ "size_step": 2 }));
    assert_eq!(grown["size_px"], json!(46.0), "two steps of 8 pixels");
    assert_eq!(
        set(&mut s, json!({ "size_step": -100 }))["size_px"],
        json!(8.0),
        "one step"
    );
    let length = set(&mut s, json!({ "size": "5%" }));
    assert_eq!(
        (length["size"].clone(), length.get("size_px")),
        (json!("5%"), None)
    );
    let longer = set(&mut s, json!({ "size_step": 3 }));
    let percent = longer["size"]
        .as_str()
        .unwrap_or_default()
        .trim_end_matches('%');
    let percent: f64 = percent.parse().unwrap_or_default();
    assert!(
        (percent - 6.5).abs() < 1e-5,
        "three tenths longer: {longer}"
    );
    let units = set(&mut s, json!({ "size": 0.2, "size_step": -1 }));
    assert!(
        (units["size"].as_f64().unwrap_or_default() - 0.18).abs() < 1e-6,
        "{units}"
    );
    let tiny = set(&mut s, json!({ "size": "5%", "size_step": -20 }));
    assert_eq!(tiny["size"], json!("0.5%"), "never below a tenth: {tiny}");
    let huge = set(&mut s, json!({ "size_px": 9990, "size_step": 5 }));
    assert_eq!(huge["size_px"], json!(10000.0), "never above 10,000 pixels");
    let both = s.err("brush.set", json!({ "size": 0.2, "size_px": 10 }));
    assert_eq!(code(both.code), "E_ARGS_INVALID");
    let mirror =
        json!({ "symmetry": { "mirror": { "y": true }, "radial": { "axis": "z", "count": 4 } } });
    let sym = set(&mut s, mirror)["symmetry"].clone();
    assert_eq!(sym["radial"]["count"], json!(4));
    for (axis, want) in [("x", [true, true, false]), ("y", [true, false, false])] {
        let m = set(&mut s, json!({ "flip_mirror": axis }))["symmetry"]["mirror"].clone();
        assert_eq!(
            [&m["x"], &m["y"], &m["z"]],
            want.map(|b| json!(b)).each_ref(),
            "{axis}"
        );
    }
    assert_eq!(
        set(&mut s, json!({ "modifier": -40 }))["modifier"],
        json!(-40.0)
    );
    let state = s.query("session.state", json!({}));
    assert_eq!(
        (
            state["brush.modifier"].clone(),
            state["symmetry.radial_count"].clone()
        ),
        (json!(-40.0), json!(4)),
        "session.state shows the settings"
    );
}

#[test]
fn falloff_belongs_to_the_brush_and_previous_goes_back_and_forth() {
    let mut s = Session::new();
    let set = |s: &mut Session, args: Value| s.query("brush.set", args);
    let first = set(&mut s, json!({}));
    assert_eq!(
        (first["falloff"].clone(), first.get("previous")),
        (json!(0.0), None)
    );
    let refused = s.err("brush.set", json!({ "previous": true }));
    assert_eq!(
        code(refused.code),
        "E_PRECONDITION",
        "nothing to go back to yet"
    );
    set(&mut s, json!({ "falloff": 40 }));
    let clay = set(&mut s, json!({ "brush": "Clay", "falloff": -20 }));
    assert_eq!(clay["previous"], json!("Draw"));
    assert_eq!(clay["falloff"], json!(-20.0), "Clay's own Falloff");
    let back = set(&mut s, json!({ "previous": true }));
    assert_eq!(
        (back["brush"].clone(), back["falloff"].clone()),
        (json!("Draw"), json!(40.0))
    );
    let again = set(&mut s, json!({ "previous": true }));
    assert_eq!(again["brush"], json!("Clay"), "and forth");
    let same = set(&mut s, json!({ "brush": "Clay" }));
    assert_eq!(
        same["previous"],
        json!("Draw"),
        "picking the same brush keeps the previous"
    );
    let both = s.err("brush.set", json!({ "brush": "Draw", "previous": true }));
    assert_eq!(code(both.code), "E_ARGS_INVALID");
}

#[test]
fn falloff_shapes_the_stroke() {
    // The same dab with a wide and a narrow Falloff: both move the surface, differently, and
    // the narrow one moves less in total (the same peak over a narrower bump).
    let moved = |falloff: f64| {
        let mut s = Session::new();
        s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
        let before = s.mesh("sphere");
        let dab = json!({ "brush": "Draw", "size": "30%", "falloff": falloff,
                          "samples": [{ "position": [0, 0, 1] }] });
        s.ok("brush.stroke", dab);
        let after = s.mesh("sphere");
        (before.positions.iter().zip(&after.positions))
            .map(|(a, b)| (0..3).map(|i| f64::from(b[i] - a[i]).abs()).sum::<f64>())
            .sum::<f64>()
    };
    let (wide, plain, narrow) = (moved(-80.0), moved(0.0), moved(80.0));
    assert!(
        narrow > 0.0 && narrow < plain && plain < wide,
        "{narrow} < {plain} < {wide}"
    );
}
