//! The interface clock: nanoseconds on SDL's event clock, so receive, submit and present times
//! compare directly with the time SDL stamps on each pen event.
//!
//! It only measures latency and paces frames. Nothing it reads reaches the document, so it does
//! not touch the determinism rule (ARCHITECTURE section 8). The safe `sdl3` crate does not
//! expose SDL's nanosecond clock, so the clock reads SDL's performance counter and learns once
//! where SDL's time zero is on it: an event pushed with no timestamp gets SDL's current time
//! stamped on it, between two counter reads.

use sdl3::event::{Event, WindowEvent};
use sdl3::{EventPump, EventSubsystem, timer};

/// Nanoseconds since SDL started, matching SDL event timestamps.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    /// Performance counter, in nanoseconds, at SDL time zero.
    zero_ns: u64,
}

fn counter_ns() -> u64 {
    let ticks = u128::from(timer::performance_counter());
    let hz = u128::from(timer::performance_frequency().max(1));
    (ticks.saturating_mul(1_000_000_000) / hz) as u64
}

impl Clock {
    /// Pushes one event for window `window_id` and reads SDL's stamp back. Every event drained
    /// on the way goes to `keep`, so none is lost. Off by at most half the time one push takes.
    pub(crate) fn calibrate(
        events: &EventSubsystem,
        pump: &mut EventPump,
        window_id: u32,
        mut keep: impl FnMut(&Event),
    ) -> Result<Self, String> {
        let marker = Event::Window {
            timestamp: 0,
            window_id,
            win_event: WindowEvent::Exposed,
        };
        let before = counter_ns();
        events.push_event(marker).map_err(|e| e.to_string())?;
        let after = counter_ns();
        let mut stamped = None;
        while let Some(event) = pump.poll_event() {
            if let Event::Window {
                win_event: WindowEvent::Exposed,
                timestamp,
                ..
            } = event
            {
                stamped = Some(timestamp);
            }
            keep(&event);
        }
        let stamped = stamped.ok_or("SDL lost the clock's marker event")?;
        Ok(Self {
            zero_ns: u64::midpoint(before, after).saturating_sub(stamped),
        })
    }

    /// Now, in nanoseconds on SDL's event clock.
    pub fn now_ns(&self) -> u64 {
        counter_ns().saturating_sub(self.zero_ns)
    }
}
