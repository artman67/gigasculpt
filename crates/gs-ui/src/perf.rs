//! Frame timing and pen latency, hop by hop (research report 05, section 4): when the
//! compositor stamped the pen sample, when the window read it, when the engine had applied it,
//! when the frame showing it was submitted to the GPU, and when it was handed to the screen.
//! The real time on the panel needs the compositor's presentation feedback or a camera; these
//! hops are what software can see.

use std::collections::VecDeque;

use gs_engine::{Hop, PerfInfo, Timing};

/// Frames kept for the statistics.
const KEEP: usize = 240;

/// When one pen frame passed each point, in nanoseconds on the platform clock.
#[derive(Clone, Copy, Debug)]
pub struct PenTimes {
    /// The newest sample's time: when the compositor sent it.
    pub sampled: u64,
    /// When the window read it from SDL.
    pub received: u64,
    /// When the engine had applied it (`pen.input` returned).
    pub applied: u64,
    /// When the frame showing it was submitted.
    pub submitted: u64,
    /// When the frame was handed to the screen.
    pub presented: u64,
}

const HOPS: [&str; 5] = [
    "compositor to window",
    "window to engine applied",
    "engine to GPU submit",
    "submit to present call",
    "pen sample to present call",
];

/// Recent frame times and pen hops.
#[derive(Clone, Debug, Default)]
pub struct Perf {
    pub adapter: String,
    pub present_mode: String,
    frames: u64,
    frame_ns: VecDeque<u64>,
    hops: [VecDeque<u64>; 5],
}

fn push(window: &mut VecDeque<u64>, value: u64) {
    if window.len() == KEEP {
        window.pop_front();
    }
    window.push_back(value);
}

/// Count, median, 95th percentile and maximum of `values`, in milliseconds.
pub fn timing(values: &VecDeque<u64>) -> Timing {
    let mut sorted: Vec<u64> = values.iter().copied().collect();
    sorted.sort_unstable();
    let at = |p: f64| {
        let i = ((sorted.len().saturating_sub(1)) as f64 * p).round() as usize;
        sorted.get(i).map_or(0.0, |&ns| ns as f64 / 1e6)
    };
    Timing {
        count: sorted.len() as u64,
        p50_ms: at(0.5),
        p95_ms: at(0.95),
        max_ms: at(1.0),
    }
}

impl Perf {
    /// One frame took `ns` to build and submit.
    pub fn frame(&mut self, ns: u64) {
        self.frames += 1;
        push(&mut self.frame_ns, ns);
    }

    /// One pen frame's times.
    pub fn pen(&mut self, t: PenTimes) {
        let points = [t.sampled, t.received, t.applied, t.submitted, t.presented];
        for (i, pair) in points.windows(2).enumerate() {
            push(&mut self.hops[i], pair[1].saturating_sub(pair[0]));
        }
        push(&mut self.hops[4], t.presented.saturating_sub(t.sampled));
    }

    /// The last frame's build time in milliseconds.
    pub fn last_frame_ms(&self) -> Option<f64> {
        self.frame_ns.back().map(|&ns| ns as f64 / 1e6)
    }

    /// As `ui.describe` reports it.
    pub fn info(&self) -> PerfInfo {
        PerfInfo {
            adapter: self.adapter.clone(),
            present_mode: self.present_mode.clone(),
            frames: self.frames,
            frame: timing(&self.frame_ns),
            hops: HOPS
                .iter()
                .zip(&self.hops)
                .map(|(name, values)| Hop {
                    name: (*name).to_string(),
                    timing: timing(values),
                })
                .collect(),
        }
    }
}
