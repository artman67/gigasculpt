//! The built-ins added in step (c) (decision 0043): their ids and places, and Flat Color, which
//! has no settings.

use serde_json::{Value, json};

use super::cavity::cavity_edits;
use super::edits::edits;
use super::{ids, names, refused};
use crate::support::agent;

/// Built-ins added after the first thirteen (decision 0043, step (c)) take ids from #1001 on, in
/// the shipped order, and sit after the user materials: the first thirteen keep #1 to #13 and
/// @0 to @12, so an old log's first copy is still #14 at @13, and a later copy goes after it.
#[test]
fn later_built_ins_leave_old_ids_and_places_as_they_were() {
    let mut s = agent();
    let before = ids(&mut s);
    assert_eq!(before[..13], (1..=13).collect::<Vec<u64>>());
    let later = before[13..].to_vec();
    let block: Vec<u64> = (1001..).take(later.len()).collect();
    assert!(!later.is_empty() && later == block, "{before:?}");
    assert_eq!(names(&mut s)[13], "flat-color", "Flat Color first of them");

    let copy = s.query("material.copy", json!({ "material": "@3" }));
    assert_eq!(
        (&copy["material"]["id"], &copy["material"]["name"]),
        (&json!(14), &json!("red-clay-copy")),
        "{copy}"
    );
    let chosen = s.query("material.select", json!({ "name": "@13" }));
    assert_eq!(chosen["selected"], json!("red-clay-copy"));
    s.ok("material.copy", json!({}));
    let after = ids(&mut s);
    assert_eq!(after, [&before[..13], &[14, 15], &later[..]].concat());
}

/// Flat Color has no settings (decision 0043, step (c)): every key `material.set` takes, and
/// putting back its shipped settings, is refused with `E_PRECONDITION` and changes nothing, and
/// it cannot be deleted. It can be renamed and copied, and a copy is flat and as fixed. No other
/// command changes a recipe, which holds "loading into Flat Color changes only its name" until
/// loading exists.
#[test]
fn flat_color_refuses_every_edit_and_changes_nothing() {
    let mut s = agent();
    let before = s.query("material.list", json!({}));
    let flat = &before["looks"][13];
    assert_eq!(
        (&flat["name"], &flat["id"], &flat["kind"], &flat["built_in"]),
        (
            &json!("flat-color"),
            &json!(1001),
            &json!("flat"),
            &json!(true)
        ),
        "{flat}"
    );
    let mut tries: Vec<Value> = (edits().into_iter().chain(cavity_edits()))
        .map(|(key, given, _)| json!({ key: given }))
        .collect();
    tries.extend([
        json!({ "environment": "soft-studio" }),
        json!({ "shipped": true }),
    ]);
    for mut args in tries {
        args["material"] = json!("flat-color");
        let (got, message) = refused(&mut s, "material.set", args.clone());
        assert_eq!(got, "E_PRECONDITION", "{args}: {message}");
        assert_eq!(
            s.query("material.list", json!({})),
            before,
            "{args} changes nothing"
        );
    }
    let (got, _) = refused(
        &mut s,
        "material.delete",
        json!({ "material": "flat-color" }),
    );
    assert_eq!(got, "E_PRECONDITION", "a built-in stays");

    s.ok(
        "material.rename",
        json!({ "material": "#1001", "name": "flat", "title": "Flat" }),
    );
    let copy = s.query("material.copy", json!({ "material": "flat" }));
    assert_eq!(copy["material"]["kind"], json!("flat"), "{copy}");
    let args = json!({ "material": copy["material"]["name"], "base_color": "#ff0000" });
    let (got, _) = refused(&mut s, "material.set", args);
    assert_eq!(got, "E_PRECONDITION", "a copy is as fixed");
}
