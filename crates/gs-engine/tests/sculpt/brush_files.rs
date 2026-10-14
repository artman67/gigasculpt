//! Brush preset files (decision 0018, "Saving a brush"): `brush.save` writes a brush into its
//! preset file's text and `brush.load` reads a saved file back as the brush's settings, so a
//! preset file loads, strokes and saves back byte for byte, the alpha picked comes back, and a
//! file that is not a saved brush is refused at its path with nothing changed.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap in tests"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::EngineConfig;
use gs_protocol::{LogRecord, Role, ScriptLine};
use gs_testkit::{Session, code, success};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke};

/// A fresh folder in the build's scratch folder, one per call.
fn folder() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let name = format!("brush-files-{}-{n}", std::process::id());
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn at(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every shipped preset file's text, by its brush's name.
fn shipped() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brushes");
    let mut out: Vec<(String, String)> = (std::fs::read_dir(dir).unwrap())
        .map(|e| read(&e.unwrap().path().to_string_lossy()))
        .map(|text| {
            let file: toml::Table = toml::from_str(&text).unwrap();
            (file["name"].as_str().unwrap().to_string(), text)
        })
        .collect();
    out.sort();
    out
}

fn text_of(brush: &str) -> String {
    let all = shipped();
    all.into_iter().find(|(n, _)| n == brush).unwrap().1
}

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

fn save(s: &mut Session, args: Value) -> Value {
    s.query("brush.save", args)
}

fn load(s: &mut Session, path: &str) -> Value {
    s.query("brush.load", json!({ "path": path }))
}

/// The farthest any vertex of `sphere` moved from `before`.
fn moved(s: &mut Session, before: &gs_protocol::PlainMesh) -> f32 {
    let after = s.mesh("sphere");
    (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f32::max))
        .fold(0.0, f32::max)
}

/// Strokes the session's brush across the sphere as the window does, carrying the brush's
/// settings; how far it moved the surface, and the scene's hash after.
fn stroke_with_session(s: &mut Session) -> (f32, String) {
    let before = s.mesh("sphere");
    let st = s.query("session.state", json!({}));
    let brush = st["brush"].as_str().unwrap().to_string();
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    let carried = json!({ "settings": st["brush.settings"] });
    s.ok("brush.stroke", stroke(&brush, &path, &carried));
    (moved(s, &before), s.hash())
}

#[test]
fn every_shipped_brush_saves_as_its_preset_file_byte_for_byte() {
    let dir = folder();
    let mut s = Session::new();
    for (name, text) in shipped() {
        let path = at(&dir, &format!("{name}.toml"));
        let out = save(&mut s, json!({ "brush": name, "path": path }));
        assert_eq!(out["brush"], json!(name));
        assert_eq!(read(&path), text, "{name}");
        // Loaded back it is the shipped brush again, with nothing changed.
        let loaded = load(&mut s, &path);
        assert_eq!(loaded["brush"], json!(name));
        assert_eq!(loaded["settings"], json!({}), "{name}");
    }
}

#[test]
fn a_preset_file_loads_strokes_as_its_settings_say_and_saves_back_byte_for_byte() {
    // The catalog's check (core.brush-presets): a file with its own strength and a comment of
    // its own, as someone would write it.
    let dir = folder();
    let text = text_of("Draw").replace(
        "strength = 0.4\n",
        "strength = 0.8 # twice the shipped strength\n",
    );
    let text = format!("# A stronger Draw.\n{text}");
    let file = at(&dir, "strong-draw.toml");
    std::fs::write(&file, &text).unwrap();
    let mut s = sphere_session(None);
    let loaded = load(&mut s, &file);
    assert_eq!(loaded["brush"], "Draw");
    assert_eq!(loaded["strength"].as_f64(), Some(f64::from(0.8_f32)));
    assert_eq!(loaded["settings"], json!({ "strength": 0.8 }), "its patch");
    let (strong, with_file) = stroke_with_session(&mut s);
    let again = at(&dir, "again.toml");
    save(&mut s, json!({ "path": again }));
    assert_eq!(
        read(&again),
        text,
        "saves back byte for byte, comments kept"
    );
    // The file's setting strokes exactly as the same setting given to brush.set, and unlike
    // the shipped brush.
    let mut set_so = sphere_session(None);
    set(&mut set_so, json!({ "brush": "Draw", "strength": 0.8 }));
    assert_eq!(stroke_with_session(&mut set_so).1, with_file);
    let mut plain = sphere_session(None);
    let (shipped_moved, _) = stroke_with_session(&mut plain);
    assert!(
        strong > 1.5 * shipped_moved,
        "twice the strength moves it further: {strong} and {shipped_moved}"
    );
    // A setting changed after loading is written where the file has it.
    set(&mut s, json!({ "strength": 0.6 }));
    let changed = at(&dir, "changed.toml");
    save(&mut s, json!({ "path": changed }));
    let want = text.replace("0.8 # twice", "0.6 # twice");
    assert_eq!(read(&changed), want);
}

#[test]
fn an_alpha_picked_by_number_or_by_name_saves_and_loads_back() {
    // The catalog's check (alpha.palette): alpha N, then saving and loading a brush, is alpha N.
    let dir = folder();
    let mut s = Session::new();
    let names: Vec<String> = (s.query("alpha.list", json!({}))["alphas"].as_array())
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.len() > 1);
    for brush in ["Clay", "Draw"] {
        set(&mut s, json!({ "brush": brush }));
        for (n, name) in names.iter().enumerate() {
            s.ok("alpha.pick", json!({ "index": n }));
            let path = at(&dir, &format!("{brush}-{n}.toml"));
            save(&mut s, json!({ "path": path }));
            assert!(
                read(&path).contains(&format!("image = \"{name}\"")),
                "{path}"
            );
            let other = (n + 1) % names.len();
            s.ok("alpha.pick", json!({ "index": other }));
            let loaded = load(&mut s, &path);
            assert_eq!(loaded["alpha"], json!(name), "{brush} with alpha {n}");
            if brush == "Draw" {
                // Draw ships no alpha: its settings gain the alpha's name, not its defaults.
                let only = json!({ "tip": { "alpha": { "image": name } } });
                assert_eq!(loaded["settings"], only, "{n}");
            }
        }
    }
    set(&mut s, json!({ "brush": "Clay", "alpha": "ring" }));
    let path = at(&dir, "ring.toml");
    save(&mut s, json!({ "path": path }));
    set(&mut s, json!({ "alpha": "square" }));
    assert_eq!(
        load(&mut s, &path)["alpha"],
        "ring",
        "by name, with brush.set"
    );
}

#[test]
fn a_paint_table_and_the_session_colors_survive_saving_and_loading() {
    let dir = folder();
    let mut s = Session::new();
    let colors = s.query(
        "color.set",
        json!({ "main": "#ff8000", "secondary": "#102030" }),
    );
    // Smooth gains a [paint] table, so it blends color too (decision 0039); Paint changes its
    // color strength.
    let paint = json!({ "paint": { "channel": "color", "color_strength": 0.5 } });
    for (brush, patch) in [
        ("Smooth", paint),
        ("Paint", json!({ "paint": { "color_strength": 0.3 } })),
    ] {
        let st = set(&mut s, json!({ "brush": brush, "settings": patch }));
        let path = at(&dir, &format!("{brush}.toml"));
        save(&mut s, json!({ "path": path }));
        assert!(read(&path).contains("color_strength = "), "{brush}");
        set(&mut s, json!({ "settings": { "paint": null } }));
        let loaded = load(&mut s, &path);
        assert_eq!(loaded["settings"], st["settings"], "{brush}'s paint table");
        assert_eq!(s.query("color.set", json!({})), colors, "the colors stay");
    }
}

#[test]
fn a_named_size_curve_is_saved_and_loaded_so_the_global_one_still_passes_it_by() {
    // Naming a size curve keeps the session's global one off the brush (decision 0036), even
    // where the curve named is the default, so the name must survive the file.
    let dir = folder();
    let mut s = Session::new();
    let flat = json!({ "pressure": { "size": [[0, 1], [1, 1]] } });
    set(&mut s, json!({ "brush": "Draw", "settings": flat }));
    let path = at(&dir, "flat.toml");
    save(&mut s, json!({ "path": path }));
    assert!(
        read(&path).contains("\n[pressure]\nsize = "),
        "{}",
        read(&path)
    );
    set(&mut s, json!({ "settings": { "pressure": null } }));
    let loaded = load(&mut s, &path);
    let size = &loaded["settings"]["pressure"]["size"];
    assert_eq!(size, &json!([[0.0, 1.0], [1.0, 1.0]]), "{loaded}");
}

/// Each refusal of `brush.load`: the file's contents and what the message says.
fn bad_files() -> Vec<(&'static str, String, &'static str)> {
    let draw = text_of("Draw");
    let clay = text_of("Clay");
    vec![
        (
            "an unknown field",
            draw.replace("[tip]\n", "[tip]\nspin = 3\n"),
            "unknown field `spin`",
        ),
        (
            "a later format",
            draw.replace("version = 1", "version = 2"),
            "format 2",
        ),
        (
            "a brush that is not shipped",
            draw.replace("brush.draw", "brush.mine"),
            "brush.mine",
        ),
        (
            "a shipped brush's id under another name",
            draw.replace("\"Draw\"", "\"Mine\""),
            "name",
        ),
        (
            "another kernel",
            draw.replace(
                "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]",
                "type = \"smooth\"\nmode = \"uniform\"\ngain = 2.0",
            ),
            "kernel",
        ),
        (
            "a table the brush has taken out",
            clay.replace("\n[tip.alpha]\nimage = \"grainy-round\"\n", ""),
            "tip.alpha",
        ),
        (
            "an alpha that does not exist",
            clay.replace("grainy-round", "no-such-alpha"),
            "no-such-alpha",
        ),
        ("not a preset", "strength = ".into(), "does not parse"),
    ]
}

#[test]
fn a_file_that_is_not_a_saved_brush_is_refused_at_its_path_and_changes_nothing() {
    let dir = folder();
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Clay", "strength": 0.5 }));
    let before = s.query("session.state", json!({}));
    let missing = at(&dir, "missing.toml");
    let e = s.err("brush.load", json!({ "path": missing }));
    assert_eq!(
        (code(e.code).as_str(), e.path.as_deref()),
        ("E_NOT_FOUND", Some("/path"))
    );
    for (what, text, says) in bad_files() {
        let path = at(&dir, "bad.toml");
        std::fs::write(&path, &text).unwrap();
        let e = s.err("brush.load", json!({ "path": path }));
        assert_eq!(e.path.as_deref(), Some("/path"), "{what}: {}", e.message);
        assert!(e.message.contains(says), "{what}: {}", e.message);
        assert_eq!(s.query("session.state", json!({})), before, "{what}");
    }
    // Saving: an unknown brush is refused at its name, a file already there at `overwrite` until
    // asked, so the window asks a plain yes or no (decision 0062, row 786).
    let e = s.err("brush.save", json!({ "brush": "Nope", "path": missing }));
    assert_eq!(
        (code(e.code).as_str(), e.path.as_deref()),
        ("E_NOT_FOUND", Some("/brush"))
    );
    assert!(!Path::new(&missing).exists(), "nothing written for it");
    let path = at(&dir, "clay.toml");
    std::fs::write(&path, "kept").unwrap();
    let e = s.err("brush.save", json!({ "path": path }));
    assert_eq!(
        (code(e.code).as_str(), e.path.as_deref()),
        ("E_PRECONDITION", Some("/overwrite"))
    );
    assert_eq!(read(&path), "kept", "a refused save writes nothing");
    let e = s.err("brush.save", json!({ "path": dir.to_string_lossy() }));
    assert_eq!(e.path.as_deref(), Some("/path"), "a folder: {}", e.message);
    save(&mut s, json!({ "path": path, "overwrite": true }));
    assert!(read(&path).contains("strength = 0.5"));
}

#[test]
fn a_held_slot_brush_is_the_one_saved_and_loaded() {
    let dir = folder();
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Draw" }));
    s.ok("brush.slot", json!({ "slot": "smooth", "hold": true }));
    set(&mut s, json!({ "strength": 0.7 }));
    let path = at(&dir, "smooth.toml");
    let out = save(&mut s, json!({ "path": path }));
    assert_eq!(out["brush"], "Smooth", "the held slot's brush");
    set(&mut s, json!({ "brush": "Flat Polish" }));
    let loaded = load(&mut s, &path);
    assert_eq!(loaded["brush"], "Smooth");
    assert_eq!(loaded["slots"]["smooth"], "Smooth", "back on the slot");
    let released = s.query("brush.slot", json!({ "release": true }));
    assert_eq!(released["brush"], "Draw", "the main brush is untouched");
}

#[test]
fn a_load_is_logged_with_its_file_hash_and_replays_while_the_file_is_unchanged() {
    let dir = folder();
    let log = log_path("brush-files");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    set(
        &mut s,
        json!({ "brush": "Clay", "strength": 0.5, "alpha": "ring" }),
    );
    let path = at(&dir, "clay.toml");
    let saved = save(&mut s, json!({ "path": path }));
    load(&mut s, &path);
    let lines: Vec<LogRecord> = (read(&log.to_string_lossy()).lines())
        .filter_map(|l| match ScriptLine::parse(l) {
            Ok(ScriptLine::Record(r)) if r.command.starts_with("brush.") => Some(r),
            _ => None,
        })
        .collect();
    let commands: Vec<&str> = lines.iter().map(|r| r.command.as_str()).collect();
    assert_eq!(
        commands,
        ["brush.set", "brush.load"],
        "a save is not logged"
    );
    let logged = &lines[1];
    assert_eq!(
        logged.args["hash"], saved["hash"],
        "the file's hash, pinned"
    );
    // Replayed in a fresh session, the line loads the same brush.
    let mut replay = Session::new();
    let request = logged.request().unwrap_or_else(|e| panic!("{e}"));
    let got = success(replay.request(request), "the logged load");
    assert_eq!(got.result["settings"], set(&mut s, json!({}))["settings"]);
    // A changed file is refused on replay.
    std::fs::write(&path, read(&path).replace("0.5", "0.6")).unwrap();
    let e = replay.err("brush.load", logged.args.clone());
    assert!(e.message.contains("changed"), "{}", e.message);
    std::fs::remove_file(&log).ok();
}
