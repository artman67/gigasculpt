//! The keymap and layout are plain files: the keymap holds only the user's changes over the
//! built-in keys and an old full one is moved aside once; both are written when missing, read
//! back as edited, refused with the reason when broken, and the layout is saved when it
//! changes.

use std::path::PathBuf;

use egui::{Key, Modifiers};
use gs_ui::{Chord, Keymap, Layout, LayoutFile};

/// A path for this run's files, not yet made.
pub fn scratch(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("ui-files-{}-{name}", std::process::id()))
}

/// What `chord` runs in `keymap`, if anything.
fn runs(keymap: &Keymap, chord: &str) -> Option<String> {
    let chord = Chord::parse(chord).unwrap_or_else(|e| panic!("{e}"));
    keymap.lookup(&chord).map(|b| b.command.clone())
}

#[test]
fn the_keymap_file_holds_only_changes_over_the_built_in_keys() {
    let path = scratch("keymap").join("keymap.toml");
    let (first, moved) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (first, moved),
        (Keymap::default(), None),
        "a new file: the defaults"
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(text.ends_with("version = 2\n"), "and no bindings: {text}");
    // The palette moves to Tab, Space is unbound, and F frames the mask.
    let changes = "\n[[bind]]\nkey = \"Tab\"\ncommand = \"ui.palette\"\n\
                   \n[[bind]]\nkey = \"Space\"\ncommand = \"\"\n\
                   \n[[bind]]\nkey = \"F\"\ncommand = \"view.frame\"\nargs = { target = \"mask\" }\n";
    std::fs::write(&path, format!("{text}{changes}")).unwrap_or_else(|e| panic!("{e}"));
    let (keymap, _) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(runs(&keymap, "Tab").as_deref(), Some("ui.palette"));
    assert_eq!(runs(&keymap, "Space"), None, "an empty command unbinds");
    let frames = keymap.bindings.iter().filter(|b| b.key == "F");
    let args: Vec<_> = frames.map(|b| b.args.to_string()).collect();
    assert_eq!(
        args,
        ["target = \"mask\"\n"],
        "the change replaces the built-in F"
    );
    assert_eq!(
        runs(&keymap, "Ctrl+Z").as_deref(),
        Some("history.undo"),
        "every other built-in key still applies"
    );
}

#[test]
fn an_old_keymap_file_is_moved_aside_once() {
    let dir = scratch("old");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("keymap.toml");
    // The old rule wrote every default key, Ctrl+1 for a layout among them.
    let old = "[[bind]]\nkey = \"Ctrl+1\"\ncommand = \"ui.layout\"\n";
    std::fs::write(&path, old).unwrap_or_else(|e| panic!("{e}"));
    let (keymap, moved) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(keymap, Keymap::default(), "the built-in keys apply");
    assert!(
        moved.is_some_and(|m| m.contains("keymap-before-0025.toml")),
        "the status line says where it went"
    );
    let before = std::fs::read_to_string(dir.join("keymap-before-0025.toml"));
    assert_eq!(before.ok().as_deref(), Some(old), "kept whole");
    let (_, again) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again, None, "only once");
}

#[test]
fn a_broken_keymap_is_refused_with_its_reason() {
    let dir = scratch("broken");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("keymap.toml");
    for (text, reason) in [
        (
            "[[bind]]\nkey = \"Ctrl+Nope\"\ncommand = \"x\"\n",
            "unknown key 'Nope'",
        ),
        (
            "[[bind]]\nkey = \"Hyper+A\"\ncommand = \"x\"\n",
            "unknown modifier 'hyper'",
        ),
        ("[[bind]]\nkey = \"A\"\n", "missing field `command`"),
        ("version = 3\n", "version 3 is unknown"),
    ] {
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
        let err = Keymap::load(&path).err().unwrap_or_default();
        assert!(err.contains(reason), "{err} should say {reason}");
        assert_eq!(
            std::fs::read_to_string(&path).ok().as_deref(),
            Some(text),
            "a broken file is left alone"
        );
    }
    let unreadable = dir.join("is-a-folder");
    std::fs::create_dir_all(&unreadable).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        Keymap::load(&unreadable).is_err(),
        "a folder is not a keymap"
    );
    assert!(LayoutFile::load(&unreadable).is_err(), "nor a layout");
}

/// R8 (decision 0025, section 5): a change naming a command the program does not have is
/// refused alone. Its key keeps its built-in binding, the other changes load, and the window's
/// status line names the key, the command and the nearest name.
#[test]
fn a_binding_to_a_command_that_does_not_exist_is_refused_alone_and_named() {
    let dir = scratch("unknown");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("keymap.toml");
    let text = "version = 2\n\n[[bind]]\nkey = \"Ctrl+D\"\ncommand = \"level.subdivid\"\n\
                \n[[bind]]\nkey = \"Tab\"\ncommand = \"ui.palette\"\n";
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
    let (keymap, said) = Keymap::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        runs(&keymap, "Tab").as_deref(),
        Some("ui.palette"),
        "the rest load"
    );
    assert_eq!(
        runs(&keymap, "Ctrl+D").as_deref(),
        Some("level.subdivide"),
        "the key keeps its built-in binding"
    );
    let said = said.unwrap_or_default();
    let named = ["Ctrl+D", "level.subdivid,", "did you mean level.subdivide"];
    assert!(named.iter().all(|n| said.contains(n)), "{said}");
    let shell = gs_ui::app::shell(&dir);
    let problem = shell.status.problem.unwrap_or_default();
    assert!(
        problem.contains("Ctrl+D") && problem.contains("level.subdivid,"),
        "the window says so: {problem}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).ok().as_deref(),
        Some(text),
        "the file is left as written"
    );
}

#[test]
fn chords_read_and_write_their_modifiers_in_any_order() {
    let chord = Chord::parse("shift + Ctrl+Z").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(chord.to_string(), "Ctrl+Shift+Z");
    let all = Chord::parse("Super+Alt+Shift+Ctrl+F2").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(all.to_string(), "Ctrl+Shift+Alt+Super+F2");
    let pressed = Modifiers {
        alt: true,
        ctrl: true,
        shift: false,
        mac_cmd: true,
        command: true,
    };
    let from_keys = Chord::pressed(Key::F2, pressed);
    assert_eq!(from_keys.to_string(), "Ctrl+Alt+Super+F2");
}

#[test]
fn the_layout_is_saved_when_it_changes() {
    let path = scratch("layout").join("layout.toml");
    let mut file = LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(file.layout, Layout::default());
    assert!(path.exists(), "written on first load");
    std::fs::remove_file(&path).unwrap_or_else(|e| panic!("{e}"));
    file.save_if_changed().unwrap_or_else(|e| panic!("{e}"));
    assert!(!path.exists(), "nothing changed, nothing written");
    file.layout.hand = gs_engine::Hand::Left;
    file.save_if_changed().unwrap_or_else(|e| panic!("{e}"));
    let again = LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    let left = Layout {
        hand: gs_engine::Hand::Left,
        ..Layout::default()
    };
    assert_eq!(again.layout, left, "the change was saved");
    // A file from before decision 0025 names a mode: it still loads, and the mode is dropped.
    std::fs::write(&path, "current = \"workbench\"\nmode = \"Sculpt\"\n")
        .unwrap_or_else(|e| panic!("{e}"));
    let old = LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(old.layout.current, "workbench");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!text.contains("mode"), "written back without it: {text}");
    std::fs::write(&path, "ui_scale = \"wide\"\n").unwrap_or_else(|e| panic!("{e}"));
    let err = LayoutFile::load(&path).err().unwrap_or_default();
    assert!(
        err.contains("layout.toml") && err.contains("line 1 `ui_scale = \"wide\"`"),
        "the reason names the file, the line and the key: {err}"
    );
}

/// The window's own files that GigaSculpt may not read (the layout file, the preferences, a
/// theme, a layout of the user's) say so at start in a person's words, naming the file; never
/// the system's text with its error number.
#[test]
#[cfg(unix)]
fn the_windows_own_files_it_may_not_read_say_so() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("not-allowed");
    let shut = |name: &str| {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap_or(&dir)).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&path, "").unwrap_or_else(|e| panic!("{e}"));
        let mode = std::fs::Permissions::from_mode(0o000);
        std::fs::set_permissions(&path, mode).unwrap_or_else(|e| panic!("{e}"));
        path
    };
    let layout = LayoutFile::load(&shut("layout.toml")).err();
    let (_, preferences) = gs_ui::Preferences::load(&shut("preferences.toml"));
    let defaults = |p: &String| p.contains("the defaults are used");
    assert!(
        preferences.as_ref().is_some_and(defaults),
        "{preferences:?}"
    );
    shut("themes/shut-theme.toml");
    let (_, themes) = gs_ui::Themes::load(&dir.join("themes"));
    shut("layouts/shut-layout.toml");
    let (_, layouts) = gs_ui::Directions::load(&dir.join("layouts"));
    let said = [
        ("layout.toml", layout),
        ("preferences.toml", preferences),
        ("shut-theme.toml", themes.into_iter().next()),
        ("shut-layout.toml", layouts.into_iter().next()),
    ];
    for (file, said) in said {
        let said = said.unwrap_or_else(|| panic!("{file}: nothing said"));
        assert!(said.contains(file), "{said}");
        assert!(said.contains("not allowed to read"), "{said}");
        assert!(!said.contains("os error"), "{said}");
        assert_eq!(said.matches(file).count(), 1, "the file named once: {said}");
    }
}
