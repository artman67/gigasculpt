//! Tilt Brush, Constant Tilt, Align To Path and Adaptive Map Size through the engine (decision
//! 0058): `brush.set` sets them, `brush.save` writes them and `brush.load` reads them back, and a
//! value out of range is refused at its key.

use std::path::PathBuf;

use gs_testkit::code;
use serde_json::json;

use crate::sphere_session;

#[test]
fn brush_set_sets_tilt_align_and_map_size_and_save_and_load_keep_them() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("tilt-{}.toml", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let mut s = sphere_session(None);
    let tip = json!({ "tip": {
        "tilt": 20.0, "constant_tilt": true, "align": 40.0, "alpha": { "adaptive": 0.25 }
    } });
    let st = s.query(
        "brush.set",
        json!({ "brush": "Clay Build", "settings": tip }),
    );
    assert_eq!(st["settings"], tip, "{st}");
    s.query("brush.save", json!({ "path": path }));
    let file = std::fs::read_to_string(&path).unwrap();
    for line in [
        "tilt = 20",
        "constant_tilt = true",
        "align = 40",
        "adaptive = 0.25",
    ] {
        assert!(file.contains(line), "{line}: {file}");
    }
    s.query("brush.set", json!({ "settings": { "tip": null } }));
    let loaded = s.query("brush.load", json!({ "path": path }));
    assert_eq!(loaded["settings"], tip, "{loaded}");
    std::fs::remove_file(&path).ok();

    for (bad, at, range) in [
        (
            json!({ "tilt": 91 }),
            "/settings/tip/tilt",
            "from -90 to 90",
        ),
        (
            json!({ "align": 101 }),
            "/settings/tip/align",
            "from 0 to 100",
        ),
        (
            json!({ "alpha": { "adaptive": 2 } }),
            "/settings/tip/alpha/adaptive",
            "from 0 to 1",
        ),
    ] {
        let e = s.err("brush.set", json!({ "settings": { "tip": bad } }));
        assert_eq!(
            (code(e.code).as_str(), e.path.as_deref()),
            ("E_ARGS_INVALID", Some(at)),
            "{bad}"
        );
        assert!(e.message.contains(range), "{}", e.message);
    }
}
