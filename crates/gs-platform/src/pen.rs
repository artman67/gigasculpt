//! Pen events to `PenSample`s, by the rules measured in `docs/spikes/pen.md`:
//!
//! 1. SDL splits one compositor frame into several events with the same time (motion, then
//!    touch, then one per changed axis, then buttons). They are merged into one sample, so a
//!    touch never carries the previous frame's pressure.
//! 2. SDL gives every hover session a new pen id, and libinput can fake a proximity-out after
//!    50 ms of silence, so a pen's state is dropped when it leaves proximity and nothing is
//!    kept per id beyond one session.
//! 3. A pen whose position is not known yet (proximity-in comes without one) makes no sample.

use std::collections::BTreeMap;

use gs_protocol::{PenSample, PenTool};

/// One SDL pen event, reduced to what a sample needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenEvent {
    /// SDL's pen id, valid for one hover session.
    pub pen: u32,
    /// SDL's event time in nanoseconds. All events of one compositor frame share it.
    pub t_ns: u64,
    /// Position in window points, for the events that carry one.
    pub at: Option<[f32; 2]>,
    /// What changed.
    pub change: PenChange,
}

/// What one pen event changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenChange {
    /// The pen came into range.
    ProximityIn,
    /// The pen left range.
    ProximityOut,
    /// Only the position (`at`).
    Motion,
    /// The tip touched (`down`) or lifted; `eraser` says which end.
    Touch { down: bool, eraser: bool },
    /// A barrel button, numbered from 1.
    Button { button: u8, down: bool },
    /// Tip pressure, 0 to 1.
    Pressure(f32),
    /// Tilt to the right, degrees.
    TiltX(f32),
    /// Tilt toward the user, degrees.
    TiltY(f32),
}

/// One pen's state during a hover session.
struct Track {
    sample: PenSample,
    /// False until an event has said where the pen is.
    placed: bool,
}

/// Merges pen events into samples: one per pen per compositor frame.
#[derive(Default)]
pub struct PenTracker {
    pens: BTreeMap<u32, Track>,
    /// The frame being merged: pen id and time.
    open: Option<(u32, u64)>,
    ready: Vec<PenSample>,
}

impl PenTracker {
    /// Applies one event. `density` turns window points into physical pixels.
    pub fn push(&mut self, event: PenEvent, density: f64) {
        if self
            .open
            .is_some_and(|open| open != (event.pen, event.t_ns))
        {
            self.close_frame();
        }
        let track = self.pens.entry(event.pen).or_insert(Track {
            sample: PenSample::hover(event.t_ns, 0.0, 0.0),
            placed: false,
        });
        let s = &mut track.sample;
        s.t_ns = event.t_ns;
        if let Some([x, y]) = event.at {
            s.x = f64::from(x) * density;
            s.y = f64::from(y) * density;
            track.placed = true;
        }
        apply(s, event.change);
        self.open = Some((event.pen, event.t_ns));
        if event.change == PenChange::ProximityOut {
            self.close_frame();
        }
    }

    /// Emits the frame being merged, and forgets its pen if the pen left range.
    fn close_frame(&mut self) {
        let Some((pen, _)) = self.open.take() else {
            return;
        };
        let Some(track) = self.pens.get(&pen) else {
            return;
        };
        if track.placed {
            self.ready.push(track.sample);
        }
        if !track.sample.in_proximity {
            self.pens.remove(&pen);
        }
    }

    /// Ends the frame being merged (the event queue is empty) and returns every sample made
    /// since the last call, oldest first.
    pub fn drain(&mut self) -> Vec<PenSample> {
        self.close_frame();
        std::mem::take(&mut self.ready)
    }

    /// True while any pen is in range.
    pub fn near(&self) -> bool {
        !self.pens.is_empty()
    }

    /// True while any pen's tip is down.
    pub fn touching(&self) -> bool {
        self.pens.values().any(|t| t.sample.contact)
    }
}

fn apply(s: &mut PenSample, change: PenChange) {
    let tool = |eraser| {
        if eraser {
            PenTool::Eraser
        } else {
            PenTool::Pen
        }
    };
    match change {
        PenChange::ProximityIn => s.in_proximity = true,
        PenChange::ProximityOut => {
            s.in_proximity = false;
            s.contact = false;
            s.pressure = 0.0;
            s.buttons = 0;
        }
        PenChange::Motion => {}
        PenChange::Touch { down, eraser } => {
            s.contact = down;
            s.tool = tool(eraser);
        }
        PenChange::Button { button, down } => {
            let bit = 1u8 << (button.saturating_sub(1) & 7);
            if down {
                s.buttons |= bit;
            } else {
                s.buttons &= !bit;
            }
        }
        PenChange::Pressure(p) => s.pressure = p,
        PenChange::TiltX(t) => s.tilt_x = t,
        PenChange::TiltY(t) => s.tilt_y = t,
    }
}
