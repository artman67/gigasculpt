//! A save while a gesture is still in progress (decision 0041, section 4): a held gizmo drag or
//! a pen still down must not reach a file, so every command that writes the document out
//! refuses for a moment, and writes the finished gesture once the pen lifts. Beside an open
//! deformer (amended 2026-10-12 for DF4): a held handle refuses the same way, an open deformer
//! with nothing held is accepted first, autosave waits while one is open, and a pen press that
//! starts a stroke accepts it.

use std::path::Path;

use gs_engine::{
    CallReport, PaletteInfo, PenDown, PenReads, PenStroke, PenTaken, Screenshot, UiDescription,
    UiPalette, WindowHost,
};
use gs_protocol::{PenSample, Role};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::autosave::{clocked, tick};
use crate::{at, folder, session};

/// A window that hands the engine `strokes` at the next call, as the studio hands it the pen's.
struct Pen(Vec<PenStroke>);

impl WindowHost for Pen {
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

    fn after_call(&mut self, _report: &CallReport) {}

    fn take_strokes(&mut self) -> Vec<PenStroke> {
        std::mem::take(&mut self.0)
    }
}

/// The pen's touch on a 640 by 400 view, with the Draw brush.
fn press() -> PenStroke {
    PenStroke::Down(PenDown {
        viewport: [0, 0, 640, 400],
        classic: false,
        brush: Some("Draw".into()),
        invert: false,
    })
}

/// The pen's `i`th sample of a short stroke across the middle of the view.
fn sample(i: u32) -> PenStroke {
    let x = 280.0 + 8.0 * f64::from(i);
    let raw = json!({ "x": x, "y": 200.0, "pressure": 0.8, "contact": true });
    PenStroke::Sample(serde_json::from_value(raw).unwrap_or_else(|e| panic!("{e}")))
}

/// A `gizmo.deformer` call bending the sphere `degrees` along its y, toward x.
fn bend(s: &mut Session, phase: &str, degrees: f32) {
    let op = json!({ "bend": { "axis": "y", "toward": "x", "degrees": degrees } });
    let transforms = json!([{ "part": "sphere", "op": op }]);
    s.ok(
        "gizmo.deformer",
        json!({ "phase": phase, "transforms": transforms }),
    );
}

/// Opens a bend on the sphere, drags a handle to `degrees` and lets go.
fn bent(s: &mut Session, degrees: f32) {
    bend(s, "open", 0.0);
    bend(s, "update", degrees);
    s.ok("gizmo.deformer", json!({ "phase": "lift" }));
}

/// The session's state.
fn state(s: &mut Session) -> Value {
    s.query("session.state", json!({}))
}

/// The sphere's points in the scene file at `path`, opened in a fresh session.
fn saved_points(path: &str) -> Vec<[f32; 3]> {
    let mut fresh = Session::with(gs_engine::EngineConfig::default(), Role::Agent);
    fresh.ok("file.open", json!({ "path": path }));
    fresh.mesh("sphere").positions
}

/// The names of the files in `dir`.
fn files(dir: &Path) -> Vec<String> {
    let names = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.file_name()));
    let mut names: Vec<String> = (names.into_iter().flatten())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every command that writes the document out, with arguments that write into `dir`.
fn writers(dir: &Path) -> [(&'static str, Value); 5] {
    let folder = dir.to_string_lossy().into_owned();
    [
        ("file.save", json!({ "path": at(dir, "a.gss") })),
        ("file.save_next", json!({})),
        ("file.quick_save", json!({})),
        ("file.export", json!({ "path": at(dir, "a.obj") })),
        ("exchange.write", json!({ "folder": folder })),
    ]
}

/// Every command that writes the document out, run now, is refused for a moment and writes
/// nothing into `dir`, where the scene's own file is `start.gss`.
fn every_write_waits(s: &mut Session, dir: &Path) {
    let own = std::fs::read(dir.join("start.gss")).ok();
    for (command, args) in writers(dir) {
        let err = s.err(command, args);
        assert_eq!(code(err.code), "E_BUSY", "{command}: {}", err.message);
        assert!(
            err.retryable && err.retry_after_ms.is_some(),
            "{command}: retry"
        );
    }
    assert_eq!(files(dir), ["start.gss"], "nothing new was written");
    let after = std::fs::read(dir.join("start.gss")).ok();
    assert!(
        own.is_some() && after == own,
        "the scene's own file is as it was"
    );
}

#[test]
fn a_save_while_a_drag_is_held_waits_and_keeps_the_drag_out_of_the_file() {
    let dir = folder("mid-drag");
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("file.save", json!({ "path": at(&dir, "start.gss") }));
    let before = s.mesh("sphere").positions;
    let lift = json!({ "phase": "update", "transform": { "op": { "move": [0, 1, 0] } } });
    s.ok("gizmo.drag", lift);
    assert_ne!(
        s.mesh("sphere").positions,
        before,
        "the drag moves the sphere live"
    );
    every_write_waits(&mut s, &dir);
    s.ok("gizmo.drag", json!({ "phase": "cancel" }));
    let path = at(&dir, "held.gss");
    s.ok("file.save", json!({ "path": path }));
    assert_eq!(saved_points(&path), before, "the file holds no drag");
}

#[test]
fn a_save_while_the_pen_is_down_waits_and_writes_the_whole_stroke_after_the_lift() {
    let dir = folder("mid-stroke");
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    s.ok("file.save", json!({ "path": at(&dir, "start.gss") }));
    let before = s.mesh("sphere").positions;
    let mut strokes = vec![press()];
    strokes.extend((0..6).map(sample));
    s.engine.attach_window(Box::new(Pen(strokes)));
    s.query("brush.set", json!({}));
    assert_ne!(s.mesh("sphere").positions, before, "the pen sculpts live");
    every_write_waits(&mut s, &dir);
    s.engine.attach_window(Box::new(Pen(vec![PenStroke::Up])));
    s.query("brush.set", json!({}));
    let path = at(&dir, "lifted.gss");
    s.ok("file.save", json!({ "path": path }));
    let stroked = s.mesh("sphere").positions;
    assert_ne!(stroked, before, "the stroke is kept");
    assert_eq!(
        saved_points(&path),
        stroked,
        "the file holds the whole stroke"
    );
}

#[test]
fn a_save_beside_an_open_deformer_waits_while_a_handle_is_held_and_accepts_it_first_when_not() {
    let dir = folder("mid-deformer");
    // Autosave on, so `file.quick_save` has a folder to write to.
    let mut s = session(0, None, Some(&folder("mid-deformer-state")));
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("file.save", json!({ "path": at(&dir, "start.gss") }));
    let before = s.mesh("sphere").positions;
    bend(&mut s, "open", 0.0);
    bend(&mut s, "update", 60.0);
    assert_ne!(
        s.mesh("sphere").positions,
        before,
        "the deformer bends live"
    );
    every_write_waits(&mut s, &dir);
    s.ok("gizmo.deformer", json!({ "phase": "cancel" }));
    assert_eq!(s.mesh("sphere").positions, before);

    for (command, args) in writers(&dir) {
        bent(&mut s, 30.0);
        let shown = s.mesh("sphere").positions;
        s.ok(command, args);
        let now = state(&mut s);
        assert_eq!(now["deformer"], Value::Null, "{command} accepts it first");
        assert_eq!(now["history.undo"], json!("Bend"), "{command}");
        assert_eq!(
            s.mesh("sphere").positions,
            shown,
            "{command}: as it was shown"
        );
        if command == "file.save" {
            let saved = saved_points(&at(&dir, "a.gss"));
            assert_eq!(saved, shown, "the file holds the accepted bend");
        }
    }
}

#[test]
fn autosave_waits_while_a_deformer_is_open() {
    let state_dir = folder("deformer-autosave");
    let (mut s, clock) = clocked(&state_dir);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    bent(&mut s, 40.0);
    let shown = s.mesh("sphere").positions;
    clock.advance(std::time::Duration::from_mins(11));
    assert_eq!(tick(&mut s), None, "no autosave while the deformer is open");
    assert_eq!(
        state(&mut s)["deformer"],
        json!("bend"),
        "and it stays open"
    );
    s.ok("gizmo.deformer", json!({ "phase": "accept" }));
    clock.advance(std::time::Duration::from_mins(3));
    let copy = tick(&mut s).expect("the autosave once it is accepted");
    assert_eq!(
        saved_points(&copy),
        shown,
        "the copy holds the accepted bend"
    );
}

#[test]
fn a_pen_press_that_starts_a_stroke_accepts_an_open_deformer_first() {
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    bent(&mut s, 40.0);
    let shown = s.mesh("sphere").positions;
    let mut strokes = vec![press()];
    strokes.extend((0..6).map(sample));
    s.engine.attach_window(Box::new(Pen(strokes)));
    s.query("brush.set", json!({}));
    let now = state(&mut s);
    assert_eq!(now["deformer"], Value::Null, "the press accepts it");
    assert_eq!(
        now["history.undo"],
        json!("Bend"),
        "before the stroke begins"
    );
    s.engine.attach_window(Box::new(Pen(vec![PenStroke::Up])));
    s.query("brush.set", json!({}));
    assert_eq!(state(&mut s)["history.undo"], json!("Draw stroke"));
    s.ok("history.undo", json!({}));
    assert_eq!(
        s.mesh("sphere").positions,
        shown,
        "the stroke went on the bend"
    );
}
