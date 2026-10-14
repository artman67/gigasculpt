//! The modifier keys' settings (`interface.modifier-keys`): in sticky mode a tap of Shift, Ctrl
//! or Alt turns it on until the next tap and a long hold stays momentary, shown on the cursor and
//! the status line as a held key is, and never steps a slot; off (the default), a tap leaves
//! nothing on. The status line's label can be turned off.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::rig::{Ui, held, key};

/// A tap of `keys`: down for one frame, then up.
fn tap(ui: &mut Ui, keys: Modifiers) {
    ui.input(&[], keys);
    ui.input(&[], Modifiers::default());
}

/// Whether the cursor and the status line say `text`, with the pen over the 3D view.
fn say(ui: &mut Ui, text: &str) -> (bool, bool) {
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let has = |labels: Vec<String>| labels.iter().any(|l| l == text);
    (has(ui.labels("viewport")), has(ui.labels("status")))
}

fn held_slot(ui: &mut Ui) -> Value {
    ui.state()["brush.slot.held"].clone()
}

#[test]
fn in_sticky_mode_a_tap_turns_a_key_on_and_a_long_hold_is_momentary() {
    let mut ui = Ui::new();
    let sticky = ui.state()["settings.sticky_modifiers"].clone();
    assert_eq!(sticky, json!(false), "off to start");
    tap(&mut ui, held("Shift"));
    assert_eq!(
        held_slot(&mut ui),
        Value::Null,
        "off, a tap leaves nothing on"
    );
    ui.ok("ui.settings", json!({ "sticky_modifiers": true }));
    tap(&mut ui, held("Shift"));
    assert_eq!(held_slot(&mut ui), "smooth", "a tap turns Shift on");
    let shown = say(&mut ui, "Shift: Smooth");
    assert_eq!(shown, (true, true), "the cursor and the status line say so");
    tap(&mut ui, held("Shift"));
    assert_eq!(held_slot(&mut ui), Value::Null, "the next tap turns it off");
    assert_eq!(say(&mut ui, "Shift: Smooth"), (false, false), "nor say so");
    // Held half a second, then let go.
    ui.input(&[], held("Ctrl"));
    ui.wait(0.5);
    assert_eq!(held_slot(&mut ui), "mask", "held, Ctrl is on");
    ui.input(&[], Modifiers::default());
    assert_eq!(held_slot(&mut ui), Value::Null, "a long hold is momentary");
    // A key pressed while it is held (Ctrl+Z) makes a shortcut, not a tap.
    ui.input(&[key("Z", true, held("Ctrl"))], held("Ctrl"));
    ui.input(&[key("Z", false, held("Ctrl"))], held("Ctrl"));
    ui.input(&[], Modifiers::default());
    assert_eq!(
        held_slot(&mut ui),
        Value::Null,
        "a shortcut turns nothing on"
    );
}

/// In sticky mode a tap only turns keys on or off (`interface.modifier-keys`): Shift+Ctrl tapped
/// on, off and on again at once holds the Select slot's brush as it was, where the same keys let
/// go and pressed again without sticky mode step it (`brush-settings.modifier-key-slots`).
#[test]
fn in_sticky_mode_a_tap_turns_a_slot_on_without_stepping_it() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "sticky_modifiers": true }));
    let both = Modifiers {
        shift: true,
        ctrl: true,
        ..Modifiers::default()
    };
    let select = |ui: &mut Ui| ui.state()["brush.slot.select"].clone();
    tap(&mut ui, both);
    assert_eq!(held_slot(&mut ui), "select", "a tap turns it on");
    tap(&mut ui, both);
    assert_eq!(held_slot(&mut ui), Value::Null, "the next turns it off");
    tap(&mut ui, both);
    assert_eq!(held_slot(&mut ui), "select", "and on again");
    assert_eq!(select(&mut ui), "Select Rect", "not stepped");
}

/// The status line's label for the held keys can be turned off (`ui.settings` `status_label`);
/// the cursor still says what they do.
#[test]
fn the_status_line_label_can_be_turned_off() {
    let mut ui = Ui::new();
    ui.input(&[], held("Shift"));
    assert_eq!(say(&mut ui, "Shift: Smooth"), (true, true), "on to start");
    ui.ok("ui.settings", json!({ "status_label": false }));
    assert_eq!(
        say(&mut ui, "Shift: Smooth"),
        (true, false),
        "the cursor only"
    );
}

/// The keys a stroke holds, a tap turned on or the pen's button stands for give egui's command
/// key as the platform has it: Cmd on macOS, Ctrl elsewhere. So on a Mac a Ctrl turned on by a
/// tap (or held for the Mask slot) is never a text field's Cmd, and Cmd held still is.
#[test]
fn the_command_key_is_cmd_on_macos_and_ctrl_elsewhere() {
    use egui::Modifiers as Keys;
    let ctrl = Keys {
        ctrl: true,
        command: true,
        ..Keys::NONE
    };
    let cmd = Keys {
        mac_cmd: true,
        ..Keys::NONE
    };
    for (mac, keys, command) in [
        (true, ctrl, false),
        (true, cmd, true),
        (false, ctrl, true),
        (false, cmd, false),
        (false, Keys::CTRL, true),
    ] {
        let given = gs_ui::commanding(keys, mac);
        assert_eq!(given.command, command, "mac {mac}, {keys:?}");
        assert_eq!(
            Keys { command, ..given },
            Keys { command, ..keys },
            "the rest as held"
        );
    }
}
