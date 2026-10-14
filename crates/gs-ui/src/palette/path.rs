//! What a person types for a file is the path the form sends (decision 0016, section 17,
//! amended 2026-10-13): a full path typed after the folder a field starts in replaces that
//! folder, one pair of quotes round it goes (a file manager copies a path with spaces so), and
//! a leading `~` is the home folder.

use std::path::{MAIN_SEPARATOR, Path};

/// Whether the argument `name` is a file or a folder: what this module reads.
pub(super) fn named(name: &str) -> bool {
    matches!(name, "path" | "file" | "folder")
}

/// The path `text` names, typed into a field that started with `start`.
pub(super) fn typed(start: &str, text: &str) -> String {
    let text = text.trim();
    let after = (text.strip_prefix(start)).filter(|_| !start.is_empty());
    home(after.and_then(whole).unwrap_or_else(|| unquoted(text)))
}

/// `text` unquoted, if it is a whole path: absolute, or from the home folder.
fn whole(text: &str) -> Option<&str> {
    let text = unquoted(text.trim());
    (Path::new(text).is_absolute() || text.starts_with('~')).then_some(text)
}

/// `text` without one pair of matching quotes round it.
fn unquoted(text: &str) -> &str {
    let inside = |quote| text.strip_prefix(quote)?.strip_suffix(quote);
    inside('"').or_else(|| inside('\'')).unwrap_or(text)
}

/// `path` with a leading `~` (alone, or before a separator) as the home folder.
fn home(path: &str) -> String {
    let rest = path.strip_prefix('~');
    let rest = rest.filter(|r| r.is_empty() || r.starts_with(['/', MAIN_SEPARATOR]));
    match (rest, std::env::home_dir()) {
        (Some(rest), Some(home)) => format!("{}{rest}", home.display()),
        _ => path.to_string(),
    }
}
