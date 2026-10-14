//! Frame timing and pen latency statistics: the recent 240 frames, their median, 95th
//! percentile and maximum, and each pen hop from consecutive timestamps.

use gs_ui::{PenTimes, Perf};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn frame_times_cover_the_recent_frames() {
    let mut perf = Perf::default();
    assert_eq!(perf.last_frame_ms(), None, "no frame yet");
    assert_eq!(perf.info().frame.count, 0);
    for ms in 1..=300u64 {
        perf.frame(ms * 1_000_000);
    }
    let info = perf.info();
    assert_eq!(info.frames, 300, "every frame is counted");
    let t = info.frame;
    assert_eq!(t.count, 240, "the recent 240 are kept: 61 to 300 ms");
    assert!(
        perf.last_frame_ms().is_some_and(|ms| close(ms, 300.0)),
        "{:?}",
        perf.last_frame_ms()
    );
    // Sorted 61..=300; median at index round(239 * 0.5) = 120, 95th at round(239 * 0.95) = 227.
    assert!(close(t.p50_ms, 181.0), "median {}", t.p50_ms);
    assert!(close(t.p95_ms, 288.0), "95th {}", t.p95_ms);
    assert!(close(t.max_ms, 300.0), "max {}", t.max_ms);
}

#[test]
fn pen_hops_are_the_gaps_between_timestamps() {
    let mut perf = Perf::default();
    let times = PenTimes {
        sampled: 1_000_000,
        received: 1_500_000,
        applied: 1_600_000,
        submitted: 2_100_000,
        presented: 2_400_000,
    };
    perf.pen(times);
    perf.pen(PenTimes {
        presented: 3_000_000,
        ..times
    });
    let hops = perf.info().hops;
    let names: Vec<&str> = hops.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "compositor to window",
            "window to engine applied",
            "engine to GPU submit",
            "submit to present call",
            "pen sample to present call"
        ]
    );
    let expect = [(0.5, 0.5), (0.1, 0.1), (0.5, 0.5), (0.3, 0.9), (1.4, 2.0)];
    for (hop, (low, high)) in hops.iter().zip(expect) {
        let t = &hop.timing;
        assert_eq!(t.count, 2, "{}", hop.name);
        assert!(close(t.max_ms, high), "{}: max {}", hop.name, t.max_ms);
        assert!(
            close(t.p50_ms, high),
            "{}: median of two is the higher",
            hop.name
        );
        assert!(t.p50_ms >= low, "{}: above the lower value {low}", hop.name);
    }
}
