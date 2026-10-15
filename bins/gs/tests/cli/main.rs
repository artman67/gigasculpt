//! The real `gs` binary against real session engines, through `gs spawn`.
//!
//! This test program doubles as the engine program: started as `<this program> serve ...` it
//! runs `gs_engine::cli::main`, the exact code the `gigasculpt` binary runs. Cargo cannot hand
//! one package's tests another package's binary, and this way the engine is always built from
//! the current source. (`bins/gigasculpt/tests` drives the real `gigasculpt` binary too.)

mod arguments;
mod clients;
#[path = "../common/mod.rs"]
mod common;
mod output;
mod recording;
mod repros;
mod scripts;
mod sessions;
mod support;

use std::ffi::OsString;
use std::process::ExitCode;

use libtest_mimic::{Arguments, Trial};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "serve") {
        return gs_engine::cli::main(args);
    }
    // Listed and run the way the standard harness does, so a runner can run each case alone.
    let trials = CASES.map(|(name, case)| {
        Trial::test(name, move || {
            case();
            Ok(())
        })
    });
    libtest_mimic::run(&Arguments::from_args(), trials.into()).exit_code()
}

/// Every case, by name.
const CASES: [(&str, fn()); 25] = [
    ("spawn_call_and_stop", sessions::spawn_call_and_stop),
    (
        "spawns_output_ends_before_its_session_does",
        sessions::spawns_output_ends_before_its_session_does,
    ),
    (
        "names_stay_in_the_session_folder",
        sessions::names_stay_in_the_session_folder,
    ),
    (
        "a_session_name_too_long_for_its_socket_is_a_usage_error",
        sessions::a_session_name_too_long_for_its_socket_is_a_usage_error,
    ),
    (
        "a_stuck_session_times_out",
        sessions::a_stuck_session_times_out,
    ),
    (
        "replays_check_every_line",
        scripts::replays_check_every_line,
    ),
    ("picking_a_session", sessions::picking_a_session),
    ("spawn_failures", sessions::spawn_failures),
    (
        "arguments_from_every_source",
        arguments::arguments_from_every_source,
    ),
    ("flags_and_bad_words", arguments::flags_and_bad_words),
    (
        "exit_codes_say_what_went_wrong",
        arguments::exit_codes_say_what_went_wrong,
    ),
    (
        "text_output_keeps_every_value",
        output::text_output_keeps_every_value,
    ),
    ("errors_keep_every_detail", output::errors_keep_every_detail),
    ("scripts_and_logs_replay", scripts::scripts_and_logs_replay),
    (
        "bad_script_lines_are_named",
        scripts::bad_script_lines_are_named,
    ),
    ("listing_and_help", output::listing_and_help),
    ("query_refuses_edits", arguments::query_refuses_edits),
    (
        "named_clients_undo_only_their_own_steps",
        clients::named_clients_undo_only_their_own_steps,
    ),
    (
        "an_oversized_request_is_refused_with_e_limit",
        clients::an_oversized_request_is_refused_with_e_limit,
    ),
    (
        "repros_in_tests_folder_no_longer_fail",
        repros::every_bundle_passes,
    ),
    (
        "panic_bundles_replay_and_minimize",
        repros::panic_bundle_minimizes,
    ),
    (
        "bundles_start_from_the_newest_autosave",
        repros::bundles_start_from_the_newest_autosave,
    ),
    (
        "bundles_carry_the_files_their_lines_read",
        repros::bundles_carry_the_files_their_lines_read,
    ),
    (
        "window_logs_replay_with_every_hash_checked",
        recording::window_logs_replay_with_every_hash_checked,
    ),
    (
        "opened_files_are_found_by_content",
        recording::opened_files_are_found_by_content,
    ),
];
