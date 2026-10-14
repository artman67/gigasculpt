//! Hint strokes through the engine (decision 0061 section 6, amended 2026-10-13, step UV7): UV
//! Hint at -1 and UV Density at 0.5, each set by `brush.stroke`'s settings, undone and redone to
//! the scene's hash.

use gs_protocol::AttrValues;
use serde_json::json;

use crate::support::agent;

#[test]
fn undo_and_redo_of_a_hint_stroke_give_the_scene_s_hash_back() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let samples = [[-0.3, 0.0, 1.0], [0.3, 0.1, 1.0]].map(|p| json!({ "position": p }));
    for (brush, value) in [("UV Hint", -1.0), ("UV Density", 0.5)] {
        let before = s.hash();
        let settings = json!({ "kernel": { "value": value } });
        let args = json!({ "part": "sphere", "brush": brush, "size": 0.6, "samples": samples, "settings": settings });
        s.ok("brush.stroke", args);
        let painted = s.hash();
        assert_ne!(painted, before, "{brush} painted");
        s.ok("history.undo", json!({}));
        assert_eq!(s.hash(), before, "{brush}: undo");
        s.ok("history.redo", json!({}));
        assert_eq!(s.hash(), painted, "{brush}: redo");
    }
    let mesh = s.mesh("sphere");
    for (column, value) in [("uv_hint", -1.0f32), ("uv_density", 0.5)] {
        let far = match mesh.attributes.get(column).map(|a| &a.values) {
            Some(AttrValues::F32(v)) => v
                .iter()
                .copied()
                .fold(0.0, |m: f32, x| if x.abs() > m.abs() { x } else { m }),
            _ => 0.0,
        };
        let toward = far * value > 0.0 && far.abs() <= value.abs() && far.abs() > value.abs() / 2.0;
        assert!(
            toward,
            "{column}: the stroke painted toward {value}, got {far}"
        );
    }
}
