//! Alphas through the engine (decision 0018): the library lists and describes its pictures,
//! brushes stamp the alphas they name, a stroke pins its picture's hash and a changed picture
//! is refused, and `alpha.preview` shows a picture as a brush stamps it.

use base64::Engine as _;
use gs_engine::EngineConfig;
use gs_protocol::{Role, ScriptLine};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke};

/// Every alpha `alpha.list` reports, by name.
fn library(s: &mut Session) -> Vec<Value> {
    s.query("alpha.list", json!({}))["alphas"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn hash_of(s: &mut Session, name: &str) -> Value {
    (library(s).into_iter())
        .find(|a| a["name"] == json!(name))
        .map_or(Value::Null, |a| a["hash"].clone())
}

#[test]
fn the_library_lists_every_recipe_and_describes_each() {
    let mut s = Session::new();
    let alphas = library(&mut s);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/alphas");
    let files = std::fs::read_dir(dir).map_or(0, Iterator::count);
    assert_eq!(alphas.len(), files, "every recipe in assets/alphas");
    let mut hashes: Vec<String> = Vec::new();
    for a in &alphas {
        let name = a["name"].as_str().unwrap_or_default();
        let d = s.query("alpha.describe", json!({ "alpha": name }));
        assert_eq!(d["hash"], a["hash"], "{name}: describe agrees");
        assert_eq!(
            (a["bits"].clone(), a["kind"].clone()),
            (json!(16), json!("height"))
        );
        assert!(d["recipe"]["layer"].is_array(), "{name}: its recipe");
        assert!(
            !d["about"].as_str().unwrap_or_default().is_empty(),
            "{name}: said what it is"
        );
        hashes.push(a["hash"].as_str().unwrap_or_default().into());
    }
    hashes.sort();
    hashes.dedup();
    assert_eq!(hashes.len(), alphas.len(), "every picture has its own name");
    let err = s.err("alpha.describe", json!({ "alpha": "squares" }));
    assert_eq!(code(err.code), "E_NOT_FOUND");
    assert!(
        err.suggestions.contains(&"square".to_string()),
        "{:?}",
        err.suggestions
    );
}

#[test]
fn a_stroke_pins_its_alpha_and_a_changed_picture_is_refused() {
    let log = log_path("alpha");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    let before = s.hash();
    s.ok("brush.stroke", stroke("Clay", &path, &json!({})));
    assert_ne!(
        s.hash(),
        before,
        "Clay stamps its grainy tip, no longer refused"
    );
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("no log: {e}"));
    let logged = (text.lines())
        .filter_map(|l| match ScriptLine::parse(l) {
            Ok(ScriptLine::Record(r)) if r.command == "brush.stroke" => Some(r),
            _ => None,
        })
        .next_back();
    let args = logged.map(|r| r.args).unwrap_or_default();
    assert_eq!(
        args["alpha_hash"],
        hash_of(&mut s, "grainy-round"),
        "pinned"
    );
    std::fs::remove_file(&log).ok();
    let now = s.hash();
    let stale = json!({ "alpha_hash": "0".repeat(64) });
    let err = s.err("brush.stroke", stroke("Clay", &path, &stale));
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert!(
        err.message.contains("grainy-round"),
        "names the picture: {}",
        err.message
    );
    assert_eq!(s.hash(), now, "nothing changed");
    let none = s.err("brush.stroke", stroke("Draw", &path, &stale));
    assert_eq!(
        (code(none.code), none.path.as_deref()),
        ("E_ARGS_INVALID".to_string(), Some("/alpha_hash")),
        "Draw stamps no alpha"
    );
}

#[test]
fn white_is_no_alpha_and_an_unknown_alpha_is_refused() {
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    let mut plain = sphere_session(None);
    plain.ok("brush.stroke", stroke("Draw", &path, &json!({})));
    let mut white = sphere_session(None);
    let args = json!({ "settings": { "tip": { "alpha": { "image": "white" } } } });
    white.ok("brush.stroke", stroke("Draw", &path, &args));
    assert_eq!(
        plain.hash(),
        white.hash(),
        "white gives the bits of no alpha"
    );
    let mut s = Session::new();
    let before = s.query("session.state", json!({}));
    let err = s.err("brush.set", json!({ "alpha": "nope" }));
    assert_eq!(code(err.code), "E_NOT_FOUND");
    assert_eq!(
        s.query("session.state", json!({})),
        before,
        "nothing changed"
    );
    s.query("brush.set", json!({ "alpha": "ring" }));
    assert_eq!(
        s.query("session.state", json!({}))["brush.alpha"],
        json!("ring")
    );
}

/// A 16-bit grey PNG's size and values.
fn decode(png_base64: &Value) -> (u32, Vec<u16>) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.as_str().unwrap_or_default())
        .unwrap_or_default();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("{e}"));
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Grayscale, png::BitDepth::Sixteen),
        "16-bit grey"
    );
    let values = buf[..info.buffer_size()]
        .chunks(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    (info.width, values)
}

#[test]
fn a_preview_shows_the_picture_as_the_brush_stamps_it() {
    let mut s = Session::new();
    let ring = s.query("alpha.preview", json!({ "alpha": "ring", "size": 64 }));
    let (size, plain) = decode(&ring["png_base64"]);
    assert_eq!(size, 64);
    // The ring: a hole in the middle, its peak three quarters of the way to the edge.
    let at = |v: &[u16], x: usize, y: usize| v[y * 64 + x];
    assert!(at(&plain, 32, 32) < 2000, "a hole: {}", at(&plain, 32, 32));
    assert!(
        at(&plain, 32 + 24, 32) > 50000,
        "the peak: {}",
        at(&plain, 56, 32)
    );
    s.query(
        "brush.set",
        json!({ "brush": "Draw", "settings": { "tip": { "alpha": { "image": "ring", "invert": true } } } }),
    );
    let stamped = s.query("alpha.preview", json!({ "brush": "Draw", "size": 64 }));
    let (_, inverted) = decode(&stamped["png_base64"]);
    // Each smaller level rounds its means once more, at most a step per level either way.
    for (a, b) in plain.iter().zip(&inverted) {
        assert!(
            (i32::from(*a) + i32::from(*b) - 65535).abs() <= 8,
            "inverted"
        );
    }
}
