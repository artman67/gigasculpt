//! The form's words: an argument's name and its choices in plain words, and its hint.

use super::kind::{Kind, Word};

/// The words of the commands' argument names and choices that are written otherwise than in
/// lower case, as each is written: the initialisms, and `AgX`, a name.
const SPELLED: [(&str, &str); 14] = [
    ("agx", "AgX"),
    ("ascii", "ASCII"),
    ("fov", "FOV"),
    ("id", "ID"),
    ("ids", "IDs"),
    ("ior", "IOR"),
    ("obj", "OBJ"),
    ("pbr", "PBR"),
    ("ply", "PLY"),
    ("stl", "STL"),
    ("uv", "UV"),
    ("uvs", "UVs"),
    ("zpr", "ZPR"),
    ("ztl", "ZTL"),
];

/// A word as its button reads: `side` beside an axis as the side it keeps (Keep +X side), a
/// letter as a capital, others in plain words (`uv` UV).
pub(super) fn word_label(name: &str, word: &str, axis: &str) -> String {
    match (name, word) {
        ("side", "positive") => format!("Keep +{axis} side"),
        ("side", "negative") => format!("Keep -{axis} side"),
        (_, w) if w.chars().count() == 1 => w.to_uppercase(),
        (_, w) => friendly(w),
    }
}

/// What a question's first button says: Replace for `overwrite`, Save for `upgrade`; else the
/// switch's own name. (Unsaved changes have their own question, `palette::unsaved`.)
pub(super) fn yes(name: &str) -> String {
    match name {
        "overwrite" => "Replace".into(),
        "upgrade" => "Save".into(),
        _ => friendly(name),
    }
}

/// A question's line: its own words about `file` ([`own_line`]), else the switch's
/// description `about`.
pub(super) fn line(name: &str, about: &str, file: Option<&str>) -> String {
    own_line(name, file).unwrap_or_else(|| about.into())
}

/// What a question asks of `file`, the file it is about, in words that say why it asks, for
/// `overwrite` and `upgrade` (review 0009, B2 and B3); none for the others.
pub(in crate::palette) fn own_line(name: &str, file: Option<&str>) -> Option<String> {
    let file = file.and_then(|f| std::path::Path::new(f).file_name()?.to_str());
    match (name, file?) {
        ("overwrite", file) => Some(format!("{file} exists. Replace it?")),
        ("upgrade", file) => Some(format!(
            "{file} was saved in an older format. Save it in the new one? Older versions of \
             GigaSculpt cannot open it then."
        )),
        _ => None,
    }
}

/// `name` in plain words, as the form labels it: spaces for underscores and dots, each word
/// [`SPELLED`] lists as it is written there (UV, `smooth_uvs` Smooth UVs, `pbr-neutral`
/// PBR-neutral), its first letter a capital.
pub(super) fn friendly(name: &str) -> String {
    let words = spelled(&name.replace(['_', '.'], " "));
    let mut chars = words.chars();
    let first = chars.next().map(|c| c.to_uppercase().to_string());
    first.unwrap_or_default() + chars.as_str()
}

/// `text` with each whole word [`SPELLED`] lists as it is written there (the UV form's poles):
/// a word runs between spaces, dashes and punctuation, so a quoted or backticked one, which
/// names a value or an argument (`` `id` ``), stays as it is.
fn spelled(text: &str) -> String {
    let edge = |c: char| " -(),.:;".contains(c);
    (text.split_inclusive(edge))
        .map(|word| {
            let bare = word.trim_end_matches(edge);
            let spelled = SPELLED.iter().find(|(w, _)| *w == bare);
            spelled.map_or_else(
                || word.into(),
                |(_, s)| format!("{s}{}", &word[bare.len()..]),
            )
        })
        .collect()
}

/// What a field says under its name and on hover: the first sentence of its description `about`
/// (decision 0025, rule 13), its initialisms in capitals ([`spelled`]). Under a row of words,
/// whose buttons show them already, a list of them is left out (`: "x", "y" or "z"`); where
/// that is the whole sentence, the next shows instead, unless it too lists them or is about the
/// argument left out, which a lit button shows. Agents read the description whole.
pub(super) fn hint(about: &str, kind: &Kind) -> String {
    spelled(&said(about, kind))
}

/// [`hint`]'s sentence, as the description writes it.
fn said(about: &str, kind: &Kind) -> String {
    let mut sentences = (about.split_inclusive(". ")).map(str::trim);
    let first = sentences.next().unwrap_or_default();
    let (Kind::Words(words, _) | Kind::WordsOrNumber(words)) = kind else {
        return first.into();
    };
    let (head, tail) = first.rsplit_once(": ").unwrap_or(("", first));
    if !lists(tail, words) {
        return first.into();
    }
    if !head.is_empty() {
        return format!("{head}.");
    }
    let next = sentences.next().unwrap_or_default();
    let left_out = next.starts_with("Left out") || next.starts_with("Defaults to");
    if left_out || lists(next, words) {
        String::new()
    } else {
        next.into()
    }
}

/// Whether `text` only lists some of `words`, each in quotes or bare: `"cube" or "uv".`,
/// `visible (the default) or all`.
fn lists(text: &str, words: &[Word]) -> bool {
    let text = text.trim_end_matches('.');
    let mut items = (text.split(", ")).flat_map(|t| t.split(" or ").flat_map(|t| t.split(" and ")));
    let word = |item: &str| {
        let item = item
            .trim()
            .trim_start_matches("or ")
            .trim_start_matches("and ");
        let item = item.trim_end_matches(" (the default)").trim_matches('"');
        words.iter().any(|w| w.value == item)
    };
    !text.is_empty() && items.all(word)
}
