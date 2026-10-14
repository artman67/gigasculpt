//! Face group colors (decision 0039, section 9) through gs-ui's render hosts: the window shows
//! every part's groups while the session's `face_groups` is on, and `view.snapshot` and
//! `view.contact_sheet` show them only when they pass `face_groups`, whatever the window does.
//! A part with no groups draws its material either way.

use serde_json::json;

use crate::canvas::{at, shot, studio};
use crate::looks::{Shot, headless, run, snapshot};

/// The columns of the pixels where `a` and `b` differ.
fn changed_columns(a: &Shot, b: &Shot) -> Vec<u32> {
    let height = (a.1.len() / 4) as u32 / a.0;
    (0..height)
        .flat_map(|y| (0..a.0).map(move |x| (x, y)))
        .filter(|&(x, y)| at(a, x, y) != at(b, x, y))
        .map(|(x, _)| x)
        .collect()
}

/// A look shows face groups only when it passes `face_groups`, so no look changes with the
/// window's setting; the cube, cut into six groups, changes color, and the sphere beside it,
/// with none, does not.
#[test]
fn looks_show_face_groups_only_when_they_ask() {
    // A sphere and, to its right, the cube, which is active and gets one group a side.
    let mut e = headless();
    run(
        &mut e,
        "group.make",
        json!({ "maker": { "by": "normals" } }),
    );
    let front = json!({ "view": "front", "size": 128, "antialias": false });
    let grouped = json!({ "view": "front", "size": 128, "antialias": false, "face_groups": true });
    let sheet = |e: &mut _, groups: bool| {
        let args = json!({ "views": ["front", "top"], "size": 128, "face_groups": groups });
        run(e, "view.contact_sheet", args)["png_base64"].clone()
    };
    let plain = snapshot(&mut e, front.clone());
    let plain_sheet = sheet(&mut e, false);
    let groups = snapshot(&mut e, grouped.clone());
    let columns = changed_columns(&plain, &groups);
    assert!(
        columns.len() > 500,
        "the groups show: {} pixels",
        columns.len()
    );
    // The front view frames both parts, so the gap between them is the picture's middle.
    assert!(
        columns.iter().all(|&x| x > plain.0 / 2),
        "only the cube, right of the middle, changes"
    );
    assert_ne!(sheet(&mut e, true), plain_sheet, "a sheet shows them too");

    run(&mut e, "view.display", json!({ "face_groups": true }));
    assert_eq!(snapshot(&mut e, front), plain, "left out: none");
    assert_eq!(sheet(&mut e, false), plain_sheet, "nor on a sheet");
    assert_eq!(
        snapshot(&mut e, grouped),
        groups,
        "asked for, the window's setting changes nothing"
    );
}

/// The window shows the groups while the setting is on and draws as before when it is off.
/// Only the 3D view's middle is compared: the status line names the last command.
#[test]
fn the_window_shows_face_groups_while_the_setting_is_on() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args| crate::canvas::run(e, false, command, args);
    run(&mut engine, "primitive.cube", json!({}));
    run(
        &mut engine,
        "group.make",
        json!({ "maker": { "by": "normals" } }),
    );
    run(&mut engine, "view.frame", json!({}));
    let [left, top, right, bottom] = crate::canvas::rect(&crate::canvas::view(&mut engine));
    let middle = |a: &Shot, b: &Shot| {
        let (columns, rows) = (
            left as u32 + 60..right as u32 - 60,
            top as u32..bottom as u32,
        );
        (rows.flat_map(|y| columns.clone().map(move |x| (x, y))))
            .filter(|&(x, y)| at(a, x, y) != at(b, x, y))
            .count()
    };
    let before = shot(&mut engine);
    run(&mut engine, "view.display", json!({ "face_groups": true }));
    let groups = shot(&mut engine);
    assert!(middle(&before, &groups) > 1000, "the groups show");
    run(&mut engine, "view.display", json!({ "face_groups": false }));
    assert_eq!(middle(&before, &shot(&mut engine)), 0, "off: as before");
}
