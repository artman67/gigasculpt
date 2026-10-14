//! How a form asks (review 0009, R3, R5, R7, R8, R13 and R15; decision 0025, section 2,
//! amended 2026-10-14): a gray Run says why under it, and Enter says it again in the problem
//! color; Reference from parts waits for a part; Decimate runs as it opens; a form is titled by
//! the row that opened it; Rename starts with the part's name, selected; Enter on a two-button
//! question takes Cancel, which is marked as Enter's; and Replace, which writes over a file,
//! reads in the warning color. Each is taken as a person takes it, from a menu.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::header::open;
use crate::refusals::nearest;
use crate::rig::{Ui, folder};
use crate::scene::{answer, form, press, problem};

/// The palette's gray Run, if it shows one: its label and why it waits.
fn gray_run(ui: &mut Ui) -> Option<(String, String)> {
    let widgets = ui.widgets("palette");
    let run = (widgets.iter()).find(|w| w["role"] == "button" && w["enabled"] == false)?;
    Some((
        run["label"].as_str()?.into(),
        run["reason"].as_str()?.into(),
    ))
}

/// Types `text` into the form's field, without Enter.
fn type_in(ui: &mut Ui, text: &str) {
    let none = Modifiers::default();
    ui.input(&[], none);
    ui.input(&[Input::Text(text.into())], none);
}

/// Review 0009, R3: Rename with spaces only cannot run. Run is gray and says why under it, not
/// only on hover; Enter sends nothing, and the reason turns the problem color.
#[test]
fn a_gray_runs_reason_is_written_under_it_and_enter_says_it_again() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    open(&mut ui, "Part");
    ui.press("menu > Rename…");
    type_in(&mut ui, "   ");
    let (run, why) = gray_run(&mut ui).unwrap_or_else(|| panic!("Run is gray"));
    assert_eq!(why, "Waits for Name");
    let reason = format!("palette > {why}");
    let [_, top, _, _] = ui.rect(&reason);
    let [_, _, _, bottom] = ui.rect(&format!("palette > {run}"));
    assert!(top >= bottom - 1.0, "under Run: {top} and {bottom}");
    let red = ui.host.lock().theme().colors.problem;
    let before = nearest(&mut ui, &reason, red);
    answer(&mut ui, "");
    assert!(ui.logged("part.rename").is_empty(), "nothing is sent");
    assert!(form(&ui).is_some(), "the form stays");
    let after = nearest(&mut ui, &reason, red);
    assert!(before > 100 && after < 40, "{before}, then {after}");
}

/// Review 0009, R8: Reference from parts with none picked waits for a part, as R3's forms do,
/// instead of sending a step the engine refuses; a part picked, Run can send it.
#[test]
fn reference_from_parts_waits_for_a_part() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let name = ui.state()["parts.active"].as_str().map(String::from);
    let name = name.unwrap_or_else(|| panic!("an active part"));
    open(&mut ui, "View");
    ui.press("menu > From parts…");
    let why = gray_run(&mut ui).map(|(_, why)| why);
    assert_eq!(why.as_deref(), Some("Waits for Parts"));
    answer(&mut ui, "");
    assert!(ui.logged("reference.open").is_empty(), "nothing is sent");
    assert_eq!(problem(&ui), None, "waiting is not a failure");
    ui.press(&format!("palette > {name}"));
    assert_eq!(gray_run(&mut ui), None, "a part picked, Run can send it");
}

/// Review 0009, R7: Decimate run as it opens, its slider untouched, works: the slider starts at
/// a value the engine takes, and the part keeps fewer faces.
#[test]
fn decimate_runs_with_its_slider_untouched() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let faces = |ui: &mut Ui| ui.ok("mesh.stats", json!({}))["parts"][0]["faces"].as_u64();
    let before = faces(&mut ui).unwrap_or_else(|| panic!("a face count"));
    open(&mut ui, "Mesh");
    ui.press("menu > Decimate…");
    answer(&mut ui, "");
    ui.wait(1.0);
    assert_eq!(problem(&ui), None, "Run works as the form opens");
    assert!(form(&ui).is_none(), "the form closes once it runs");
    let after = faces(&mut ui).unwrap_or_else(|| panic!("a face count"));
    assert!(after < before, "{after} faces of {before}");
}

/// Review 0009, R5: a form is titled by the row that opened it. File > Save… in the Part file
/// group asks where, its Run reading Save Part file, not the scene's Save.
#[test]
fn a_form_is_titled_by_the_row_that_opened_it() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    press(&mut ui, "Save…");
    assert_eq!(form(&ui).map(|f| f.0).as_deref(), Some("file.save"));
    let run = ui.widget("palette", "Save Part file");
    assert_eq!(run["role"], "button", "{run}");
}

/// Review 0009, R15: Part > Rename… opens with the part's name, selected, so what is typed
/// replaces it.
#[test]
fn rename_opens_with_the_parts_name_selected() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let name = ui.state()["parts.active"].as_str().map(String::from);
    let name = name.unwrap_or_else(|| panic!("an active part"));
    open(&mut ui, "Part");
    ui.press("menu > Rename…");
    assert_eq!(form(&ui).map(|f| f.2), Some(vec![("name".into(), name)]));
    answer(&mut ui, "head");
    assert_eq!(ui.state()["parts.active"], json!("head"));
}

/// Saves the scene in a folder of its own beside a file called taken.gss, then asks Save as…
/// onto that file, as a person does; gives the file.
fn asked_to_replace(ui: &mut Ui, name: &str) -> std::path::PathBuf {
    let dir = folder(name);
    let taken = dir.join("taken.gss");
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("file.save", json!({ "path": dir.join("own.gss") }));
    std::fs::write(&taken, "kept").unwrap_or_else(|e| panic!("{e}"));
    press(ui, "Save as…");
    answer(ui, "taken.gss");
    let asks = ["taken.gss exists. Replace it?", "Replace", "Cancel"];
    assert_eq!(
        ui.labels("palette"),
        asks,
        "Save as onto a file asks Replace"
    );
    taken
}

/// Review 0009, R13: Enter on a two-button question takes the safe choice, Cancel, which is
/// marked as Enter's: the file stays as it was and the question goes.
#[test]
fn enter_on_a_question_takes_cancel() {
    let mut ui = Ui::new();
    let taken = asked_to_replace(&mut ui, "enter-cancels");
    let cancel = ui.widget("palette", "Cancel");
    assert_eq!(cancel["key"], "Enter", "marked as Enter's: {cancel}");
    answer(&mut ui, "");
    assert!(form(&ui).is_none(), "Enter answered the question");
    let kept = std::fs::read_to_string(&taken).ok();
    assert_eq!(kept.as_deref(), Some("kept"), "the file is untouched");
}

/// Review 0009, questions: Replace, which writes over a file, reads in the theme's warning
/// color, as Delete and Don't save do; Cancel beside it does not.
#[test]
fn replace_reads_in_the_warning_color() {
    let mut ui = Ui::with_looks();
    asked_to_replace(&mut ui, "replace-warns");
    let warning = ui.host.lock().theme().colors.warning();
    let replace = nearest(&mut ui, "palette > Replace", warning);
    let cancel = nearest(&mut ui, "palette > Cancel", warning);
    assert!(replace < 40 && cancel > 100, "{replace} and {cancel}");
}
