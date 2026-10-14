//! `visibility.grow` and `visibility.shrink` against the catalog's checks (decision 0026, 6), on
//! grids through commands the way an agent calls them: rings of faces shown and hidden, the open
//! border kept, and growing to whole pieces and whole face groups. Exact undo is the claims
//! harness's (`claims`), which runs each example.

use std::ops::Range;

use gs_testkit::{Session, fixtures};
use serde_json::json;

use crate::groups::{ids, twice};
use crate::support::agent;

/// The faces of an `n` by `n` grid (`fixtures::patch`) in rows `rows` and columns `cols`.
fn block(n: u32, rows: Range<u32>, cols: Range<u32>) -> Vec<u32> {
    rows.flat_map(|i| cols.clone().map(move |j| i * n + j))
        .collect()
}

/// The visible faces of the only part, ascending.
fn shown(s: &mut Session, part: &str) -> Vec<u32> {
    let hide = ids(&s.mesh(part), "hide");
    (0..hide.len() as u32)
        .filter(|&f| hide[f as usize] == 0)
        .collect()
}

/// Show only these faces.
fn only(s: &mut Session, faces: &[u32]) {
    let select = json!({ "select": { "by": "faces", "indexes": faces } });
    s.ok("visibility.only", select);
}

fn grid(n: u32) -> Session {
    let mut s = agent();
    s.add_part("grid", &fixtures::patch(n));
    s
}

#[test]
fn grow_shows_a_ring_of_faces_per_step_and_shrink_hides_one_keeping_the_open_border() {
    let n = 9;
    let mut s = grid(n);
    let faces = s.mesh("grid").faces.len();
    only(&mut s, &block(n, 4..5, 4..5));
    s.ok("visibility.grow", json!({}));
    assert_eq!(shown(&mut s, "grid"), block(n, 3..6, 3..6), "one ring");
    s.ok("history.undo", json!({}));
    let grown = s.ok("visibility.grow", json!({ "rings": 2 }));
    assert_eq!(shown(&mut s, "grid"), block(n, 2..7, 2..7), "two rings");
    assert_eq!(grown.result["visible"], json!(25));
    assert_eq!(s.mesh("grid").faces.len(), faces, "no face made");
    only(&mut s, &block(n, 2..6, 2..6));
    s.ok("visibility.shrink", json!({}));
    assert_eq!(
        shown(&mut s, "grid"),
        block(n, 3..5, 3..5),
        "4 by 4 to 2 by 2"
    );
    assert_eq!(s.mesh("grid").faces.len(), faces, "no face removed");
    // In the grid's corner nothing hidden is beside the border faces, so they stay.
    only(&mut s, &block(n, 0..4, 0..4));
    s.ok("visibility.shrink", json!({}));
    assert_eq!(
        shown(&mut s, "grid"),
        block(n, 0..3, 0..3),
        "the border side"
    );
    s.err("visibility.grow", json!({ "rings": 101 }));
    s.err("visibility.grow", json!({ "rings": 2, "to": "shells" }));
}

#[test]
fn grow_to_shells_shows_each_piece_with_a_visible_face_and_to_groups_each_group() {
    let n = 6;
    let mut s = agent();
    s.add_part("two", &twice(&fixtures::patch(n)));
    only(&mut s, &[7]);
    s.ok("visibility.grow", json!({ "to": "shells" }));
    let piece: Vec<u32> = (0..n * n).collect();
    assert_eq!(
        shown(&mut s, "two"),
        piece,
        "the first piece, not the second"
    );
    // Three groups of three rows each; a few faces of the middle group visible.
    let n = 9;
    let mut s = grid(n);
    for (id, rows) in [(1, 0..3), (2, 3..6), (3, 6..9)] {
        let faces = block(n, rows, 0..n);
        let select = json!({ "select": { "by": "faces", "indexes": faces }, "id": id });
        s.ok("group.assign", select);
    }
    only(&mut s, &block(n, 4..5, 2..4));
    s.ok("visibility.grow", json!({ "to": "groups" }));
    assert_eq!(
        shown(&mut s, "grid"),
        block(n, 3..6, 0..n),
        "all of group 2"
    );
}
