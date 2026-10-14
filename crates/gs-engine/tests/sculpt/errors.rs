//! Strokes and picks that cannot work are refused with the argument at fault, and change
//! nothing (`Session::err` checks the scene stayed as it was).

use gs_testkit::code;
use serde_json::{Value, json};

use crate::sphere_session;

/// The error code and path of a call that must fail.
fn refused(command: &str, args: Value) -> (String, Option<String>) {
    let mut s = sphere_session(None);
    let err = s.err(command, args);
    (code(err.code), err.path)
}

#[test]
fn strokes_say_which_argument_is_wrong() {
    let screen = json!([{ "position": [500, 500] }]);
    let world = json!([{ "position": [0, 0, 1] }]);
    for (args, path) in [
        (
            json!({ "brush": "Draw", "samples": screen.clone() }),
            "/samples/0/position",
        ),
        (
            json!({ "brush": "Draw", "samples": world.clone(), "camera": "front" }),
            "/samples/0/position",
        ),
        (
            json!({ "brush": "Draw", "samples": world.clone(), "size_px": 30 }),
            "/size_px",
        ),
        (
            json!({ "brush": "Draw", "samples": screen.clone(), "camera": "front", "size": 0.1, "size_px": 3 }),
            "/size",
        ),
        (
            json!({ "brush": "Draw", "samples": screen.clone(), "camera": "front", "view_dir": [0, 0, -1] }),
            "/view_dir",
        ),
        (
            json!({ "brush": "Draw", "samples": screen.clone(), "camera": "front", "viewport": [0, 100] }),
            "/viewport",
        ),
        (
            json!({ "brush": "Draw", "samples": screen, "camera": "front", "viewport": [100, 20000] }),
            "/viewport",
        ),
        (
            json!({ "brush": "Smooth", "samples": world.clone(), "modifier": 30 }),
            "/modifier",
        ),
        (
            json!({ "brush": "Draw", "samples": world.clone(), "view_dir": [0, 0, 0] }),
            "/view_dir",
        ),
        (
            json!({ "brush": "Draw", "samples": world.clone(), "size": -0.1 }),
            "/size",
        ),
        (
            json!({ "brush": "Draw", "samples": world, "size": "0%" }),
            "/size",
        ),
    ] {
        let got = refused("brush.stroke", args.clone());
        assert_eq!(got, ("E_ARGS_INVALID".into(), Some(path.into())), "{args}");
    }
    // The message says which kind of point a stroke needs, and a brush name that matches no
    // brush points to the list.
    let mut s = sphere_session(None);
    let world = json!({ "brush": "Draw", "samples": [{ "position": [500, 500] }] });
    let screen =
        json!({ "brush": "Draw", "samples": [{ "position": [0, 0, 1] }], "camera": "front" });
    let typo = json!({ "brush": "Drawing", "samples": [{ "position": [0, 0, 1] }] });
    assert!(
        s.err("brush.stroke", world)
            .message
            .contains("a world point")
    );
    assert!(
        s.err("brush.stroke", screen)
            .message
            .contains("the camera's image")
    );
    let unknown = s.err("brush.stroke", typo);
    assert_eq!(code(unknown.code), "E_NOT_FOUND");
    assert!(
        unknown.hint.is_some_and(|h| h.contains("brush.list")),
        "the hint names brush.list"
    );
}

#[test]
fn live_detail_says_which_setting_is_wrong() {
    // Screen mode needs a brush sized in pixels on screen; sizes are positive, constant mode
    // has no default size, and the rest have their ranges.
    for (live_detail, path) in [
        (json!({ "mode": "screen" }), "/live_detail/mode"),
        (json!({ "mode": "constant" }), "/live_detail/size"),
        (json!({ "size": 0 }), "/live_detail/size"),
        (json!({ "merge": 1.5 }), "/live_detail/merge"),
        (json!({ "budget": 0 }), "/live_detail/budget"),
    ] {
        let args = json!({ "brush": "Draw", "samples": [{ "position": [0, 0, 1] }],
            "live_detail": live_detail });
        let got = refused("brush.stroke", args.clone());
        assert_eq!(got, ("E_ARGS_INVALID".into(), Some(path.into())), "{args}");
    }
}

#[test]
fn picks_say_what_they_need() {
    let mut s = sphere_session(None);
    for args in [
        json!({ "camera": "front" }),
        json!({ "origin": [0, 0, 5] }),
        json!({ "origin": [0, 0, 5], "direction": [0, 0, 0] }),
        json!({ "camera": "front", "at": [1, 1], "origin": [0, 0, 5], "direction": [0, 0, -1] }),
    ] {
        let err = s.err("pick.ray", args.clone());
        assert_eq!(code(err.code), "E_ARGS_INVALID", "{args}: {}", err.message);
    }
    let elsewhere = [
        (
            "pick.ray",
            json!({ "part": "head", "origin": [0, 0, 5], "direction": [0, 0, -1] }),
        ),
        (
            "pick.nearest",
            json!({ "part": "head", "position": [0, 0, 0] }),
        ),
    ];
    for (command, args) in elsewhere {
        let err = s.err(command, args);
        assert_eq!(code(err.code), "E_NOT_FOUND", "{command}: {}", err.message);
    }
    let infinite = s.err("pick.nearest", json!({ "position": [1e39, 0, 0] }));
    assert_eq!(code(infinite.code), "E_ARGS_INVALID");
}
