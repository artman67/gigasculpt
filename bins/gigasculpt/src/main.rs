//! The GigaSculpt app. With no arguments (or `window`) it opens the window; with
//! `serve --headless` it runs the engine with no window, rendering offscreen looks on its own
//! device. Both command lines live in libraries (`gs_ui::app`, `gs_engine::cli`), so tests run
//! exactly this code.

// A release build on Windows opens no console window beside the app's own.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if gs_ui::app::wants_window(&args) {
        gs_ui::app::main(&args)
    } else {
        gs_engine::cli::main_with(args, gs_ui::Offscreen::host)
    }
}
