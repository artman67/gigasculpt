//! Curve files (catalog `interface.curve-editor`, decision 0048 section 5): `curve.save` writes
//! a curve, its points, their kinds and its settings, into a file with one key, `curve`, and
//! `curve.load` answers it exactly as written. Neither is logged.

use std::path::PathBuf;

use gs_core::curve::Curve;
use gs_engine::EngineConfig;
use gs_protocol::{ErrorCode, Role, ScriptLine};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::log_path;

fn curve(v: &Value) -> Curve {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

#[test]
fn a_saved_five_point_curve_loads_back_exactly() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("curve-files");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join(format!("five-{}.toml", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let log = log_path("curve-files");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    let five = json!({
        "points": [[0, 0.1], [0.13, 0.37], [0.4, 0.29], [0.71, 0.93], [1, 1]],
        "smooth": true,
        "sharp": [2],
        "steps": 4,
        "step_strength": 0.5,
        "tile": 2
    });
    s.ok(
        "curve.save",
        json!({ "path": path, "curve": five, "overwrite": true }),
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
    let file: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!(file.keys().collect::<Vec<_>>(), ["curve"], "{text}");
    let back = s.query("curve.load", json!({ "path": path }));
    let (sent, loaded) = (curve(&five), curve(&back["curve"]));
    assert_eq!(loaded, sent, "the same curve: {back}");
    let bits = |c: &Curve| {
        c.points()
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&loaded), bits(&sent), "every point's bits");
    // Not again without overwrite, and a missing file is not found.
    let again = s.err("curve.save", json!({ "path": path, "curve": five }));
    assert_eq!(again.code, ErrorCode::Precondition, "{}", again.message);
    let missing = format!("{path}.gone");
    let gone = s.err("curve.load", json!({ "path": missing }));
    assert_eq!(gone.code, ErrorCode::NotFound, "{}", gone.message);
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    let curves = (logged.lines())
        .filter_map(|l| match ScriptLine::parse(l) {
            Ok(ScriptLine::Record(r)) => Some(r.command),
            _ => None,
        })
        .filter(|c| c.starts_with("curve."))
        .count();
    assert_eq!(curves, 0, "neither is logged");
}
