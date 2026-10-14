//! The palette's session state (decision 0039, section 11, amended for P6): `color.set`'s
//! `alternate`, which decides whether Alt paints Secondary or is ignored for color, and its
//! `swap`, which swaps Main and Secondary; `session.state` names all four values.

use gs_engine::EngineConfig;
use gs_io::mesh::column;
use gs_protocol::{AttrDomain, PlainMesh, Role};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke, stroke_lines};

const RED: &str = "#ff0000";
const DARK: &str = "#102030";

/// The part's colors, white where it has none.
fn colors(m: &PlainMesh) -> Vec<u32> {
    let white = vec![u32::MAX; m.positions.len()];
    column(m, "color", AttrDomain::Vertex).map_or(white, <[u32]>::to_vec)
}

/// The sphere's colors after `palette` is set and one Paint stroke, with Alt held when `alt`,
/// crosses its front with the same seed every time.
fn painted(palette: &Value, alt: bool) -> Vec<u32> {
    let mut s = sphere_session(None);
    s.ok("color.set", palette.clone());
    let path = line([-0.3, 0.0, 0.96], [0.3, 0.1, 0.95], 10);
    let extra = json!({ "size": 0.3, "invert": alt, "seed": 7 });
    s.ok("brush.stroke", stroke("Paint", &path, &extra));
    colors(&s.mesh("sphere"))
}

/// With Alternate on, Alt paints Secondary: the same as a stroke without Alt with the two
/// swapped. With it off, Alt is ignored for color: the stroke paints Main, as without Alt.
#[test]
fn alt_paints_secondary_with_alternate_on_and_main_with_it_off() {
    let palette = json!({ "main": RED, "secondary": DARK });
    let main = painted(&palette, false);
    assert!(main.contains(&0xff00_00ff), "the stroke paints Main");
    let swapped = painted(&json!({ "main": DARK, "secondary": RED }), false);
    assert_eq!(painted(&palette, true), swapped, "on: Alt paints Secondary");
    let off = json!({ "main": RED, "secondary": DARK, "alternate": false });
    assert_eq!(
        painted(&off, true),
        main,
        "off: Alt changes nothing for color"
    );
    assert_eq!(painted(&off, false), main, "off: no Alt, Main as ever");
}

/// A stroke made with Alternate off logs it beside the colors, so the log replays alone in a
/// session with Alternate on; with it on, the logged colors are as they were before Alternate.
#[test]
fn the_log_keeps_alternate_off_and_replays_alone() {
    let log = log_path("palette");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let path = line([-0.3, 0.0, 0.96], [0.3, 0.1, 0.95], 10);
    let alt = json!({ "size": 0.3, "invert": true });
    s.ok("color.set", json!({ "main": RED, "secondary": DARK }));
    s.ok("brush.stroke", stroke("Paint", &path, &alt));
    s.ok("color.set", json!({ "alternate": false }));
    s.ok("brush.stroke", stroke("Paint", &path, &alt));
    let lines = stroke_lines(&log);
    std::fs::remove_file(&log).ok();
    assert_eq!(lines.len(), 2);
    let on = json!({ "main": RED, "secondary": DARK });
    let off = json!({ "main": RED, "secondary": DARK, "alternate": false });
    assert_eq!(lines[0].args["colors"], on, "on: the colors alone");
    assert_eq!(lines[1].args["colors"], off, "off: logged");
    let mut replay = sphere_session(None);
    for line in &lines {
        let request = line.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), "the logged stroke");
        assert_eq!(Some(replay.hash()), line.hash, "replays exactly");
    }
}

/// `swap` swaps Main and Secondary exactly and first, so a color given beside it lands on the
/// swapped pair; Gradient and Alternate stay as they were, and a refused call changes nothing.
#[test]
fn swap_swaps_main_and_secondary_exactly() {
    let mut s = Session::new();
    let set = json!({ "main": "#ff8000", "secondary": DARK, "gradient": true, "alternate": false });
    let start = s.query("color.set", set);
    let want =
        json!({ "main": "#ff8000", "secondary": DARK, "gradient": true, "alternate": false });
    assert_eq!(start, want, "Alternate off is reported");
    let swapped =
        json!({ "main": DARK, "secondary": "#ff8000", "gradient": true, "alternate": false });
    assert_eq!(s.query("color.set", json!({ "swap": true })), swapped);
    assert_eq!(
        s.query("color.set", json!({ "swap": true })),
        want,
        "twice: as before"
    );
    let both = s.query(
        "color.set",
        json!({ "swap": true, "main": RED, "alternate": true }),
    );
    let after = json!({ "main": RED, "secondary": "#ff8000", "gradient": true });
    assert_eq!(
        both, after,
        "swapped first, then Main set; Alternate on is left out"
    );
    let err = s.err("color.set", json!({ "swap": true, "secondary": "red" }));
    assert_eq!(err.path.as_deref(), Some("/secondary"), "{}", err.message);
    assert_eq!(
        s.query("color.set", json!({})),
        after,
        "refused: nothing swapped"
    );
}

/// `session.state` names the palette: `color.main`, `.secondary`, `.gradient` and `.alternate`.
#[test]
fn session_state_names_the_palette() {
    let mut s = Session::new();
    let names = [
        "color.main",
        "color.secondary",
        "color.gradient",
        "color.alternate",
    ];
    let read = |s: &mut Session| {
        let state = s.query("session.state", json!({}));
        names.map(|name| state[name].clone())
    };
    assert_eq!(
        read(&mut s),
        [
            json!("#ffffff"),
            json!("#000000"),
            json!(false),
            json!(true)
        ]
    );
    s.ok(
        "color.set",
        json!({ "main": RED, "gradient": true, "alternate": false }),
    );
    s.ok("color.set", json!({ "swap": true }));
    let want = [json!("#000000"), json!(RED), json!(true), json!(false)];
    assert_eq!(read(&mut s), want);
}
