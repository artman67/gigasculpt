//! The palette across a session's opens, recoveries and autosaves (decision 0043, section 7, as
//! amended on `p1/palette-e`): a file without a palette opens with the shipped one and the user
//! folder's materials, and a replay of the open's log run reads the folder too; a recovery says
//! when its autosave's palette does not read; a palette step makes an autosave due.

use std::path::Path;
use std::time::Duration;

use gs_engine::{EngineConfig, StateFolder};
use gs_io::scene::Kept;
use gs_protocol::{LogRecord, Role};
use gs_testkit::{Session, success};
use serde_json::json;

use crate::autosave::{clocked, mark_dead, tick};
use crate::golden::golden;
use crate::palette::write;
use crate::{at, folder, save, session};

/// A MatCap look file for the user folder.
const WAX: &str = "name = \"My Wax\"\n\n[material]\nbase_color = [0.6, 0.12, 0.06]\n";

/// Every material as `#id name`, in palette order.
fn names(s: &mut Session) -> Vec<String> {
    let list = s.query("material.list", json!({}));
    let looks = list["looks"].as_array().cloned().unwrap_or_default();
    (looks.iter())
        .map(|m| format!("#{} {}", m["id"], m["name"].as_str().unwrap_or_default()))
        .collect()
}

/// A fresh session that has replayed the log run at `log`.
fn replayed(log: &Path) -> Session {
    let mut s = session(0, None, None);
    let text = std::fs::read_to_string(log).unwrap();
    for (number, line) in (2..).zip(text.lines().skip(1)) {
        let record: LogRecord = serde_json::from_str(line).unwrap();
        let request = record.request().unwrap();
        success(s.request(request), &format!("log line {number}: {line}"));
    }
    s
}

/// Section 7, "The user folder through an open": a file of format 4 and a Part file open with
/// the shipped palette and the user folder's materials, with the ids they had at start, and a
/// replay of the open's log run reads the folder too; a file whose palette reads keeps its own.
#[test]
fn an_old_file_and_a_part_file_keep_the_folders_materials() {
    let dir = folder("palette-folder");
    let looks = dir.join("materials");
    std::fs::create_dir_all(&looks).unwrap();
    std::fs::write(looks.join("my-wax.toml"), WAX).unwrap();
    let log = dir.join("session.jsonl");
    let part_file = at(&dir, "sphere.gss");
    let mut s = session(0, Some(log.clone()), None);
    s.ok("material.folder", json!({ "folder": looks }));
    let started = names(&mut s);
    assert!(started.contains(&"#14 my-wax".into()), "{started:?}");
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    save(&mut s, json!({ "path": part_file, "parts": ["sphere"] }));

    let open = |s: &mut Session, file: &str| {
        s.ok(
            "material.copy",
            json!({ "material": "bone", "name": "other" }),
        );
        s.ok(
            "file.open",
            json!({ "path": file, "discard_changes": true }),
        );
    };
    open(&mut s, &golden("v5.gss"));
    let own = names(&mut s);
    assert!(
        !own.iter().any(|m| m.ends_with(" my-wax")),
        "a file with a palette keeps its own: {own:?}"
    );
    for file in [golden("v4.gss"), part_file] {
        open(&mut s, &file);
        assert_eq!(
            names(&mut s),
            started,
            "{file}: the shipped materials and the folder's, as at start"
        );
        assert_eq!(
            names(&mut replayed(&log)),
            started,
            "{file}: a replay of the open's log run reads the folder too"
        );
    }
}

/// Section 7, "A palette that does not read": a recovery says so in `skipped`, as an open does.
#[test]
fn a_recovery_says_its_palette_does_not_read() {
    let dir = folder("palette-recover-unread");
    let state = dir.join("state");
    let config = EngineConfig {
        session: "crashed".into(),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    let copy = s.ok("file.quick_save", json!({})).result["path"]
        .as_str()
        .unwrap()
        .to_string();
    let file = gs_io::scene::read(Path::new(&copy), Kept::None).unwrap();
    let mut scene = file.scene.clone();
    scene["palette"]["materials"][1]["recipe"]["sheen"] = json!(1.0);
    write(&copy, scene, file.session);
    mark_dead(&state);

    let mut r = session(0, None, Some(&state));
    let id = r.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    let got = r.ok("file.recover", json!({ "session": id, "replay": false }));
    let said = got.result["skipped"].to_string();
    assert!(
        said.contains("palette") && said.contains("unknown field `sheen`"),
        "{said}"
    );
}

/// Section 7, "A palette step makes an autosave due": after one, Rest Duration writes a copy
/// holding it, as after an edit, and then nothing is due.
#[test]
fn a_palette_step_makes_the_autosave_due() {
    let two_minutes = Duration::from_secs(120);
    let state = folder("palette-autosave");
    let (mut s, clock) = clocked(&state);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    clock.advance(two_minutes);
    assert!(tick(&mut s).is_some(), "the sphere is autosaved");
    s.ok("material.copy", json!({}));
    clock.advance(two_minutes);
    let copy = tick(&mut s).expect("the copy made an autosave due");
    let seen = s.query("file.inspect", json!({ "path": copy }));
    assert_eq!(
        seen["materials"],
        json!(21),
        "the twenty built-ins and the copy"
    );
    clock.advance(two_minutes);
    assert_eq!(tick(&mut s), None, "and then nothing is due");
}
