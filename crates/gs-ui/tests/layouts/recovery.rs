//! The recovery prompt (decision 0016): a crashed session that `file.recovery` lists is offered
//! back in a small dialog whose buttons are the commands an agent would run, and Later closes
//! it.

use gs_platform::{Button, Input, Modifiers};
use gs_protocol::Step;
use gs_ui::Recovery;
use serde_json::json;

use crate::rig::{Ui, center};

#[test]
fn a_crashed_session_is_offered_back_with_commands() {
    let mut ui = Ui::new();
    let listed = json!({ "candidates": [{
        "id": "main-4242", "session": "main", "autosave": "/state/autosave/head-1/head-003.gss",
        "replayable": true, "offer": "the newest autosave plus the 12 commands after it"
    }] });
    ui.host.lock().shell.recovery = Recovery::from_result(&listed);
    ui.settle();
    assert_eq!(
        ui.labels("recovery"),
        ["Recover", "Autosave only", "Discard", "Later"]
    );
    let widgets = ui.widgets("recovery");
    let command = |i: usize| serde_json::from_value::<Step>(widgets[i]["command"].clone()).ok();
    let recover = command(0).expect("Recover runs a command");
    assert_eq!(
        (recover.command.as_str(), recover.args),
        ("file.recover", json!({ "session": "main-4242" }))
    );
    let discard = command(2).expect("Discard runs a command");
    assert_eq!(
        (discard.command.as_str(), discard.args),
        ("file.recovery", json!({ "discard": "main-4242" }))
    );
    let [x, y] = center(&widgets[3]["rect"]);
    let none = Modifiers::default();
    let click = |down| Input::MouseButton {
        button: Button::Left,
        down,
        x,
        y,
    };
    ui.input(&[Input::MouseMove { x, y }, click(true)], none);
    ui.input(&[click(false)], none);
    assert!(ui.labels("recovery").is_empty(), "Later closes the prompt");
}
