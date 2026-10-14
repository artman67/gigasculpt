//! Why `gs` failed, and the exit code that says so.
//!
//! Exit codes: 0 ok, 1 internal or protocol error (or a replay that diverged), 2 bad arguments,
//! 3 precondition, busy, paused, stale or refused, 4 engine not reachable, 5 timeout.

use gs_protocol::{CommandError, ErrorCode};

/// A failed `gs` call.
pub enum Failure {
    /// The engine refused or failed the command.
    Command(Box<CommandError>),
    /// The command line itself is wrong.
    Usage(String),
    /// No engine to talk to.
    Unreachable(String),
    /// Something took too long.
    Timeout(String),
    /// The engine answered with something unexpected.
    Protocol(String),
    /// The engine refused the request for this client's role, or because it is too large.
    Refused(String),
    /// A replayed script ended up with a different scene than the log recorded.
    Diverged(String),
    /// A failure in line `line` of a script.
    Script { line: String, failure: Box<Failure> },
}

/// Makes a `map_err` function that wraps an error as `kind`, after `context`.
pub fn because<E: std::fmt::Display>(
    kind: fn(String) -> Failure,
    context: String,
) -> impl FnOnce(E) -> Failure {
    move |e| kind(format!("{context}: {e}"))
}

impl Failure {
    /// The process exit code for this failure.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Command(err) => match err.code {
                ErrorCode::UnknownCommand | ErrorCode::ArgsInvalid | ErrorCode::NotFound => 2,
                ErrorCode::Precondition
                | ErrorCode::Busy
                | ErrorCode::Paused
                | ErrorCode::Stale
                | ErrorCode::Limit
                | ErrorCode::Damaged => 3,
                ErrorCode::Timeout => 5,
                ErrorCode::Cancelled | ErrorCode::Internal => 1,
            },
            Self::Usage(_) => 2,
            Self::Refused(_) => 3,
            Self::Unreachable(_) => 4,
            Self::Timeout(_) => 5,
            Self::Protocol(_) | Self::Diverged(_) => 1,
            Self::Script { failure, .. } => failure.exit_code(),
        }
    }
}
