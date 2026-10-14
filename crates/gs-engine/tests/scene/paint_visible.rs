//! Each part's paint switch in a scene file (decision 0039, section 10, amended for P6): a part
//! whose paint is hidden opens hidden, and a scene saved before the switch existed opens with
//! every part's paint showing.

use serde_json::json;

use crate::golden::golden;
use crate::{at, folder, save, session};

/// Whether each part shows its paint, by name, as `part.list` reports it.
fn shown(s: &mut gs_testkit::Session) -> Vec<(String, serde_json::Value)> {
    let list = s.query("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    (rows.iter())
        .map(|row| {
            (
                row["name"].as_str().unwrap_or_default().into(),
                row["paint_visible"].clone(),
            )
        })
        .collect()
}

/// A saved and reopened scene keeps which parts hide their paint, and hashes the same.
#[test]
fn a_saved_scene_keeps_each_parts_paint_switch() {
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    s.ok(
        "part.set_paint_visible",
        json!({ "part": "sphere", "visible": false }),
    );
    let path = at(&folder("paint-visible"), "switch.gss");
    save(&mut s, json!({ "path": path }));
    let mut opened = session(0, None, None);
    opened.ok("file.open", json!({ "path": path }));
    let want = vec![
        ("sphere".into(), json!(false)),
        ("cube".into(), json!(true)),
    ];
    assert_eq!(shown(&mut opened), want);
    assert_eq!(opened.hash(), s.hash(), "the same scene");
}

/// The version 5 file, from before the switch, opens with every part's paint showing.
#[test]
fn an_old_scene_opens_with_every_parts_paint_showing() {
    let mut s = session(0, None, None);
    s.ok("file.open", json!({ "path": golden("v5.gss") }));
    let rows = shown(&mut s);
    assert!(!rows.is_empty(), "the file has parts");
    for (name, paint) in rows {
        assert_eq!(paint, json!(true), "{name}");
    }
}
