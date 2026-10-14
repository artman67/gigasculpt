//! A form's life (decision 0025, section 2, amended 2026-10-14): a form is asked, typed and
//! sent; refused, it opens again as typed with why under its field; answered, it ends, so the
//! same step sent later from a key and refused says why on the status line and never brings the
//! old form back.

use gs_platform::Modifiers;

use crate::rig::{Ui, folder};
use crate::scene::{answer, form, problem};

/// Ctrl held, with Shift and Alt as asked.
fn mods(shift: bool, alt: bool) -> Modifiers {
    Modifiers {
        ctrl: true,
        shift,
        alt,
        ..Modifiers::default()
    }
}

/// A file where the layouts folder should be, so nothing can be saved in it.
fn block(layouts: &std::path::Path) {
    std::fs::write(layouts, "in the way").unwrap_or_else(|e| panic!("{e}"));
}

/// Refusals' handback (item 260, note 5): Save layout… refused keeps its name typed; answered,
/// it saves and ends; Ctrl+Shift+1 later sends the same step, and its refusal says why on the
/// line with the old form left shut.
#[test]
fn a_form_that_ran_stays_shut_when_a_key_sends_its_step_again() {
    let mut ui = Ui::new();
    let dir = folder("form-life");
    let layouts = dir.join("layouts");
    ui.host.lock().shell.layouts = Some(layouts.clone());
    block(&layouts);
    ui.tap("I", mods(true, true));
    answer(&mut ui, "arrangement-1");
    let asked = form(&ui).unwrap_or_else(|| panic!("refused, the form stays"));
    assert_eq!(asked.2, [("save".into(), "arrangement-1".into())]);
    std::fs::remove_file(&layouts).unwrap_or_else(|e| panic!("{e}"));
    answer(&mut ui, "");
    assert!(layouts.join("arrangement-1.toml").is_file(), "saved");
    assert!(form(&ui).is_none(), "answered, the form ends");
    std::fs::remove_dir_all(&layouts).unwrap_or_else(|e| panic!("{e}"));
    block(&layouts);
    ui.tap("1", mods(true, false));
    assert!(
        form(&ui).is_none(),
        "the old form stays shut: {:?}",
        form(&ui)
    );
    assert!(problem(&ui).is_some(), "the line says why");
}
