//! The real SDL window on SDL's offscreen video driver: nothing appears on screen.

use std::time::Duration;

use gs_platform::{Input, Platform, WindowOptions};
use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod, Scancode};

/// One test, because SDL may only be started from one thread at a time.
#[test]
fn the_offscreen_window_reads_events_sizes_and_the_clock() {
    let options = WindowOptions {
        title: "gs-platform test".into(),
        width: 320,
        height: 200,
        driver: Some("offscreen".into()),
    };
    let mut platform = Platform::open(&options).unwrap_or_else(|e| panic!("open: {e}"));
    assert_eq!(platform.driver(), "offscreen");
    let density = platform.pump(Duration::ZERO).density;
    assert!(density > 0.0, "density {density}");
    let (w, h) = platform.size_in_pixels();
    assert_eq!(
        (w, h),
        ((320.0 * density) as u32, (200.0 * density) as u32),
        "pixels are points times density"
    );

    handles_stay_on_the_window_thread(&platform);
    text_input_toggles(&mut platform);
    queued_events_arrive(&mut platform);
    the_clock_is_sdls_event_clock(&platform);
}

fn handles_stay_on_the_window_thread(platform: &Platform) {
    let handle = platform.surface_handle();
    let here = (
        handle.window_handle().err().map(|e| format!("{e:?}")),
        handle.display_handle().err().map(|e| format!("{e:?}")),
    );
    let unsupported = Some(format!("{:?}", HandleError::NotSupported));
    assert_eq!(
        here,
        (unsupported.clone(), unsupported),
        "on its own thread the window is found, but offscreen has no native handle to give"
    );
    let elsewhere = std::thread::spawn(move || {
        (
            handle.window_handle().err().map(|e| format!("{e:?}")),
            handle.display_handle().err().map(|e| format!("{e:?}")),
        )
    })
    .join()
    .unwrap_or_else(|_| panic!("thread"));
    let unavailable = Some(format!("{:?}", HandleError::Unavailable));
    assert_eq!(
        elsewhere,
        (unavailable.clone(), unavailable),
        "other threads"
    );
}

fn text_input_toggles(platform: &mut Platform) {
    platform.set_text_input(true);
    assert!(platform.text_input_active(), "text input on");
    platform.set_text_input(true);
    assert!(platform.text_input_active(), "still on");
    platform.set_text_input(false);
    assert!(!platform.text_input_active(), "text input off");
}

fn queued_events_arrive(platform: &mut Platform) {
    let sdl = sdl3::init().unwrap_or_else(|e| panic!("{e}"));
    let events = sdl.event().unwrap_or_else(|e| panic!("{e}"));
    let push = |event| events.push_event(event).unwrap_or_else(|e| panic!("{e}"));
    let empty = platform.pump(Duration::from_millis(1));
    let before = empty.received_ns;
    push(Event::KeyDown {
        timestamp: 0,
        window_id: 0,
        keycode: Some(Keycode::Q),
        scancode: Some(Scancode::Q),
        keymod: Mod::NOMOD,
        repeat: false,
        which: 0,
        raw: 0,
    });
    push(Event::Window {
        timestamp: 0,
        window_id: 0,
        win_event: WindowEvent::CloseRequested,
    });
    let pumped = platform.pump(Duration::from_millis(500));
    assert!(
        pumped.received_ns >= before,
        "the receive time moves forward"
    );
    let keys = pumped
        .inputs
        .iter()
        .filter(|i| matches!(i, Input::Key { key, down: true, .. } if key == "Q"))
        .count();
    assert_eq!(keys, 1, "{:?}", pumped.inputs);
    assert!(pumped.inputs.contains(&Input::Close), "{:?}", pumped.inputs);
}

fn the_clock_is_sdls_event_clock(platform: &Platform) {
    // Within 0.1 ms of SDL's own clock (which the safe crate reads in whole milliseconds), at
    // every phase of the millisecond.
    const SLACK: u64 = 100_000;
    for _ in 0..200_000 {
        let earliest = sdl3::timer::ticks() * 1_000_000;
        let now = platform.now_ns();
        let latest = (sdl3::timer::ticks() + 1) * 1_000_000;
        assert!(
            earliest.saturating_sub(SLACK) <= now && now <= latest + SLACK,
            "{earliest} <= {now} < {latest}, give or take {SLACK} ns"
        );
    }
}
