//! A scene holding both UV hint columns (decision 0061 section 6, amended 2026-10-13, step UV7),
//! painted with UV Hint and UV Density, saves and opens with them bit for bit.

use gs_testkit::compare::bits_by_position;
use serde_json::json;

use crate::{at, folder, save, session, stroke};

#[test]
fn a_scene_with_both_hint_columns_opens_with_them_to_the_bit() {
    let dir = folder("uv-hints");
    let path = at(&dir, "hints.gss");
    let mut a = session(7, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 10 }));
    let mut hint = stroke("UV Hint", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]);
    hint["settings"] = json!({ "kernel": { "value": -1 } });
    a.ok("brush.stroke", hint);
    let density = stroke("UV Density", [0.0, -0.5, 1.0], [0.3, -0.3, 1.0]);
    a.ok("brush.stroke", density);
    let (hash, mesh) = (a.hash(), a.mesh("sphere"));
    save(&mut a, json!({ "path": path }));

    let mut b = session(99, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["refused"], json!([]), "the part opens");
    assert_eq!(b.hash(), hash, "the same scene, bit for bit");
    let back = b.mesh("sphere");
    for column in ["uv_hint", "uv_density"] {
        assert!(mesh.attributes.contains_key(column), "{column} was painted");
        let same = bits_by_position(&back, column) == bits_by_position(&mesh, column);
        assert!(same, "{column} bit for bit");
    }
}
