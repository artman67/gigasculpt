//! The curve line's points through the engine (decision 0055): the window's Alt taps, double
//! tap and Space reach one logged `brush.stroke` as marks on its samples, which replays bit for
//! bit; a mark on a brush that takes no points is refused.

use gs_engine::{PenDown, PenStroke};
use gs_protocol::ErrorCode;
use gs_testkit::success;
use serde_json::json;

use crate::feel::{Window, logging};
use crate::{log_path, sphere_session, stroke_lines};

#[test]
fn the_window_s_taps_and_space_are_logged_on_their_samples_and_replay_exactly() {
    let log = log_path("curve-points");
    let mut s = logging(&log);
    let before = s.hash();
    // Across the sphere's middle in a 640 by 400 view, wavering up and down.
    let sample = |i: u32| {
        let (x, y) = (260.0 + 10.0 * f64::from(i), 200.0 + 6.0 * f64::from(i % 3));
        let raw = json!({ "x": x, "y": y, "pressure": 0.8, "contact": true });
        PenStroke::Sample(serde_json::from_value(raw).unwrap_or_else(|e| panic!("{e}")))
    };
    let tap = |point: PenStroke| [PenStroke::Alt(true), point, PenStroke::Alt(false)];
    let down = PenDown {
        viewport: [0, 0, 640, 400],
        classic: false,
        brush: Some("Clip Curve".into()),
        invert: false,
    };
    let mut strokes = vec![PenStroke::Down(down)];
    strokes.extend((0..3).map(sample));
    strokes.extend(tap(PenStroke::Point));
    strokes.extend((3..6).map(sample));
    strokes.extend(tap(PenStroke::Point));
    strokes.extend(tap(PenStroke::Corner));
    strokes.extend((6..8).map(sample));
    strokes.push(PenStroke::Space(true));
    strokes.extend((8..10).map(sample));
    strokes.push(PenStroke::Space(false));
    strokes.extend((10..13).map(sample));
    strokes.push(PenStroke::Up);
    s.engine.attach_window(Box::new(Window(strokes)));
    s.ok("brush.set", json!({}));
    assert_ne!(s.hash(), before, "the clip changed the sphere");
    let lines = stroke_lines(&log);
    assert_eq!(lines.len(), 1, "the pen's stroke is logged");
    let samples = lines[0].args["samples"].as_array().cloned();
    let samples = samples.unwrap_or_default();
    let marks: Vec<(usize, &str)> = (samples.iter().enumerate())
        .filter_map(|(i, s)| s["point"].as_str().map(|p| (i, p)))
        .collect();
    assert_eq!(
        marks,
        [(2, "soft"), (5, "sharp")],
        "each point on its sample"
    );
    let moving: Vec<usize> = (samples.iter().enumerate())
        .filter(|(_, s)| s["moves"] == json!(true))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(moving, [8, 9], "the samples drawn with Space held");
    let mut replay = sphere_session(None);
    let request = lines[0].request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), lines[0].hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_point_or_a_move_on_a_brush_that_takes_none_is_refused() {
    let mut s = sphere_session(None);
    for (field, value) in [("point", json!("soft")), ("moves", json!(true))] {
        let mut second = json!({ "position": [0.1, 0.1, 0.99] });
        second[field] = value;
        let samples = json!([{ "position": [0.0, 0.0, 1.0] }, second]);
        let e = s.err(
            "brush.stroke",
            json!({ "brush": "Draw", "samples": samples }),
        );
        assert_eq!(e.code, ErrorCode::ArgsInvalid, "{field}");
        let want = format!("/samples/1/{field}");
        assert_eq!(e.path.as_deref(), Some(want.as_str()), "{field}");
    }
}
