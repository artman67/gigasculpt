//! A save or an open from the window runs as a job, so the engine refuses it on the job's
//! thread; a refusal that asks still reaches the window as a question, never as a problem
//! (decision 0016, section 17, amended 2026-10-13). A real session server with the window's
//! host attached, the steps asked for as a key asks for them and run by the window loop's own
//! command path (`app::commands`).

use serde_json::json;

use crate::busy::Rig;

/// The arguments the palette's form asks for, and its command; none while it shows no form.
fn asked(rig: &Rig) -> Option<(String, Vec<String>)> {
    let studio = rig.host.lock();
    let form = studio.shell.palette.form.as_ref()?;
    let names = form.fields.iter().map(|(n, _)| n.clone()).collect();
    Some((form.step.command.clone(), names))
}

#[test]
fn a_first_save_from_the_window_asks_where_and_shows_no_problem() {
    let rig = Rig::start("asks-save");
    rig.ok("primitive.sphere", json!({}));
    // Ctrl+S on a scene with no file.
    rig.press("file.save", json!({}));
    rig.until_done();
    assert_eq!(
        asked(&rig),
        Some(("file.save".into(), vec!["path".into()])),
        "the form asks where to save"
    );
    assert_eq!(rig.problem(), "", "asking is not a failure");
}

/// The labels the palette shows, as the second frame draws it (egui sizes a new window in its
/// first frame).
fn palette(rig: &Rig) -> Vec<String> {
    let mut studio = rig.host.lock();
    studio.interface();
    studio.interface();
    let panels = studio.shell.panels();
    let shown = panels.iter().filter(|p| p.id == "palette");
    shown
        .flat_map(|p| p.widgets.iter().map(|w| w.label.clone()))
        .collect()
}

/// Open from the window with unsaved changes: the job's refusal asks Save, Don't save or
/// Cancel, and shows no problem (review 0009, C8).
#[test]
fn an_open_with_unsaved_changes_from_the_window_asks_first_and_shows_no_problem() {
    let rig = Rig::start("asks-open");
    let path = crate::files::scratch("asks-open").join("one.gss");
    std::fs::create_dir_all(path.parent().unwrap_or(&path)).unwrap_or_else(|e| panic!("{e}"));
    rig.ok("primitive.sphere", json!({}));
    // The save is a job too: it ends before the open is asked for.
    rig.press("file.save", json!({ "path": path }));
    rig.until_done();
    rig.ok("primitive.cube", json!({}));
    rig.press("file.open", json!({ "path": path }));
    rig.until_done();
    let line = "Save the changes to one.gss before opening another scene?";
    let asking = rig.host.lock().shell.palette.unsaved.asking.is_some();
    assert!(asking, "it asks: {}", rig.problem());
    assert_eq!(palette(&rig), [line, "Save", "Don't save", "Cancel"]);
    assert_eq!(rig.problem(), "", "asking is not a failure");
}

/// The status line once a frame is drawn after the window's job ended.
fn line_after_a_frame(rig: &Rig) -> String {
    rig.host.lock().interface();
    rig.problem()
}

/// A save refused on the job's thread reaches the status line in a person's words, with no
/// code and what to do, as one refused at once does; the next save that lands clears it
/// (review 0009, C1 and C2).
#[test]
fn a_save_refused_on_the_jobs_thread_reads_in_a_persons_words_and_a_good_one_clears_it() {
    let rig = Rig::start("refused-job");
    let dir = crate::files::scratch("refused-job");
    let gone = dir.join("gone");
    std::fs::create_dir_all(&gone).unwrap_or_else(|e| panic!("{e}"));
    rig.ok("primitive.sphere", json!({}));
    rig.press("file.save", json!({ "path": gone.join("one.gss") }));
    rig.until_done();
    std::fs::remove_dir_all(&gone).unwrap_or_else(|e| panic!("{e}"));
    rig.ok("primitive.cube", json!({}));
    // Ctrl+S, into the folder that is gone.
    rig.press("file.save", json!({}));
    rig.until_done();
    let said = line_after_a_frame(&rig);
    assert!(said.starts_with("There is no folder"), "{said}");
    assert!(said.contains("Check the folder"), "what to do: {said}");
    rig.press("file.save", json!({ "path": dir.join("two.gss") }));
    rig.until_done();
    assert_eq!(
        line_after_a_frame(&rig),
        "",
        "the save that lands clears it"
    );
}
