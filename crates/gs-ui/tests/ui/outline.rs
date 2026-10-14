//! The active part's outline (decision 0033, section 5) through gs-ui's render hosts: the
//! window outlines the active part while its view draws two parts or more and the session's
//! `active_outline` is on, and the outline follows `part.select`; `view.snapshot` and
//! `view.contact_sheet` outline it only when they pass `active_outline`, whatever the window
//! does.

use serde_json::{Value, json};

use crate::canvas::{at, shot, studio};
use crate::looks::{Shot, headless, run, snapshot};

/// The pixels where `a` and `b` differ, among `points`.
fn changed(a: &Shot, b: &Shot, points: &[(u32, u32)]) -> Vec<(u32, u32)> {
    (points.iter().copied())
        .filter(|&(x, y)| at(a, x, y) != at(b, x, y))
        .collect()
}

/// The outline's orange, #FFAA40, give or take the dither.
fn orange(p: [u8; 4]) -> bool {
    let close = |v: u8, to: i32| (i32::from(v) - to).abs() <= 1;
    close(p[0], 255) && close(p[1], 170) && close(p[2], 64)
}

/// The column between two parts side by side in `plain`, among `points`: the middle of the
/// widest run of columns no surface covers, between the first covered column and the last.
fn gap(plain: &Shot, points: &[(u32, u32)]) -> u32 {
    let background = gs_render::BACKGROUND;
    let mut covered: Vec<u32> = (points.iter())
        .filter(|&&(x, y)| at(plain, x, y) != background)
        .map(|p| p.0)
        .collect();
    covered.sort_unstable();
    covered.dedup();
    let widest = covered.windows(2).max_by_key(|w| w[1] - w[0]);
    let widest = widest.unwrap_or_else(|| panic!("no two parts side by side"));
    assert!(widest[1] - widest[0] > 4, "the parts touch: {widest:?}");
    u32::midpoint(widest[0], widest[1])
}

/// Checks that `outlined` differs from `plain` only by an outline around the part on the
/// `right` (else left) of column `split`: each changed pixel was background, within 2 pixels of
/// a covered one and on that side, and many are the outline's orange.
fn outlines_one_side(plain: &Shot, outlined: &Shot, points: &[(u32, u32)], right: bool) {
    let split = gap(plain, points);
    let changed = changed(plain, outlined, points);
    let background = gs_render::BACKGROUND;
    let covered = |x: u32, y: u32| at(plain, x, y) != background;
    let near = |&(x, y): &(u32, u32)| {
        let around = (y.saturating_sub(2)..=y + 2)
            .flat_map(|v| (x.saturating_sub(2)..=x + 2).map(move |u| (u, v)));
        around
            .filter(|p| points.contains(p))
            .any(|(u, v)| covered(u, v))
    };
    assert!(changed.len() > 100, "an outline: {} pixels", changed.len());
    assert!(
        changed.iter().all(|&(x, y)| !covered(x, y)),
        "no part's own pixels change"
    );
    assert!(changed.iter().all(near), "the outline hugs the part");
    let side = |&&(x, _): &&(u32, u32)| (x > split) == right;
    assert_eq!(
        changed.iter().filter(|p| !side(p)).count(),
        0,
        "only the active part, the one {} of column {split}, is outlined",
        if right { "right" } else { "left" }
    );
    let lit = changed
        .iter()
        .filter(|&&(x, y)| orange(at(outlined, x, y)))
        .count();
    assert!(lit > 50, "in orange: {lit} of {} pixels", changed.len());
}

/// Every pixel of a picture.
fn everywhere(shot: &Shot) -> Vec<(u32, u32)> {
    let height = (shot.1.len() / 4) as u32 / shot.0;
    (0..height)
        .flat_map(|y| (0..shot.0).map(move |x| (x, y)))
        .collect()
}

/// A look outlines the active part only when it passes `active_outline`, so no look changes
/// with the window's setting; the outline goes round the active part alone, follows
/// `part.select`, and is not drawn while the look shows one part.
#[test]
fn looks_outline_the_active_part_only_when_they_ask() {
    // A sphere and, to its right, a cube, made last and so active. One sample per pixel, so a
    // pixel is a part's or not; sheets keep their smooth edges.
    let mut e = headless();
    let front = json!({ "view": "front", "size": 128, "antialias": false });
    let outlined =
        json!({ "view": "front", "size": 128, "antialias": false, "active_outline": true });
    let sheet = |e: &mut _, outline: bool| {
        let args = json!({ "views": ["front", "top"], "size": 128, "active_outline": outline });
        run(e, "view.contact_sheet", args)["png_base64"].clone()
    };
    let plain = snapshot(&mut e, front.clone());
    let plain_sheet = sheet(&mut e, false);
    let points = everywhere(&plain);

    let cube = snapshot(&mut e, outlined.clone());
    outlines_one_side(&plain, &cube, &points, true);
    assert_ne!(sheet(&mut e, true), plain_sheet, "a sheet outlines it too");

    run(&mut e, "view.display", json!({ "active_outline": false }));
    assert_eq!(snapshot(&mut e, front.clone()), plain, "left out: none");
    assert_eq!(sheet(&mut e, false), plain_sheet, "nor on a sheet");
    assert_eq!(
        snapshot(&mut e, outlined.clone()),
        cube,
        "asked for, the window's setting changes nothing"
    );

    run(&mut e, "part.select", json!({ "part": "sphere" }));
    assert_eq!(
        snapshot(&mut e, front.clone()),
        plain,
        "selecting alone draws nothing"
    );
    let sphere = snapshot(&mut e, outlined.clone());
    outlines_one_side(&plain, &sphere, &points, false);

    let hide = json!({ "parts": ["cube"], "visible": false });
    run(&mut e, "part.set_visible", hide);
    assert_eq!(
        snapshot(&mut e, outlined),
        snapshot(&mut e, front),
        "one part shown: no outline"
    );
}

/// The window outlines the active part while its view draws two parts or more and the setting
/// is on: one part draws the same with it on or off, two outline the active one, the outline
/// follows `part.select`, and off draws the scene as before. Only the 3D view's middle is
/// compared: the status line names the last command.
#[test]
fn the_window_outlines_the_active_part_and_follows_it() {
    let (mut engine, _host) = studio(1.0);
    let run = |e: &mut _, command: &str, args: Value| crate::canvas::run(e, false, command, args);
    let outline = |e: &mut _, on: bool| {
        run(e, "view.display", json!({ "active_outline": on }));
        shot(e)
    };
    run(
        &mut engine,
        "primitive.sphere",
        json!({ "subdivisions": 6 }),
    );
    let [left, top, right, bottom] = crate::canvas::rect(&crate::canvas::view(&mut engine));
    let (columns, rows) = (
        left as u32 + 60..right as u32 - 60,
        top as u32..bottom as u32,
    );
    let points: Vec<(u32, u32)> = rows
        .flat_map(|y| columns.clone().map(move |x| (x, y)))
        .collect();

    let one = shot(&mut engine);
    let off = outline(&mut engine, false);
    let moved = changed(&one, &off, &points);
    assert!(
        moved.is_empty(),
        "one part: nothing to outline, {} moved",
        moved.len()
    );
    outline(&mut engine, true);

    run(&mut engine, "primitive.cube", json!({}));
    let right_of = json!({ "part": "cube", "direction": [1, 0, 0], "distance": 2.5 });
    run(&mut engine, "mesh.translate", right_of);
    run(&mut engine, "view.frame", json!({}));
    let cube = shot(&mut engine);
    let plain = outline(&mut engine, false);
    outlines_one_side(&plain, &cube, &points, true);

    run(&mut engine, "part.select", json!({ "part": "sphere" }));
    let still = shot(&mut engine);
    let moved = changed(&plain, &still, &points);
    assert!(
        moved.is_empty(),
        "off: selecting draws as before, {} moved",
        moved.len()
    );
    let sphere = outline(&mut engine, true);
    outlines_one_side(&plain, &sphere, &points, false);
}
