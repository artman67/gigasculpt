//! Layouts are plain files. The shipped one writes out and reads back the same; a file in the
//! user's layouts folder replaces the shipped one of its name or adds a layout; a broken file
//! is refused with its file, line and key named, and the shipped one stays.

use gs_ui::{CONTROLS_DIR, Directions, to_toml};

use crate::rig::folder;

fn shipped() -> Directions {
    Directions::shipped().unwrap_or_else(|e| panic!("{e}"))
}

/// `dir` with its controls folder made.
fn with_controls(dir: std::path::PathBuf) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join(CONTROLS_DIR)).unwrap_or_else(|e| panic!("{e}"));
    dir
}

#[test]
fn layouts_save_to_files_and_load_back_and_user_files_replace_or_add() {
    let all = shipped();
    let dir = with_controls(folder("round-trip"));
    for (file, controls) in all.control_files() {
        let text = to_toml(controls).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.join(file), text).unwrap_or_else(|e| panic!("{e}"));
    }
    for d in &all.list {
        let text = to_toml(d).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.join(format!("{}.toml", d.name)), text)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let (again, problems) = Directions::load(&dir);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        (again.controls, again.list),
        (all.controls.clone(), all.list.clone()),
        "read back the same"
    );
    // An edited copy replaces the shipped layout; a new file adds one, in its order.
    let edited = to_toml(&all.list[0])
        .unwrap_or_default()
        .replace("Workbench", "My bench");
    std::fs::write(dir.join("workbench.toml"), edited).unwrap_or_else(|e| panic!("{e}"));
    let mine = "title = \"Mine\"\norder = 0\n\n[[panel]]\nid = \"a\"\ntitle = \"A\"\nedge = \"bottom\"\nsize = 40\nshow = [\"history\"]\n\n[[panel]]\nid = \"b\"\ntitle = \"B\"\nedge = \"top\"\nsize = 40\nshow = [\"symmetry\"]\n";
    std::fs::write(dir.join("mine.toml"), mine).unwrap_or_else(|e| panic!("{e}"));
    let (user, problems) = Directions::load(&dir);
    assert!(problems.is_empty(), "{problems:?}");
    let titles: Vec<&str> = user.list.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles, ["Mine", "My bench"]);
    let (none, problems) = Directions::load(&dir.join("missing"));
    assert!(
        problems.is_empty() && none.list == all.list,
        "no folder: the shipped layout"
    );
}

#[test]
fn a_user_controls_file_replaces_only_its_trays_sets_and_another_name_is_refused() {
    let all = shipped();
    let dir = with_controls(folder("tray-file"));
    let settings_file = |(f, _): &(&str, _)| *f == "controls/settings.toml";
    let (file, settings) =
        (all.control_files().find(settings_file)).unwrap_or_else(|| panic!("no settings file"));
    let edited = to_toml(settings).unwrap_or_default();
    let edited = edited.replace("title = \"Theme\"", "title = \"Themes\"");
    std::fs::write(dir.join(file), edited).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.join(CONTROLS_DIR).join("mine.toml"), "").unwrap_or_else(|e| panic!("{e}"));
    let (loaded, problems) = Directions::load(&dir);
    let problem = problems.join("; ");
    assert!(
        problem.contains("controls/mine.toml: not a tray's file"),
        "{problem}"
    );
    let titles = |d: &Directions| -> Vec<String> {
        d.controls.sets.iter().map(|s| s.title.clone()).collect()
    };
    let expected = titles(&all)
        .into_iter()
        .map(|t| if t == "Theme" { "Themes".into() } else { t });
    assert_eq!(
        titles(&loaded),
        expected.collect::<Vec<_>>(),
        "the other trays keep theirs"
    );
}

#[test]
fn a_broken_layout_file_is_refused_naming_the_key_and_the_shipped_one_stays() {
    let all = shipped();
    let workbench = to_toml(&all.list[0]).unwrap_or_default();
    let cases = [
        (
            workbench.replace("title = ", "titel = "),
            "unknown field `titel`",
        ),
        (
            workbench.replace("size = 64.0", "size = \"wide\""),
            "`size = \"wide\"`",
        ),
        (
            workbench.replace("\"recall\"", "\"recal\""),
            "show: nothing called `recal`",
        ),
        (
            workbench.replace("\"recall\"", "\"brushes/Paints\""),
            "nothing called `brushes/Paints`",
        ),
        (
            // The wheel's comes first; the Library's Brushes tab shows them too.
            workbench.replacen("show = \"brushes\"", "show = \"pens\"", 1),
            "wheel: show: nothing called `pens`",
        ),
        (
            workbench.replace("id = \"tray\"", "id = \"brush\""),
            "panel `brush`: id: used twice",
        ),
        (
            workbench.replace("\"gizmo.tool\"", "\"gizmo.on\""),
            "tool `gizmo`: when: no state value is called that",
        ),
        (
            workbench.replace("edge = \"left\"", "edge = \"middle\""),
            "`edge = \"middle\"`",
        ),
        // Panel modes went with decision 0025: a file that still has one is refused.
        (
            workbench.replace("id = \"brush\"", "id = \"brush\"\nmode = \"Sculpt\""),
            "unknown field `mode`",
        ),
    ];
    for (text, says) in cases {
        let dir = folder("broken");
        std::fs::write(dir.join("workbench.toml"), &text).unwrap_or_else(|e| panic!("{e}"));
        let (loaded, problems) = Directions::load(&dir);
        let problem = problems.join("; ");
        assert!(
            problem.contains("workbench.toml") && problem.contains(says),
            "{says}: {problem}"
        );
        let kept = loaded.get("workbench").map(|d| d.title.as_str());
        assert_eq!(kept, Some("Workbench"), "the shipped Workbench stays");
    }
    // A control that runs nothing, and one that shows a state value session.state does not
    // report: refused naming the control and the key, with the names there are.
    let mirror = "command = \"brush.set\", args = { flip_mirror = \"x\" }, shows = \"symmetry.w\"";
    let cases = [
        (
            "{ label = \"Draw\" }".to_string(),
            "control `Draw`: give `command`",
        ),
        (
            format!("{{ label = \"Mirror W\", {mirror} }}"),
            "control `Mirror W`: shows: no state value called `symmetry.w`",
        ),
    ];
    for (control, says) in cases {
        let dir = with_controls(folder("broken-controls"));
        let controls =
            format!("[[set]]\nname = \"broken\"\ntitle = \"S\"\ncontrols = [{control}]\n");
        let file = dir.join(CONTROLS_DIR).join("brush-tray.toml");
        std::fs::write(file, controls).unwrap_or_else(|e| panic!("{e}"));
        let (loaded, problems) = Directions::load(&dir);
        let problem = problems.join("; ");
        assert!(problem.contains(says), "{says}: {problem}");
        assert_eq!(loaded.controls, all.controls, "the shipped controls stay");
        assert_eq!(loaded.list.len(), 1, "and the layout with them");
    }
}
