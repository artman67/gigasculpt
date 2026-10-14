//! Copy, cut and paste in a text field (review 0009, B1), each key sent as the window sends it:
//! the system clipboard is read for a paste key, and the text a field copies or cuts comes out
//! in the frame for the window to put on the clipboard. Outside a field the same keys stay keys,
//! so Ctrl+Shift+C still shrinks the visible area and Ctrl+Shift+X grows it over the 3D view.

use gs_platform::{Input, Modifiers};
use gs_protocol::Step;
use serde_json::json;

use crate::rig::{Ui, folder, key};
use crate::scene::{form, press, problem};

/// Copy, cut and paste's modifier: Cmd on macOS, Ctrl elsewhere.
fn command() -> Modifiers {
    let mac = cfg!(target_os = "macos");
    Modifiers {
        ctrl: !mac,
        logo: mac,
        ..Modifiers::default()
    }
}

/// Presses and lets go of `name` with `held`, a frame each, as the window does with `clipboard`
/// on the system clipboard: the text the frames copied, and the commands they asked for.
fn chord(
    ui: &mut Ui,
    name: &str,
    held: Modifiers,
    clipboard: Option<&str>,
) -> (Option<String>, Vec<Step>) {
    let (mut copied, mut asked) = (None, Vec::new());
    for down in [true, false] {
        let inputs = [key(name, down, held)];
        let mut studio = ui.host.lock();
        studio.clipboard(&inputs, || clipboard.map(String::from));
        studio.input(&inputs, held);
        let frame = studio.frame().unwrap_or_else(|e| panic!("{e}"));
        copied = copied.or(frame.copied);
        asked.extend(studio.shell.take_commands());
    }
    (copied, asked)
}

/// A first Save of the new scene, which asks for the path in a form whose field has the
/// keyboard, and `text` typed into it.
fn save_form(ui: &mut Ui, text: &str) {
    press(ui, "Save");
    let none = Modifiers::default();
    ui.input(&[], none);
    ui.input(&[Input::Text(text.into())], none);
}

/// The text in the open form's field.
fn typed(ui: &Ui) -> String {
    let (_, _, fields) = form(ui).unwrap_or_else(|| panic!("a form is open"));
    let field = fields.first().unwrap_or_else(|| panic!("a field"));
    field.1.clone()
}

/// Catalog `interface.text-fields`: a path on the clipboard pastes into the Save form's field and
/// shows there, at the cursor, and Enter saves at it; an empty clipboard pastes nothing and says
/// nothing; Shift+Insert pastes too (but on macOS, which has no Insert key); the paste key runs
/// no command.
#[test]
fn a_path_pasted_into_the_save_form_shows_in_its_field_and_saves_there() {
    let mut ui = Ui::new();
    let dir = folder("pasted-path");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    save_form(&mut ui, "");
    chord(&mut ui, "V", command(), None);
    assert_eq!(typed(&ui), "", "an empty clipboard pastes nothing");
    assert_eq!(problem(&ui), None, "and says nothing");
    let folder_path = format!("{}/", dir.display());
    let asked = chord(&mut ui, "V", command(), Some(&folder_path)).1;
    assert!(asked.is_empty(), "the paste key runs no command: {asked:?}");
    let (name, held) = if cfg!(target_os = "macos") {
        ("V", command())
    } else {
        ("Insert", crate::rig::held("Shift"))
    };
    chord(&mut ui, name, held, Some("pasted.gss"));
    let path = dir.join("pasted.gss");
    assert_eq!(
        typed(&ui),
        path.display().to_string(),
        "the pasted path shows"
    );
    let none = Modifiers::default();
    ui.input(&[key("Enter", true, none)], none);
    ui.input(&[key("Enter", false, none)], none);
    assert!(form(&ui).is_none(), "the form ran");
    assert!(path.exists(), "Enter saved at the pasted path");
}

/// Catalog `interface.text-fields`: with the field's text selected, copy puts it in the frame's
/// output for the system clipboard and leaves the field as it was; cut does too and takes it out
/// of the field. Nothing selected copies nothing. A clipboard of lines pastes into a one-line
/// field as one line, each line break a space.
#[test]
fn copy_puts_the_selected_text_in_the_frame_and_cut_also_takes_it_out() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    save_form(&mut ui, "head.gss");
    assert_eq!(
        chord(&mut ui, "C", command(), None).0,
        None,
        "nothing selected"
    );
    chord(&mut ui, "A", command(), None);
    let (copied, asked) = chord(&mut ui, "C", command(), None);
    assert_eq!(copied.as_deref(), Some("head.gss"), "copy");
    assert_eq!(typed(&ui), "head.gss", "copy leaves the text");
    assert!(asked.is_empty(), "copy runs no command: {asked:?}");
    let (cut, asked) = chord(&mut ui, "X", command(), None);
    assert_eq!(cut.as_deref(), Some("head.gss"), "cut");
    assert_eq!(typed(&ui), "", "cut takes the text out");
    assert!(asked.is_empty(), "cut runs no command: {asked:?}");
    chord(&mut ui, "V", command(), Some("head\r\ntwo.gss"));
    assert_eq!(typed(&ui), "head two.gss", "lines paste as one line");
}

/// Catalog `interface.text-fields`: over the 3D view Ctrl+Shift+C still shrinks the visible area
/// and Ctrl+Shift+X grows it, copying nothing; in a field the same keys with Shift copy and cut,
/// asking no command.
#[test]
fn ctrl_shift_c_and_x_shrink_and_grow_over_the_view_and_copy_and_cut_in_a_field() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };
    for (name, runs) in [("C", "visibility.shrink"), ("X", "visibility.grow")] {
        let (copied, asked) = chord(&mut ui, name, ctrl_shift, None);
        // Ctrl+Shift held also holds its brush slot (Select Rect), as every held chord does.
        let asked = asked.iter().map(|s| s.command.as_str());
        let asked: Vec<&str> = asked.filter(|c| *c != "brush.slot").collect();
        assert_eq!(
            (copied, asked),
            (None, vec![runs]),
            "Ctrl+Shift+{name} over the 3D view"
        );
    }
    save_form(&mut ui, "head.gss");
    chord(&mut ui, "A", command(), None);
    let with_shift = Modifiers {
        shift: true,
        ..command()
    };
    for (name, left) in [("C", "head.gss"), ("X", "")] {
        let (copied, asked) = chord(&mut ui, name, with_shift, None);
        assert_eq!(
            copied.as_deref(),
            Some("head.gss"),
            "Shift+{name} in a field"
        );
        assert!(asked.is_empty(), "Shift+{name} runs no command: {asked:?}");
        assert_eq!(typed(&ui), left, "Shift+{name} leaves");
    }
}
