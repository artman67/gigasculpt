//! The window's command line and its files: which arguments open the window, and the shell
//! built from the keymap and layout files, with problems shown instead of failing.

use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gs_ui::app::{shell, wants_window};

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn no_arguments_or_window_open_the_window() {
    assert!(wants_window(&args(&[])), "no program name at all");
    assert!(wants_window(&args(&["gigasculpt"])), "no arguments");
    assert!(wants_window(&args(&["gigasculpt", "window"])));
    assert!(wants_window(&args(&[
        "gigasculpt",
        "window",
        "--session",
        "a"
    ])));
    assert!(!wants_window(&args(&["gigasculpt", "serve", "--headless"])));
    assert!(!wants_window(&args(&["gigasculpt", "--help"])));
    assert!(!wants_window(&args(&["gigasculpt", "serve", "window"])));
}

/// A fresh configuration folder for this run.
fn config(name: &str) -> PathBuf {
    let dir = crate::files::scratch(&format!("config-{name}"));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

#[test]
fn the_shell_uses_the_files_and_reports_problems_instead_of_failing() {
    let fresh = config("fresh");
    let s = shell(&fresh);
    assert_eq!(s.status.problem, None, "fresh files are no problem");
    assert!(fresh.join("keymap.toml").exists() && fresh.join("layout.toml").exists());
    let broken = config("broken");
    std::fs::write(broken.join("keymap.toml"), "[[bind]]\nkey = 3\n")
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(broken.join("layout.toml"), "ui_scale = \"big\"\n")
        .unwrap_or_else(|e| panic!("{e}"));
    let s = shell(&broken);
    let problem = s.status.problem.unwrap_or_default();
    assert!(
        problem.contains("keymap.toml") && problem.contains("layout.toml"),
        "{problem}"
    );
    assert_eq!(
        s.keymap.key_for("ui.palette"),
        Some("Space"),
        "the defaults stand in"
    );
}

#[test]
#[cfg(unix)]
fn files_that_cannot_be_read_are_reported_and_left_alone() {
    let dir = config("unreadable");
    for name in ["keymap.toml", "layout.toml"] {
        let path = dir.join(name);
        std::fs::write(&path, "# mine\n").unwrap_or_else(|e| panic!("{e}"));
        let write_only = std::fs::Permissions::from_mode(0o200);
        std::fs::set_permissions(&path, write_only).unwrap_or_else(|e| panic!("{e}"));
    }
    let s = shell(&dir);
    let problem = s.status.problem.unwrap_or_default();
    assert!(
        problem.contains("keymap.toml") && problem.contains("layout.toml"),
        "{problem}"
    );
    for name in ["keymap.toml", "layout.toml"] {
        let path = dir.join(name);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(text, "# mine\n", "{name} was not overwritten with defaults");
    }
}
