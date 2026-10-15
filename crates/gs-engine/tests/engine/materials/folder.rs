//! The user folder (decision 0043, step (d)): `material.folder` reads every `.toml` file in a
//! folder, in name order, into user materials before the later built-ins, with new ids; it
//! refuses what it cannot read, lists it in `material.list`'s `refused` and reads the rest; it
//! is no palette step and clears the palette's history; a replay reads the folder again.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gs_protocol::Role;
use gs_testkit::Session;
use serde_json::{Value, json};

use super::{ids, log_path, names, refused, remove};
use crate::determinism::seeded;
use crate::support::{EngineSession, agent};

/// A standard material's look file, titled `title`.
fn standard(title: &str) -> String {
    format!(
        "name = \"{title}\"\nkind = \"standard\"\nenvironment = \"soft-studio\"\n\n\
         [material]\nbase_color = [0.5, 0.25, 0.125]\nspecular_roughness = 0.5\n"
    )
}

/// A MatCap look file (the kind left out), titled `title`.
fn matcap(title: &str) -> String {
    format!("name = \"{title}\"\n\n[material]\nbase_color = [0.6, 0.12, 0.06]\n")
}

/// A fresh folder under the test target folder holding `files`, each a name and its bytes.
fn folder(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("materials-{}-{name}", std::process::id()))
        .join("materials");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    for (file, bytes) in files {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    dir
}

/// Removes a folder `folder` made, with the log beside it.
fn clean(dir: &Path) {
    if let Some(parent) = dir.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
}

fn read(s: &mut Session, dir: &Path) -> Value {
    s.query("material.folder", json!({ "folder": dir }))
}

#[test]
fn the_folder_reads_each_look_file_in_name_order() {
    let wax = matcap("My Wax");
    let clay = standard("My Clay");
    let glaze = standard("Glaze");
    let dir = folder(
        "order",
        &[
            ("b-clay.toml", clay.as_bytes()),
            ("a-wax.toml", wax.as_bytes()),
            ("B-glaze.toml", glaze.as_bytes()),
            ("c-shine.TOML", glaze.as_bytes()),
            ("notes.txt", b"not a look"),
            ("wax.toml.bak", wax.as_bytes()),
        ],
    );
    std::fs::create_dir_all(dir.join("inner.toml")).ok();
    let mut s = agent();
    s.ok("material.copy", json!({}));
    let before = ids(&mut s);

    let got = read(&mut s, &dir);
    assert_eq!(
        got["read"],
        json!(["B-glaze", "a-wax", "b-clay", "c-shine"]),
        "every .toml file (the extension in any case), in byte order of their names; other \
         files and folders skipped: {got}"
    );
    assert_eq!(got["refused"], json!([]), "{got}");
    assert_eq!(
        names(&mut s)[13..19],
        [
            "gray-copy",
            "B-glaze",
            "a-wax",
            "b-clay",
            "c-shine",
            "flat-color"
        ],
        "after the user materials already there, before the later built-ins"
    );
    assert_eq!(
        ids(&mut s),
        [&before[..14], &[15, 16, 17, 18], &before[14..]].concat(),
        "new ids"
    );
    let list = s.query("material.list", json!({}));
    let clay = &list["looks"][16];
    assert_eq!(
        (
            &clay["title"],
            &clay["kind"],
            &clay["environment"],
            &clay["built_in"]
        ),
        (
            &json!("My Clay"),
            &json!("standard"),
            &json!("soft-studio"),
            &json!(false)
        ),
        "named by its file, titled by its recipe: {clay}"
    );
    assert_eq!(
        list["looks"][15]["kind"],
        json!("matcap"),
        "the kind left out"
    );
    assert_eq!(list["selected"], json!("gray"), "reading chooses nothing");

    let (got, message) = refused(&mut s, "material.undo", json!({}));
    assert_eq!(
        got, "E_PRECONDITION",
        "reading is no step and clears the history, the copy's step with it: {message}"
    );
    let chosen = s.query("material.select", json!({ "name": "a-wax" }));
    assert_eq!(chosen["selected"], json!("a-wax"));
    let copy = s.query("material.copy", json!({}));
    assert_eq!(copy["material"]["id"], json!(19), "ids go on after them");
    s.ok("material.undo", json!({}));
    clean(&dir);
}

#[test]
fn the_folder_refuses_what_it_cannot_read_and_reads_the_rest() {
    let good = standard("Good");
    let range = "name = \"Range\"\n\n[material]\nspecular_ior = 0.5\n";
    let key = "name = \"Key\"\nshine = 2\n\n[material]\n";
    let taken = matcap("Gray Again");
    let dir = folder(
        "refused",
        &[
            ("good.toml", good.as_bytes()),
            ("range.toml", range.as_bytes()),
            ("key.toml", key.as_bytes()),
            ("gray.toml", taken.as_bytes()),
            ("main.toml", taken.as_bytes()),
            ("@two.toml", taken.as_bytes()),
            ("bytes.toml", &[0xff, 0xfe, 0x00]),
        ],
    );
    let mut s = agent();
    let got = read(&mut s, &dir);
    assert_eq!(got["read"], json!(["good"]), "{got}");
    let refusals = s.query("material.list", json!({}))["refused"].clone();
    assert_eq!(got["refused"], refusals, "material.list lists them too");
    let said: Vec<(String, String)> = (refusals.as_array().into_iter().flatten())
        .map(|r| {
            (
                r["file"].as_str().unwrap_or("").into(),
                r["error"].as_str().unwrap_or("").into(),
            )
        })
        .collect();
    let files: Vec<&str> = said.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(
        files,
        [
            "@two.toml",
            "bytes.toml",
            "gray.toml",
            "key.toml",
            "main.toml",
            "range.toml"
        ],
        "each by its file name, in name order"
    );
    for (file, words) in [
        ("@two.toml", "# and @"),
        ("bytes.toml", "cannot read"),
        ("gray.toml", "already named 'gray'"),
        ("key.toml", "shine"),
        ("main.toml", "main"),
        ("range.toml", "material.specular_ior"),
    ] {
        let error = &said.iter().find(|(f, _)| f == file).map(|(_, e)| e.clone());
        let error = error.clone().unwrap_or_default();
        assert!(error.contains(words), "{file}: {error}");
    }
    assert_eq!(names(&mut s)[13], "good");
    assert_eq!(
        names(&mut s).iter().filter(|n| *n == "gray").count(),
        1,
        "the built-in keeps its name"
    );

    let (got, message) = refused(
        &mut s,
        "material.folder",
        json!({ "folder": dir.join("nowhere") }),
    );
    assert_eq!(got, "E_NOT_FOUND", "{message}");
    let (got, message) = refused(
        &mut s,
        "material.folder",
        json!({ "folder": dir.join("good.toml") }),
    );
    assert_eq!(got, "E_PRECONDITION", "a file is not a folder: {message}");
    clean(&dir);
}

/// A look file GigaSculpt may not read is refused in a person's words and the rest are read;
/// a folder it may not read is refused so. Never the system's text with its error number.
#[test]
#[cfg(unix)]
fn a_file_or_a_folder_it_may_not_read_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path, mode: u32| {
        let mode = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(path, mode).unwrap_or_else(|e| panic!("{e}"));
    };
    let good = standard("Good");
    let files = [
        ("good.toml", good.as_bytes()),
        ("shut.toml", good.as_bytes()),
    ];
    let dir = folder("not-allowed", &files);
    mode(&dir.join("shut.toml"), 0o000);
    let mut s = agent();
    let got = read(&mut s, &dir);
    assert_eq!(got["read"], json!(["good"]), "{got}");
    assert_eq!(got["refused"][0]["file"], "shut.toml", "{got}");
    let file = got["refused"][0]["error"]
        .as_str()
        .unwrap_or("")
        .to_string();
    mode(&dir, 0o000);
    let (code, whole) = refused(&mut s, "material.folder", json!({ "folder": dir }));
    mode(&dir, 0o755);
    clean(&dir);
    assert_eq!(code, "E_PRECONDITION", "{whole}");
    for message in [file, whole] {
        assert!(message.contains("not allowed to read"), "{message}");
        assert!(!message.contains("os error"), "{message}");
    }
}

/// Decision 0043, step (d): "a hundred files stay fast". The time, without a renderer, goes to
/// the test's output; the limit is the command's cost class (Short, under 2 s).
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test times the call; nothing the engine computes reads the clock"
)]
fn a_hundred_files_read_fast() {
    let texts: Vec<(String, String)> = (0..100)
        .map(|n| (format!("look-{n:03}.toml"), standard(&format!("Look {n}"))))
        .collect();
    let files: Vec<(&str, &[u8])> = (texts.iter())
        .map(|(f, t)| (f.as_str(), t.as_bytes()))
        .collect();
    let dir = folder("hundred", &files);
    let mut s = agent();
    let start = Instant::now();
    let got = read(&mut s, &dir);
    let took = start.elapsed();
    eprintln!("material.folder read 100 files in {took:?}");
    assert_eq!(got["read"].as_array().map(Vec::len), Some(100), "{got}");
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
    clean(&dir);
}

/// `material.folder` is logged, so a replay reads the folder again and ends with the same
/// palette and the same active material.
#[test]
fn the_folder_replays() {
    let wax = matcap("My Wax");
    let dir = folder("replay", &[("wax.toml", wax.as_bytes())]);
    let log = log_path("folder-replay");
    let mut s = seeded(0, Some(log.clone()));
    read(&mut s, &dir);
    s.ok("material.select", json!({ "name": "wax" }));
    s.ok("material.copy", json!({}));
    let palette = s.query("material.list", json!({}));

    let mut replay = Session::with(gs_engine::EngineConfig::default(), Role::Agent);
    replay.replay_log(&log);
    assert_eq!(replay.query("material.list", json!({})), palette);
    remove(&log);
    clean(&dir);
}
