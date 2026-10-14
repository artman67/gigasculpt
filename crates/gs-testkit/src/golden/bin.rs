//! The `golden` tool; see `gs_testkit::golden::cli`.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (text, code) = match gs_testkit::golden::cli::run(&args) {
        Ok(text) => (text, ExitCode::SUCCESS),
        Err(text) => (text, ExitCode::FAILURE),
    };
    match writeln!(std::io::stdout(), "{text}") {
        Ok(()) => code,
        Err(_) => ExitCode::FAILURE,
    }
}
