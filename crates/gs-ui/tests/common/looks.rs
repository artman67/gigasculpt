//! Time passing in a test window as it passes in the real one: the window's loop looks for
//! something to draw every few milliseconds while nothing moves, and draws only when the
//! interface asked to draw again by then.

/// How often the window looks, in seconds (gs-ui `app.rs`, `ANIMATING`).
pub const LOOK: f64 = 0.004;

/// `secs` cut into the window's looks, in nanoseconds each: at least one.
pub fn looks(secs: f64) -> Vec<u64> {
    let n = (secs / LOOK).round().max(1.0) as u64;
    vec![(secs * 1e9) as u64 / n; n as usize]
}
