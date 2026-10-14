//! A part's boolean operation and group start in a scene file (decision 0057, section 5): the
//! two fields are left out while they hold their defaults, so a scene saved before parts had
//! them opens with every part a union that starts no group, and saves its parts to the same
//! bytes.

use std::path::Path;

use gs_io::scene::{Kept, read};
use serde_json::json;

use crate::golden::golden;
use crate::{at, folder, save, session};

/// The version 5 file, opened and saved again, keeps every part and its properties bit for bit
/// and its scene section as it was. The session section keeps the command counter, which the
/// open itself moves, so the two files differ there and nowhere else.
#[test]
fn an_old_scene_opens_with_union_parts_and_saves_its_parts_byte_for_byte() {
    let path = golden("v5.gss");
    let mut s = session(0, None, None);
    s.ok("file.open", json!({ "path": path }));
    let dir = folder("boolean-props");
    let again = at(&dir, "again.gss");
    save(&mut s, json!({ "path": again }));
    let file = |p: &str| read(Path::new(p), Kept::None).unwrap();
    let (old, new) = (file(&path), file(&again));
    assert_eq!(
        format!("{:?}", new.parts),
        format!("{:?}", old.parts),
        "every part, its properties included"
    );
    assert_eq!((&new.writer, &new.scene), (&old.writer, &old.scene));
    let mut session = new.session.clone();
    session["counter"] = old.session["counter"].clone();
    assert_eq!(session, old.session, "the session but for its counter");
    let len = |p: &str| std::fs::metadata(p).unwrap().len();
    assert_eq!(len(&again), len(&path), "the same length");
    let list = s.query("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    assert!(!rows.is_empty(), "{list}");
    for row in &rows {
        assert_eq!(
            (&row["operation"], &row["start"]),
            (&json!("union"), &json!(false)),
            "{row}"
        );
    }
}
