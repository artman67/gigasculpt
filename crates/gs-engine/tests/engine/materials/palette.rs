//! The palette itself (decision 0043, step (a)): listing and choosing by id, place or name,
//! Reference Views following a material by id, the palette's own undo, which leaves the sculpt,
//! the agent rule on it, and the log replaying it.

use gs_protocol::Role;
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use super::{log_path, look, names, refused, remove, values};
use crate::determinism::seeded;
use crate::support::{EngineSession, agent, stroke};

/// The first Reference View's own material, or null when it draws the main view's.
fn reference_look(s: &mut Session) -> Value {
    s.query("reference.list", json!({}))[0]["reference"]["look"]["matcap"].clone()
}

#[test]
fn the_palette_lists_and_chooses_without_a_renderer() {
    let mut s = agent();
    let list = s.query("material.list", json!({}));
    let looks = list["looks"].as_array().cloned().unwrap_or_default();
    let ids: Vec<u64> = looks.iter().filter_map(|m| m["id"].as_u64()).collect();
    assert_eq!(
        ids[..13],
        (1..=13).collect::<Vec<u64>>(),
        "the first thirteen shipped looks are #1 to #13, the later ones from #1001"
    );
    assert!(looks.iter().all(|m| m["built_in"] == json!(true)), "{list}");
    assert_eq!(
        (&looks[1]["name"], &looks[1]["title"]),
        (&json!("red-wax"), &json!("MatCap Red Wax"))
    );
    assert_eq!(list["selected"], json!("gray"), "MatCap Gray is active");
    assert_eq!(
        look(&mut s),
        json!("gray"),
        "session.state names it, renderer or not"
    );

    for (key, name) in [("#2", "red-wax"), ("@2", "white"), ("chrome-a", "chrome-a")] {
        let chosen = s.query("material.select", json!({ "name": key }));
        assert_eq!(chosen["selected"], json!(name), "{key}");
        assert_eq!(look(&mut s), json!(name), "{key}");
    }
    let shown = s.query("view.look", json!({ "view": "main", "matcap": "#1" }));
    assert_eq!(
        shown["matcap"],
        json!("gray"),
        "view.look chooses through the palette"
    );

    let e = s.err("material.select", json!({ "name": "red wax" }));
    assert_eq!(code(e.code), "E_NOT_FOUND", "{}", e.message);
    assert!(
        e.suggestions.contains(&"red-wax".to_string()),
        "{:?}",
        e.suggestions
    );
    assert!(e.hint.unwrap_or_default().contains("material.list"));
    let (got, _) = refused(&mut s, "material.select", json!({ "name": "#99" }));
    assert_eq!(got, "E_NOT_FOUND", "an id no material has");
    s.ok("primitive.sphere", json!({}));
    let (got, message) = refused(
        &mut s,
        "reference.open",
        json!({ "scene": true, "look": { "matcap": "purple" } }),
    );
    assert_eq!(
        got, "E_NOT_FOUND",
        "a reference's look is checked too: {message}"
    );
    assert_eq!(look(&mut s), json!("gray"), "a refusal changes nothing");
}

#[test]
fn reference_views_follow_a_material_by_id() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("material.copy", json!({ "name": "mine" }));
    s.ok(
        "reference.open",
        json!({ "scene": true, "look": { "matcap": "#14" } }),
    );
    assert_eq!(reference_look(&mut s), json!("mine"), "kept by name");

    let renamed = s.query(
        "material.rename",
        json!({ "material": "mine", "name": "ours" }),
    );
    assert_eq!(renamed["references"], json!(["ref:1"]), "{renamed}");
    assert_eq!(reference_look(&mut s), json!("ours"), "renamed with it");
    let undone = s.query("material.undo", json!({}));
    assert_eq!(undone["references"], json!(["ref:1"]), "{undone}");
    assert_eq!(reference_look(&mut s), json!("mine"), "undo keeps it right");
    s.ok("material.redo", json!({}));

    let deleted = s.query("material.delete", json!({ "material": "ours" }));
    assert_eq!(deleted["references"], json!(["ref:1"]));
    assert_eq!(
        reference_look(&mut s),
        Value::Null,
        "the main view's look now"
    );
    let copy = s.query("material.copy", json!({}));
    assert_eq!(copy["references"], json!([]), "a copy moves no reference");
}

/// The Check clause "pressing the palette Undo restores it and leaves the sculpt", with a
/// rename (decision 0043, section 9).
#[test]
fn the_palette_undo_leaves_the_sculpt() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let (got, message) = refused(&mut s, "material.undo", json!({}));
    assert_eq!(got, "E_PRECONDITION", "nothing to undo: {message}");
    s.ok("material.copy", json!({ "name": "mine" }));
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    s.ok(
        "material.rename",
        json!({ "material": "mine", "title": "Mine" }),
    );
    let (hash, history) = (s.hash(), s.history());

    let undone = s.query("material.undo", json!({}));
    assert_eq!(
        (&undone["command"], &undone["by"], &undone["active_removed"]),
        (&json!("material.rename"), &json!("test"), &json!(false))
    );
    let title = |s: &mut Session| s.query("material.list", json!({}))["looks"][13]["title"].clone();
    assert_eq!(
        title(&mut s),
        json!("MatCap Gray copy"),
        "the title is back"
    );
    assert_eq!(s.hash(), hash, "the sculpt stays");
    assert_eq!(s.history(), history, "and so does its undo");

    s.ok("history.undo", json!({}));
    assert_ne!(s.hash(), hash, "the scene's undo took the stroke back");
    assert_eq!(
        title(&mut s),
        json!("MatCap Gray copy"),
        "and left the palette"
    );
    let redone = s.query("material.redo", json!({}));
    assert_eq!(redone["command"], json!("material.rename"));
    assert_eq!(title(&mut s), json!("Mine"));

    s.ok("material.undo", json!({}));
    s.ok("material.copy", json!({ "name": "other" }));
    let (got, _) = refused(&mut s, "material.redo", json!({}));
    assert_eq!(
        got, "E_PRECONDITION",
        "a new step clears what could be redone"
    );
    s.ok("material.undo", json!({}));
    let again = s.query("material.copy", json!({ "name": "other" }));
    assert_eq!(
        again["material"]["id"],
        json!(16),
        "#15 is never handed out again"
    );

    s.ok("material.select", json!({ "name": "other" }));
    let gone = s.query("material.undo", json!({}));
    assert_eq!(
        (&gone["active"], &gone["active_removed"]),
        (&json!("gray"), &json!(true)),
        "undoing the active material's copy makes the first one active: {gone}"
    );
}

/// Decision 0043, section 5: an agent steps over only its own palette steps; the human steps
/// over anyone's.
#[test]
fn an_agent_undoes_only_its_own_palette_steps() {
    let mut s = agent();
    s.as_human("material.copy", json!({ "name": "theirs" }));
    let e = s.err("material.undo", json!({}));
    assert_eq!(code(e.code), "E_STALE", "{}", e.message);
    assert!(
        e.message.contains("material.copy by window"),
        "{}",
        e.message
    );
    assert!(
        names(&mut s).contains(&"theirs".to_string()),
        "the step stays"
    );
    s.ok("material.copy", json!({ "name": "mine" }));
    s.ok("material.undo", json!({}));
    let undone = s.as_human("material.undo", json!({}));
    assert!(
        matches!(undone.outcome, gs_protocol::Outcome::Ok(_)),
        "{undone:?}"
    );
    assert!(!names(&mut s).iter().any(|n| n == "theirs" || n == "mine"));

    // The same for an edit: the window's slider is the human's step.
    s.as_human("material.set", json!({ "specular_weight": 0.25 }));
    let e = s.err("material.undo", json!({}));
    assert_eq!(code(e.code), "E_STALE", "{}", e.message);
    assert!(
        e.message.contains("material.set by window"),
        "{}",
        e.message
    );
    assert_eq!(
        values(&mut s, 0)["specular_weight"],
        json!(0.25),
        "it stays"
    );
}

/// Palette commands are logged, so a replay ends with the same palette, every value bit for
/// bit, and the same active material.
#[test]
fn palette_commands_replay() {
    let log = log_path("palette");
    let mut s = seeded(0, Some(log.clone()));
    s.ok(
        "material.copy",
        json!({ "material": "skin-4", "name": "skin" }),
    );
    let edit = json!({ "material": "skin", "base_color": "#c05040", "specular_roughness": 0.61 });
    s.ok("material.set", edit);
    s.ok("material.set", json!({ "specular_weight": 0.37 }));
    s.ok(
        "material.set",
        json!({ "material": "red-wax", "coat_weight": 0.3 }),
    );
    s.ok(
        "material.set",
        json!({ "material": "red-wax", "shipped": true }),
    );
    s.ok(
        "material.rename",
        json!({ "material": "skin", "title": "Skin" }),
    );
    s.ok("material.copy", json!({}));
    s.ok("material.delete", json!({ "material": "gray-copy" }));
    s.ok("material.undo", json!({}));
    s.ok("material.select", json!({ "name": "#14" }));
    let palette = s.query("material.list", json!({}));

    let mut replay = Session::with(gs_engine::EngineConfig::default(), Role::Agent);
    replay.replay_log(&log);
    assert_eq!(replay.query("material.list", json!({})), palette);
    assert_eq!(look(&mut replay), json!("skin"));
    remove(&log);
}
