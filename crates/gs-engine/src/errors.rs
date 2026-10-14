//! Building `CommandError`s, "did you mean" suggestions, and explaining decoder errors.

use gs_protocol::{CommandError, ErrorCode};

use crate::Unavailable;

/// How many did-you-mean candidates an error lists at most.
const SUGGESTIONS: usize = 3;

/// Candidates less similar than this (Jaro-Winkler) are not suggested. (At 0.7, `arguments`
/// would not suggest `args`.)
const SIMILAR: f64 = 0.6;

/// An error with a code and a message, and nothing else set yet.
pub fn error(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError {
        code,
        message: message.into(),
        path: None,
        hint: None,
        suggestions: Vec::new(),
        violations: Vec::new(),
        example: None,
        retryable: false,
        retry_after_ms: None,
    }
}

/// A bug in the engine.
pub fn internal(message: &str) -> CommandError {
    error(ErrorCode::Internal, format!("internal error: {message}"))
}

/// A bad argument at JSON pointer `path`.
pub fn args_invalid(path: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        path: Some(path.to_string()),
        ..error(ErrorCode::ArgsInvalid, message)
    }
}

/// A named thing (`what` is "part", "checkpoint", ...) that does not exist, with the closest
/// existing names as suggestions.
pub fn not_found<'a>(
    what: &str,
    name: &str,
    existing: impl IntoIterator<Item = &'a str>,
) -> CommandError {
    CommandError {
        suggestions: did_you_mean(name, existing),
        ..error(
            ErrorCode::NotFound,
            format!("there is no {what} named '{name}'"),
        )
    }
}

/// A failed precondition; one that an argument would lift names it as its path.
pub fn unavailable(reason: Unavailable) -> CommandError {
    CommandError {
        hint: reason.fix,
        path: reason.asks.map(|a| format!("/{a}")),
        ..error(ErrorCode::Precondition, reason.reason)
    }
}

/// The argument a refusal asks for: a failed precondition naming an argument `args` does not
/// give. The window asks for it and runs the step again (decision 0016, section 17); a dry run
/// leaves such a control enabled.
pub fn asks<'a>(e: &'a CommandError, args: &serde_json::Value) -> Option<&'a str> {
    let name = e.path.as_deref()?.strip_prefix('/')?;
    let named = !name.is_empty() && !name.contains('/');
    let given = args.get(name).is_some_and(|v| !v.is_null());
    (e.code == ErrorCode::Precondition && named && !given).then_some(name)
}

/// The candidates similar to `word`, best first, at most three. A candidate's last dotted
/// segment counts too, so `sphere` finds `primitive.sphere`.
pub fn did_you_mean<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = candidates
        .into_iter()
        .map(|c| (similarity(word, c), c))
        .filter(|(score, _)| *score >= SIMILAR)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(SUGGESTIONS)
        .map(|(_, c)| c.to_string())
        .collect()
}

fn similarity(word: &str, candidate: &str) -> f64 {
    let last = candidate.rsplit('.').next().unwrap_or(candidate);
    strsim::jaro_winkler(word, candidate).max(strsim::jaro_winkler(word, last))
}

/// A value the decoder refused, explained: the JSON pointer of the value, the message, and for
/// a misspelled field the closest known field names.
pub struct DecodeFailure {
    pub path: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

/// Explains a decoder error at the JSON pointer of the refused value (for an unknown field,
/// the field itself). serde names an unknown field as "unknown field `x`, expected one of `a`,
/// `b`" and an unknown enum value as "unknown variant `x`, ...": the expected names become
/// did-you-mean suggestions.
pub fn explain_decode(err: &serde_path_to_error::Error<serde_json::Error>) -> DecodeFailure {
    let path = pointer(err.path());
    let message = err.inner().to_string();
    let quoted: Vec<&str> = message.split('`').skip(1).step_by(2).collect();
    // Only "unknown ..." messages quote a name followed by the names it could have been.
    let suggestions = match quoted.split_first() {
        Some((word, known)) => did_you_mean(word, known.iter().copied()),
        None => Vec::new(),
    };
    DecodeFailure {
        path,
        message,
        suggestions,
    }
}

/// A command's arguments the decoder refused, as `E_ARGS_INVALID` at the refused value. (The
/// schema check runs first and catches unknown names, so what reaches the decoder is a value
/// it cannot hold, such as a number too large.)
pub fn decode_error(err: &serde_path_to_error::Error<serde_json::Error>) -> CommandError {
    let found = explain_decode(err);
    args_invalid(&found.path, found.message)
}

/// A decoder path as a JSON pointer (RFC 6901), for example `/commands/0/args`.
fn pointer(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let mut pointer = String::new();
    for segment in path {
        let token = match segment {
            Segment::Seq { index } => index.to_string(),
            Segment::Map { key } => key.replace('~', "~0").replace('/', "~1"),
            Segment::Enum { .. } | Segment::Unknown => continue,
        };
        pointer.push('/');
        pointer.push_str(&token);
    }
    pointer
}
