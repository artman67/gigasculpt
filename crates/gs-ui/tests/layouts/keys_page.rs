//! The public keys page (`publish/public/docs/keys.md`), written from the built-in keymap in
//! the words of Settings' Keys page: each key with what it does, under the keymap's groups,
//! then what the held keys do to a stroke. With `GS_WRITE_DOCS=1` the test writes the page
//! again; without it, a page that no longer matches the keys fails, naming that command.

use std::path::Path;

use serde_json::json;

use crate::rig::Ui;

/// The page, from the repository's root.
const PAGE: &str = "publish/public/docs/keys.md";

/// The command that writes the page again.
const WRITE: &str =
    "GS_WRITE_DOCS=1 gate/slot.sh cargo nextest run -p gs-ui --test layouts keys_page";

/// The keymap's groups, by the keys sheet's headings, as the page titles them.
const GROUPS: [(&str, &str); 5] = [
    ("General keys", "General"),
    ("View keys", "View"),
    ("Sculpt keys", "Brushes and sculpting"),
    ("Mask keys", "Mask and visibility"),
    ("Geometry keys", "Geometry"),
];

/// The page's top, before the groups.
const TOP: &str = "# Keys

These are the keys GigaSculpt starts with. Your own changes, made in Window > Settings > Keys,
override them, and that page always lists the keys as they are now.

Keys joined by `+` are pressed together. Keys separated by spaces are pressed one after
another: `B S T` is B, then S, then T, and picks the Draw brush. Every brush has a key like
that, B and two letters from its name, and the pointer on a brush's button shows its key.

This page is written from the program's built-in keys.
";

/// The heading and words of the held keys, after the groups.
const HELD: &str = "Held while you stroke

Held while the pen strokes, Shift, Ctrl and Ctrl+Shift stroke with the brush on that key's
slot instead of the brush you picked, and Alt makes the stroke work the other way. These are
the brushes the slots start with.";

/// One row of a table: `key`, then what it does.
fn row(key: &str, does: &str) -> String {
    format!("| `{key}` | {} |\n", does.replace('|', "\\|"))
}

/// A group's heading, `title` (with its words, if any), and its table's heading row.
fn heading(title: &str) -> String {
    format!("\n## {title}\n\n| Key | What it does |\n|---|---|\n")
}

/// The page, from the keys sheet's lines (`lines`): the held keys' lines come before the
/// first group's heading, and the sheet ends at the window's Reset page.
fn page(lines: &[String]) -> String {
    let first = lines.iter().position(|l| l == GROUPS[0].0);
    let first = first.unwrap_or_else(|| panic!("no keys sheet in {lines:?}"));
    let end = (lines.iter().position(|l| l == "Reset page")).unwrap_or(lines.len());
    let groups = lines[first..end].iter().filter_map(|line| {
        match (
            GROUPS.iter().find(|(h, _)| h == line),
            line.split_once(": "),
        ) {
            (Some((_, title)), _) => Some(heading(title)),
            (None, Some((key, does))) => Some(row(key, does)),
            (None, None) => None,
        }
    });
    let held = (lines[..first].iter()).filter_map(|l| l.split_once(" held: "));
    (std::iter::once(TOP.to_string()).chain(groups))
        .chain([heading(HELD)])
        .chain(held.map(|(keys, does)| row(keys, does)))
        .collect()
}

/// The keys page says what the built-in keys do, as Settings' Keys page says it in a new
/// window, whose keymap has no change.
#[test]
fn the_keys_page_is_written_from_the_built_in_keys() {
    let mut ui = Ui::new();
    // Tall enough for the whole sheet: `ui.describe` lists only what shows.
    ui.host.lock().resize([1800, 8000], 1.0);
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    let written = page(&ui.labels("settings"));
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(PAGE);
    if std::env::var_os("GS_WRITE_DOCS").is_some_and(|v| v == "1") {
        std::fs::write(&path, &written).unwrap_or_else(|e| panic!("{PAGE}: {e}"));
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == written,
        "{PAGE} does not match the built-in keys; write it again with: {WRITE}"
    );
}
