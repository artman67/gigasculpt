//! Fuzzy matching: a match exactly when the query's letters appear in order (checked against a
//! plain subsequence test), and the ranking rules the palette relies on.

use gs_ui::score;
use proptest::prelude::*;

/// The reference: is `query` (lower case, spaces dropped) a subsequence of `text`?
fn in_order(query: &str, text: &str) -> bool {
    let mut rest = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|q| rest.any(|t| t == q))
}

proptest! {
    #[test]
    fn matches_exactly_when_the_letters_appear_in_order(
        query in "[a-dA-D. ]{0,4}",
        text in "[a-d._-]{0,10}",
    ) {
        prop_assert_eq!(score(&query, &text).is_some(), in_order(&query, &text));
    }
}

fn s(query: &str, text: &str) -> i32 {
    score(query, text).unwrap_or_else(|| panic!("{query} should match {text}"))
}

#[test]
fn ranking_rules() {
    assert_eq!(score("", "anything"), Some(0), "an empty query matches all");
    assert_eq!(score("  ", "anything"), Some(0), "so does a blank one");
    assert_eq!(s("SPH", "primitive.sphere"), s("sph", "primitive.sphere"));
    assert_eq!(s("s ph", "primitive.sphere"), s("sph", "primitive.sphere"));
    assert!(s("abc", "abcxx") > s("abc", "axbxc"), "runs beat gaps");
    assert!(s("a", "a.b") > s("a", "a.bcd"), "shorter wins a tie");
    for sep in ['.', '_', '-', ' '] {
        let word = format!("xy{sep}b");
        assert!(
            s("b", &word) > s("b", "xyzb"),
            "a word starts after {sep:?}"
        );
    }
    assert!(s("b", "bxyz") > s("b", "xbyz"), "and at the start");
    assert!(score("zz", "z").is_none(), "each letter used once");
}

#[test]
fn scores_follow_the_documented_rule() {
    // "ms" in "mesh.stats": m at a word start (1 + 8), s is the next s after it, inside the
    // word and not adjacent (1): (9 + 1) * 64 - 10.
    assert_eq!(score("ms", "mesh.stats"), Some(10 * 64 - 10));
    // "pc" in "primitive.cube": p at a word start (9), c at a word start (9).
    assert_eq!(score("pc", "primitive.cube"), Some(18 * 64 - 14));
    // "ub" in "cube": u inside (1), b right after it (1 + 4).
    assert_eq!(score("ub", "cube"), Some(6 * 64 - 4));
}
