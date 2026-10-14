//! Gesture strokes off the model (decision 0017 as amended 2026-10-12, items 17 and 18): with
//! classic navigation on, a Clip Curve pressed on empty canvas starts there and cuts the active
//! part, the engine saying nothing of empty canvas, and its log line replays exactly; a Draw
//! stroke pressed there is still no stroke but empty canvas, which the window turns into
//! navigation. In an empty scene a gesture's press is empty canvas too.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gs_engine::{
    CallReport, PaletteInfo, PenDown, PenReads, PenStroke, PenTaken, Screenshot, UiDescription,
    UiPalette, WindowHost,
};
use gs_protocol::PenSample;
use gs_testkit::{Session, success};
use serde_json::json;

use crate::feel::logging;
use crate::{log_path, sphere_session, stroke_lines};

/// A window that hands the engine `strokes` at the next call and keeps whether a call's report
/// said the press fell on empty canvas.
struct Canvas {
    strokes: Vec<PenStroke>,
    said: Arc<AtomicBool>,
}

impl WindowHost for Canvas {
    fn has_window(&self) -> bool {
        false
    }

    fn pen_input(&mut self, _samples: &[PenSample], _reads: &PenReads) -> PenTaken {
        PenTaken {
            cursor: None,
            gizmo: false,
        }
    }

    fn describe(&mut self) -> UiDescription {
        UiDescription::default()
    }

    fn screenshot(&mut self) -> Option<Result<Screenshot, String>> {
        None
    }

    fn palette(&mut self, _request: &UiPalette) -> Option<Result<PaletteInfo, String>> {
        None
    }

    fn after_call(&mut self, report: &CallReport) {
        self.said.fetch_or(report.on_canvas, Ordering::SeqCst);
    }

    fn take_strokes(&mut self) -> Vec<PenStroke> {
        std::mem::take(&mut self.strokes)
    }
}

/// A stroke with `brush` and classic navigation on in a 640 by 400 view, from empty canvas left
/// of the sphere across it, a little above its middle, to empty canvas on its right, run at
/// `s`'s next call. True when the engine said the press fell on empty canvas.
fn across(s: &mut Session, brush: &str) -> bool {
    let sample = |i: u32| {
        let at = PenSample::hover(4_000_000 * u64::from(i), 100.0 + 40.0 * f64::from(i), 180.0);
        PenStroke::Sample(PenSample {
            pressure: 0.8,
            contact: true,
            ..at
        })
    };
    let down = PenDown {
        viewport: [0, 0, 640, 400],
        classic: true,
        brush: Some(brush.into()),
        invert: false,
    };
    let mut strokes = vec![PenStroke::Down(down)];
    strokes.extend((0..12).map(sample));
    strokes.push(PenStroke::Up);
    let said = Arc::new(AtomicBool::new(false));
    let canvas = Canvas {
        strokes,
        said: Arc::clone(&said),
    };
    s.engine.attach_window(Box::new(canvas));
    s.ok("brush.set", json!({}));
    said.load(Ordering::SeqCst)
}

#[test]
fn a_clip_curve_pressed_off_the_model_cuts_it_where_a_draw_stroke_still_navigates() {
    let log = log_path("off-model");
    let mut s = logging(&log);
    let before = s.hash();
    assert!(
        across(&mut s, "Draw"),
        "a Draw press off the model is empty canvas"
    );
    assert_eq!(s.hash(), before, "and strokes nothing");
    assert!(stroke_lines(&log).is_empty(), "nothing is logged");

    assert!(
        !across(&mut s, "Clip Curve"),
        "a Clip Curve press there is no empty canvas"
    );
    assert_ne!(s.hash(), before, "it cut the sphere");
    let lines = stroke_lines(&log);
    assert_eq!(lines.len(), 1, "the pen's stroke is logged");
    let mut replay = sphere_session(None);
    let request = lines[0].request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), lines[0].hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}

#[test]
fn in_an_empty_scene_a_gesture_s_press_is_empty_canvas() {
    let mut s = Session::new();
    assert!(
        across(&mut s, "Clip Curve"),
        "with no part to cut the press turns the view"
    );
}
