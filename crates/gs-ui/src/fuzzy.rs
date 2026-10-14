//! Fuzzy matching for the command palette: the query's letters must appear in order.

/// How well `query` matches `text`, case-insensitively, or `None` if its characters do not all
/// appear in order. Each letter is matched at its first place after the previous one and
/// scores 1, plus 8 at the start of a word (after `.`, `_`, `-` or a space), plus 4 right after
/// the previous letter. The score is that total times 64, minus the text's length, so among
/// equal matches the shorter text wins. Spaces in the query are ignored; an empty query
/// matches everything with score 0.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut at = 0;
    let mut previous: Option<usize> = None;
    let mut total = 0;
    for q in query.chars().flat_map(char::to_lowercase) {
        if q.is_whitespace() {
            continue;
        }
        let found = at + text[at..].iter().position(|&c| c == q)?;
        let word_start = found == 0 || matches!(text[found - 1], '.' | '_' | '-' | ' ');
        let run = previous.is_some_and(|p| p + 1 == found);
        total += 1 + 8 * i32::from(word_start) + 4 * i32::from(run);
        previous = Some(found);
        at = found + 1;
    }
    if previous.is_none() {
        return Some(0);
    }
    Some(total * 64 - i32::try_from(text.len()).unwrap_or(i32::MAX))
}
