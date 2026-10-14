//! What a person sees typing the command line in a terminal (decision 0063, amended 2026-10-14
//! (Release-prep)). On Windows the terminal runs `gigasculpt.com`, this same program built for
//! the console (the `console` feature), so here the real binary is typed at; CI's Windows job
//! types the same commands at the twin itself, from a folder holding both programs.

use std::process::{Command, Output};

/// `gigasculpt ARGS`, as typed.
fn typed(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gigasculpt"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run gigasculpt: {e}"))
}

/// The help goes to the terminal's output, not the window; arguments after a command reach it.
#[test]
fn typed_help_prints_the_usage_and_each_commands_options() {
    let help = typed(&["--help"]);
    let said = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.status.success() && said.contains("Usage: gigasculpt") && said.contains("serve"),
        "gigasculpt --help: {help:?}"
    );
    let serve = typed(&["serve", "--help"]);
    let said = String::from_utf8_lossy(&serve.stdout);
    assert!(
        serve.status.success() && said.contains("--headless") && said.contains("Folders"),
        "gigasculpt serve --help: {serve:?}"
    );
}
