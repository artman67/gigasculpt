//! The GigaSculpt app. With no arguments (or `window`) it opens the window; with a file
//! (`gigasculpt head.gss`, as a file manager runs it on a double-click) it opens the window on
//! that scene, or imports that mesh; with
//! `serve --headless` it runs the engine with no window, rendering offscreen looks on its own
//! device. Both command lines live in libraries (`gs_ui::app`, `gs_engine::cli`), so tests run
//! exactly this code.

// A release build on Windows opens no console window beside the app's own. Built with the
// `console` feature it is the console twin, `gigasculpt.com`, which a terminal runs for
// `gigasculpt` before `gigasculpt.exe` (decision 0063, amended 2026-10-14 (Release-prep)).
#![cfg_attr(
    not(any(debug_assertions, feature = "console")),
    windows_subsystem = "windows"
)]

fn main() -> std::process::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if gs_ui::app::wants_window(&args) {
        gs_ui::app::main(&args)
    } else {
        gs_engine::cli::main_with(args, gs_ui::Offscreen::host)
    }
}
