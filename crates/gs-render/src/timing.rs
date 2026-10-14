//! GPU timestamps around each pass of a frame, for measuring what each pass costs (decision
//! 0019). Needs `TIMESTAMP_QUERY`; without it there is nothing to time.

use std::sync::{Mutex, PoisonError};

use crate::device::Gpu;

/// At most this many passes are timed in one frame.
const MAX_PASSES: u32 = 32;

/// What a frame records besides its picture: triangle counts and pass times.
#[derive(Clone, Copy, Default)]
pub(crate) struct Probe<'a> {
    pub stats: Option<&'a wgpu::QuerySet>,
    pub timing: Option<&'a Timing>,
}

/// A query set with a begin and an end timestamp per pass, and the passes' names in order.
pub(crate) struct Timing {
    set: wgpu::QuerySet,
    names: Mutex<Vec<&'static str>>,
}

impl Timing {
    /// `None` when the device cannot time passes.
    pub fn new(gpu: &Gpu) -> Option<Timing> {
        gpu.device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Timing {
                set: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("pass times"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2 * MAX_PASSES,
                }),
                names: Mutex::new(Vec::new()),
            })
    }

    /// The query indices for the next pass, named `name`.
    fn next(&self, name: &'static str) -> Option<u32> {
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        let k = names.len() as u32;
        (k < MAX_PASSES).then(|| {
            names.push(name);
            2 * k
        })
    }

    pub fn render(&self, name: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.next(name).map(|k| wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(k),
            end_of_pass_write_index: Some(k + 1),
        })
    }

    /// The same two query slots as [`Timing::render`], for a compute pass.
    pub fn compute(&self, name: &'static str) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let w = self.render(name)?;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: w.query_set,
            beginning_of_pass_write_index: w.beginning_of_pass_write_index,
            end_of_pass_write_index: w.end_of_pass_write_index,
        })
    }

    /// Resolves every recorded timestamp into `buffer` (`QUERY_RESOLVE`, 8 bytes each).
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer) -> u32 {
        let count = 2 * self
            .names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len() as u32;
        if count > 0 {
            encoder.resolve_query_set(&self.set, 0..count, buffer, 0);
        }
        count
    }

    /// Milliseconds per pass from resolved ticks, passes of the same name summed, in order.
    pub fn times(&self, ticks: &[u64], period_ns: f32) -> Vec<(&'static str, f64)> {
        let names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<(&'static str, f64)> = Vec::new();
        for (k, name) in names.iter().enumerate() {
            let (a, b) = (ticks[2 * k], ticks[2 * k + 1]);
            let ms = b.saturating_sub(a) as f64 * f64::from(period_ns) / 1e6;
            match out.iter_mut().find(|(n, _)| n == name) {
                Some((_, t)) => *t += ms,
                None => out.push((name, ms)),
            }
        }
        out
    }
}
