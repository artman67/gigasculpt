//! What a person sees when the engine says no (decision 0025, section 2, amended 2026-10-14):
//! a refusal reads in a person's words with no code, a refused form stays open with what was
//! typed and why under its field, a name is fixed rather than refused where the fix is plain,
//! the line clears once the next step succeeds, and the buttons that delete read in the theme's
//! warning color. Each is taken as a person takes it: typed into a form or picked from a menu.

use gs_platform::Modifiers;
use serde_json::json;

use crate::header::open;
use crate::rig::{Ui, folder, held};
use crate::scene::{answer, form, press, problem};

/// Everything the palette and the status line say, joined.
pub(crate) fn said(ui: &mut Ui) -> String {
    let mut all = ui.labels("palette");
    all.extend(ui.labels("status"));
    all.join(" | ")
}

/// What the open form's first field holds.
pub(crate) fn typed(ui: &Ui) -> Option<String> {
    form(ui).and_then(|f| f.2.first().map(|(_, text)| text.clone()))
}

/// Review 0009, C1 and C4: Import given a text file says, in a person's words and with no
/// code, which files import, under the field, and the form stays open with the path typed.
#[test]
fn importing_a_text_file_says_what_imports_in_a_persons_words() {
    let mut ui = Ui::new();
    let dir = folder("refused-import");
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "a shopping list").unwrap_or_else(|e| panic!("{e}"));
    press(&mut ui, "Import…");
    let start = typed(&ui).unwrap_or_else(|| panic!("Import asks for the file"));
    answer(&mut ui, &notes.display().to_string());
    let kept = format!("{start}{}", notes.display());
    assert_eq!(typed(&ui), Some(kept), "the form stays, as typed");
    let words = said(&mut ui);
    assert!(words.contains("OBJ, PLY, STL, ZTL and ZPR"), "{words}");
    assert!(!words.contains("E_"), "no code: {words}");
    assert_eq!(problem(&ui), None, "the why is under the field");
}

/// Review 0009, C2: a problem clears once the next step succeeds. Ctrl+S into a folder that is
/// gone shows why on the line, with what to do; Save as into a folder that is there clears it.
/// The window finds this refusal by the engine's words, so this fails if they change.
#[test]
fn a_good_save_after_a_refused_one_clears_the_line() {
    let mut ui = Ui::new();
    let dir = folder("refused-then-saved");
    let gone = dir.join("gone");
    std::fs::create_dir_all(&gone).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": gone.join("one.gss") }));
    std::fs::remove_dir_all(&gone).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    ui.tap("S", held("Ctrl"));
    let line = problem(&ui).unwrap_or_default();
    assert!(line.contains("no folder"), "Ctrl+S is refused: {line:?}");
    assert!(line.contains("make the folder first"), "{line}");
    assert!(!line.contains("E_"), "in a person's words: {line}");
    press(&mut ui, "Save as…");
    answer(&mut ui, &dir.join("two.gss").display().to_string());
    assert!(dir.join("two.gss").is_file(), "saved: {}", said(&mut ui));
    assert_eq!(problem(&ui), None, "the save clears the line");
    let words = said(&mut ui);
    assert!(!words.contains("Problem"), "{words}");
}

/// Review 0009, C4: Rename to a name another part has keeps the form open with the name typed
/// and why under it; Escape closes it and the why with it.
#[test]
fn rename_to_a_taken_name_keeps_the_form_open_with_the_text() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    open(&mut ui, "Part");
    ui.press("menu > Rename…");
    answer(&mut ui, "sphere");
    let asked = form(&ui).unwrap_or_else(|| panic!("the form stays: {}", said(&mut ui)));
    assert_eq!(asked.0, "part.rename");
    assert_eq!(asked.2, [("name".to_string(), "sphere".to_string())]);
    let words = said(&mut ui);
    assert!(words.contains("already exists"), "{words}");
    assert!(words.contains("Pick another name"), "{words}");
    assert!(!words.contains("E_"), "{words}");
    ui.tap("Escape", Modifiers::default());
    assert!(form(&ui).is_none(), "Escape closes the form");
    assert_eq!(problem(&ui), None, "and nothing is left on the line");
    assert_eq!(ui.state()["parts.active"], json!("cube"), "nothing renamed");
}

/// Review 0009, R1: Open or Import run with no file name after the scene's folder, which the
/// form starts at, asks for one in place of the system's "Is a directory", and the form stays
/// open.
#[test]
fn a_file_form_run_with_no_file_name_asks_for_one() {
    let dir = folder("refused-folder-only");
    for (label, file) in [("Open…", "one.gss"), ("Import…", "two.gss")] {
        let mut ui = Ui::new();
        ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        ui.ok("file.save", json!({ "path": dir.join(file) }));
        press(&mut ui, label);
        let start = typed(&ui).unwrap_or_default();
        assert!(
            start.ends_with(std::path::MAIN_SEPARATOR),
            "{label}: {start:?}"
        );
        answer(&mut ui, "");
        let words = said(&mut ui);
        assert!(words.contains("Type a file name"), "{label}: {words}");
        assert!(!words.contains("os error"), "{label}: {words}");
        assert!(!words.contains("E_"), "{label}: {words}");
        assert!(form(&ui).is_some(), "{label}: the form stays open");
    }
}

/// Review 0009, R2: Undo or Redo by its key with no step to take shows no problem: the gray
/// button says why already, on hover.
#[test]
fn undo_or_redo_with_nothing_to_take_shows_no_problem() {
    let mut ui = Ui::new();
    let ctrl_shift = Modifiers {
        shift: true,
        ..held("Ctrl")
    };
    for (key, mods) in [("Z", held("Ctrl")), ("Z", ctrl_shift), ("Y", held("Ctrl"))] {
        ui.tap(key, mods);
        assert_eq!(problem(&ui), None, "{key} with {mods:?}: {}", said(&mut ui));
    }
}

/// Review 0009, C6: a scene saved under a mesh file's name is pointed at File > Export, the
/// form kept as typed, and nothing is written.
#[test]
fn save_as_a_mesh_name_points_to_export() {
    let mut ui = Ui::new();
    let dir = folder("refused-mesh-name");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    press(&mut ui, "Save as…");
    let start = typed(&ui).unwrap_or_else(|| panic!("Save as asks for the file"));
    let head = dir.join("head.obj");
    answer(&mut ui, &head.display().to_string());
    let kept = format!("{start}{}", head.display());
    assert_eq!(typed(&ui), Some(kept), "the form stays, as typed");
    let words = said(&mut ui);
    assert!(words.contains("File > Export"), "{words}");
    assert!(!words.contains("E_"), "{words}");
    assert!(!head.exists() && !dir.join("head.obj.gss").exists());
}

/// The color of every pixel inside `rect`, in the last frame drawn.
fn pixels(ui: &mut Ui, rect: [f64; 4]) -> Vec<[u8; 3]> {
    ui.settle();
    ui.wait(0.2);
    let studio = ui.host.lock();
    let render = (studio.render.as_ref()).unwrap_or_else(|| panic!("a renderer"));
    let target = render.target().unwrap_or_else(|| panic!("a frame"));
    let image = (render.renderer.read_frame(target)).unwrap_or_else(|e| panic!("{e}"));
    let [x0, y0, x1, y1] = rect.map(|v| v as usize);
    let width = image.width as usize;
    let at = |x: usize, y: usize| {
        let i = 4 * (y * width + x);
        [image.rgba[i], image.rgba[i + 1], image.rgba[i + 2]]
    };
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| at(x, y))
        .collect()
}

/// How near `color` comes in the widget `name`: the smallest sum of the channels' differences
/// over its pixels.
pub(crate) fn nearest(ui: &mut Ui, name: &str, color: egui::Color32) -> u32 {
    let rect = ui.rect(name);
    let want = [color.r(), color.g(), color.b()];
    let off = |p: [u8; 3]| {
        (0..3)
            .map(|i| u32::from(p[i].abs_diff(want[i])))
            .sum::<u32>()
    };
    pixels(ui, rect)
        .into_iter()
        .map(off)
        .min()
        .unwrap_or(u32::MAX)
}

/// Review 0009: a button that deletes looks like no other. Part > Delete reads in the theme's
/// warning color and Duplicate beside it does not; the unsaved question's Don't save does and
/// its Save does not.
#[test]
fn delete_reads_in_the_warning_color() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.settle();
    let warning = ui.host.lock().theme().colors.warning();
    open(&mut ui, "Part");
    let delete = nearest(&mut ui, "menu > This part", warning);
    let duplicate = nearest(&mut ui, "menu > Duplicate", warning);
    assert!(delete < 40 && duplicate > 100, "{delete} and {duplicate}");
    ui.tap("Escape", Modifiers::default());
    press(&mut ui, "New…");
    let drop = nearest(&mut ui, "palette > Don't save", warning);
    let save = nearest(&mut ui, "palette > Save", warning);
    assert!(drop < 40 && save > 100, "{drop} and {save}");
}
