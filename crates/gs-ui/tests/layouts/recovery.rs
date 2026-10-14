//! The recovery dialog (decision 0016, section 11, as amended 2026-10-13), taken as a person
//! takes it: a session crashes, the window starts as the program starts it and offers the work
//! back, and its buttons are clicked and its keys pressed. Recover, Autosave only and Later are
//! one click; Discard asks first in the same dialog, and only Delete deletes. Escape is Later on
//! the offer and Cancel on the question; Enter is the offer's first choice and the question's
//! Cancel, so a stray key never deletes. Each button that runs something is the command an agent
//! would run.

use std::path::{Path, PathBuf};

use gs_engine::{EngineConfig, StateFolder};
use gs_platform::{Button, Input, Modifiers};
use gs_protocol::{Role, Step};
use gs_testkit::session::Session;
use serde_json::{Value, json};

use crate::refusals::nearest;
use crate::region::middle;
use crate::rig::{Ui, folder};

/// The offer's buttons when the crashed session's log replays, safest first.
pub(crate) const OFFER: [&str; 4] = ["Recover", "Autosave only", "Discard", "Later"];

/// What Discard asks.
const QUESTION: &str = "Delete the autosaves of the session that closed? This cannot be undone.";

/// A session that crashed with its state folder in `dir`: session `main`, logging where the
/// window logs, made a sphere, autosaved and moved the sphere, then stopped without closing; its
/// marker is moved to a process that is gone, as a crash leaves it. Returns its autosave folder
/// and its mesh hash at the last command.
pub(crate) fn crash(dir: &Path) -> (PathBuf, String) {
    let state = dir.join("state");
    let config = EngineConfig {
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config.with_standard_log(state.clone()), Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let copy = s.ok("file.quick_save", json!({})).result["path"].clone();
    let copy = PathBuf::from(copy.as_str().unwrap_or_default());
    let up = json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 0.5 });
    s.ok("mesh.translate", up);
    let hash = s.hash();
    drop(s);
    let gone = gone_pid();
    let sessions = state.join("sessions");
    let live = sessions.join(format!("main-{}.json", std::process::id()));
    let read = std::fs::read(&live).unwrap_or_else(|e| panic!("{e}"));
    let mut marker: Value = serde_json::from_slice(&read).unwrap_or_else(|e| panic!("{e}"));
    marker["pid"] = gone.into();
    let dead = sessions.join(format!("main-{gone}.json"));
    std::fs::write(dead, marker.to_string()).unwrap_or_else(|e| panic!("{e}"));
    std::fs::remove_file(live).unwrap_or_else(|e| panic!("{e}"));
    let folder = copy.parent().map(Path::to_path_buf);
    (folder.unwrap_or_else(|| panic!("no autosave")), hash)
}

/// The id of a process that has ended.
fn gone_pid() -> u32 {
    let mut gone = std::process::Command::new("true")
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    gone.wait().unwrap_or_else(|e| panic!("{e}"));
    gone.id()
}

/// The window started on `dir` after the crash, as the program starts it, its first frame
/// drawn.
pub(crate) fn launched(dir: &Path) -> Ui {
    let mut ui = Ui::launch(dir, &dir.join("state"));
    ui.settle();
    ui
}

/// What the dialog says, and its buttons in order; nothing when it is shut.
fn dialog(ui: &mut Ui) -> (String, Vec<String>) {
    let (mut said, mut buttons) = (String::new(), Vec::new());
    for widget in ui.widgets("recovery") {
        let label = widget["label"].as_str().unwrap_or_default().to_string();
        match widget["role"].as_str() {
            Some("button") => buttons.push(label),
            _ => said = label,
        }
    }
    (said, buttons)
}

/// The dialog's buttons, in order; none when it is shut.
pub(crate) fn buttons(ui: &mut Ui) -> Vec<String> {
    dialog(ui).1
}

/// What the dialog says.
fn said(ui: &mut Ui) -> String {
    dialog(ui).0
}

/// The command the dialog's button `label` runs, if it runs one.
fn command(ui: &mut Ui, label: &str) -> Option<Step> {
    let widgets = ui.widgets("recovery");
    let button = widgets.iter().find(|w| w["label"] == label)?;
    serde_json::from_value(button["command"].clone()).ok()
}

/// Review 0009, B4: Discard is the one button in the window that deletes files. It asks in the
/// same dialog what goes and that it cannot be undone; Cancel goes back to the offer, and only
/// Delete deletes the crashed session's autosaves.
#[test]
fn discard_asks_first_and_only_delete_deletes_the_autosaves() {
    let dir = folder("recovery-discard");
    let (autosaves, _) = crash(&dir);
    let mut ui = launched(&dir);
    assert_eq!(buttons(&mut ui), OFFER, "the crash is offered back");
    ui.press("recovery > Discard");
    assert_eq!(said(&mut ui), QUESTION, "Discard asks first");
    assert_eq!(buttons(&mut ui), ["Delete", "Cancel"]);
    assert!(autosaves.exists(), "nothing is deleted before Delete");
    ui.press("recovery > Cancel");
    assert_eq!(buttons(&mut ui), OFFER, "Cancel goes back to the offer");
    assert!(autosaves.exists(), "and deletes nothing");
    ui.press("recovery > Discard");
    ui.press("recovery > Delete");
    assert!(buttons(&mut ui).is_empty(), "Delete closes the dialog");
    assert!(!autosaves.exists(), "and deletes the autosaves");
    let found = ui.ok("file.recovery", json!({}));
    assert_eq!(found["candidates"], json!([]), "nothing is offered now");
}

/// Review 0009, C10: Escape on the question is Cancel and on the offer Later, which loses
/// nothing: the offer comes back at the next start. The palette opened in front of the offer
/// takes the first Escape itself.
#[test]
fn escape_is_cancel_on_the_question_and_later_on_the_offer() {
    let dir = folder("recovery-escape");
    let (autosaves, _) = crash(&dir);
    let mut ui = launched(&dir);
    let none = Modifiers::default();
    ui.press("recovery > Discard");
    ui.tap("Escape", none);
    assert_eq!(buttons(&mut ui), OFFER, "Escape on the question is Cancel");
    ui.tap("Space", none);
    assert!(ui.host.lock().shell.palette.open, "Space opens the palette");
    ui.tap("Escape", none);
    assert!(
        !ui.host.lock().shell.palette.open,
        "Escape shuts the palette"
    );
    assert_eq!(buttons(&mut ui), OFFER, "and leaves the offer behind it");
    ui.tap("Escape", none);
    assert!(buttons(&mut ui).is_empty(), "Escape on the offer is Later");
    assert!(autosaves.exists(), "which deletes nothing");
    drop(ui);
    let mut ui = launched(&dir);
    assert_eq!(
        buttons(&mut ui),
        OFFER,
        "the offer is back at the next start"
    );
}

/// Review 0009, C10: Enter on the question is Cancel, so a stray Enter never deletes; Enter on
/// the offer is its first and safest choice, Recover, which brings the scene back to the crashed
/// session's last command.
#[test]
fn enter_is_cancel_on_the_question_and_recover_on_the_offer() {
    let dir = folder("recovery-enter");
    let (autosaves, hash) = crash(&dir);
    let mut ui = launched(&dir);
    let none = Modifiers::default();
    ui.press("recovery > Discard");
    ui.tap("Enter", none);
    assert_eq!(buttons(&mut ui), OFFER, "Enter on the question is Cancel");
    assert!(autosaves.exists(), "and deletes nothing");
    ui.tap("Enter", none);
    assert!(buttons(&mut ui).is_empty(), "Enter on the offer answers it");
    assert_eq!(
        ui.session.hash(),
        hash,
        "with Recover: the last command too"
    );
}

/// (Escape-notes), Enter through the one order (Rough-forms, 2026-10-14): with the palette open
/// and the recovery question clicked in front of it, Enter answers only the question (its
/// Cancel), and the palette behind it runs nothing (its Sphere row) and stays open.
#[test]
fn enter_with_the_recovery_question_over_the_palette_answers_only_the_question() {
    let dir = folder("recovery-enter-over-palette");
    let (autosaves, _) = crash(&dir);
    let mut ui = launched(&dir);
    let none = Modifiers::default();
    ui.tap("Space", none);
    // A few rows, so the palette along the top leaves the dialog's buttons clear.
    ui.input(&[Input::Text("sphere".into())], none);
    assert!(ui.host.lock().shell.palette.open, "Space opens the palette");
    ui.press("recovery > Discard");
    assert_eq!(
        buttons(&mut ui),
        ["Delete", "Cancel"],
        "the question is asked"
    );
    let before = ui.session.hash();
    ui.tap("Enter", none);
    assert_eq!(buttons(&mut ui), OFFER, "Enter on the question is Cancel");
    assert!(autosaves.exists(), "and deletes nothing");
    assert!(
        ui.host.lock().shell.palette.open,
        "the palette is still open"
    );
    assert_eq!(ui.session.hash(), before, "and ran nothing");
}

/// Review 0009, C10: Recover with unsaved changes asks about them first, the offer out of the
/// way; Cancel on that question brings the offer back, as it lost nothing. Later puts it away,
/// and File > Recover… brings it back while its session waits.
#[test]
fn cancel_on_the_unsaved_question_keeps_the_offer() {
    let dir = folder("recovery-cancel");
    crash(&dir);
    let mut ui = launched(&dir);
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    ui.press("recovery > Recover");
    let asked = crate::scene::question("recovering", "this scene");
    assert_eq!(ui.labels("palette"), asked, "the changes are asked about");
    assert!(buttons(&mut ui).is_empty(), "the offer waits behind");
    ui.press("palette > Cancel");
    assert_eq!(buttons(&mut ui), OFFER, "Cancel brings the offer back");
    ui.press("recovery > Later");
    assert!(buttons(&mut ui).is_empty(), "Later puts it away");
    crate::scene::press(&mut ui, "Recover…");
    assert_eq!(buttons(&mut ui), OFFER, "File > Recover… brings it back");
}

/// The buttons that delete look like no other (review 0009): Discard on the offer and Delete on
/// its question read in the theme's warning color, Later and Cancel beside them do not. The
/// window launches drawing on lavapipe, so its pixels can be read.
#[test]
fn discard_and_delete_read_in_the_warning_color() {
    let dir = folder("recovery-colors");
    crash(&dir);
    let mut ui = Ui::launch_drawn(&dir, &dir.join("state"));
    ui.settle();
    let warning = ui.host.lock().theme().colors.warning();
    let discard = nearest(&mut ui, "recovery > Discard", warning);
    let later = nearest(&mut ui, "recovery > Later", warning);
    assert!(discard < 40 && later > 100, "{discard} and {later}");
    ui.press("recovery > Discard");
    let delete = nearest(&mut ui, "recovery > Delete", warning);
    let cancel = nearest(&mut ui, "recovery > Cancel", warning);
    assert!(delete < 40 && cancel > 100, "{delete} and {cancel}");
}

/// The buttons that run something are the commands an agent would run (Discard's is its
/// question's Delete), and a click on Later closes the dialog.
#[test]
fn the_buttons_are_the_commands_an_agent_runs_and_later_closes() {
    let dir = folder("recovery-commands");
    crash(&dir);
    let mut ui = launched(&dir);
    let found = ui.ok("file.recovery", json!({}));
    let id = found["candidates"][0]["id"].clone();
    let runs = |ui: &mut Ui, label: &str| command(ui, label).map(|s| (s.command, s.args));
    let recover = ("file.recover".to_string(), json!({ "session": id }));
    assert_eq!(runs(&mut ui, "Recover"), Some(recover));
    let autosave = json!({ "session": id, "replay": false });
    let only = ("file.recover".to_string(), autosave);
    assert_eq!(runs(&mut ui, "Autosave only"), Some(only));
    assert!(command(&mut ui, "Discard").is_none(), "Discard only asks");
    ui.press("recovery > Discard");
    let discard = ("file.recovery".to_string(), json!({ "discard": id }));
    assert_eq!(runs(&mut ui, "Delete"), Some(discard));
    ui.press("recovery > Cancel");
    let [x, y] = middle(ui.rect("recovery > Later"));
    let click = |down| Input::MouseButton {
        button: Button::Left,
        down,
        x,
        y,
    };
    let none = Modifiers::default();
    ui.input(&[Input::MouseMove { x, y }, click(true)], none);
    ui.input(&[click(false)], none);
    assert!(buttons(&mut ui).is_empty(), "Later closes the dialog");
}
