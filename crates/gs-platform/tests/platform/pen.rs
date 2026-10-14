//! The pen rules as properties over generated SDL event streams (ARCHITECTURE 10.1 kinds 1 and
//! 3): several pens, each hover session a run of compositor frames, frames from different pens
//! interleaved, mouse events in between. The `EventMapper` must agree with a slow reference
//! that folds whole frames: one sample per pen per frame, a pen's state dropped when it leaves
//! range, and no mouse motion while any pen's tip is down.

use std::collections::BTreeMap;

use gs_platform::{EventMapper, Input};
use gs_protocol::{PenSample, PenTool};
use proptest::prelude::*;
use sdl3::event::Event;
use sdl3::pen::PenAxis;

/// One generated SDL pen event, before it gets its pen id, time and position.
#[derive(Clone, Debug)]
enum Ev {
    In,
    Out,
    Motion(f32, f32),
    Touch {
        down: bool,
        eraser: bool,
    },
    Button {
        button: u8,
        down: bool,
    },
    Pressure(f32),
    TiltX(f32),
    TiltY(f32),
    /// An axis no sample carries.
    Distance(f32),
}

/// A compositor frame of one pen: its events, all with one time.
#[derive(Clone, Debug)]
struct Frame {
    pen: u32,
    t_ns: u64,
    events: Vec<Ev>,
}

#[derive(Clone, Debug)]
enum Item {
    Frame(Frame),
    Mouse(f32, f32),
}

fn middle_event() -> impl Strategy<Value = Ev> {
    prop_oneof![
        (0.0f32..3000.0, 0.0f32..2000.0).prop_map(|(x, y)| Ev::Motion(x, y)),
        (any::<bool>(), any::<bool>()).prop_map(|(down, eraser)| Ev::Touch { down, eraser }),
        (1u8..=2, any::<bool>()).prop_map(|(button, down)| Ev::Button { button, down }),
        (0.0f32..=1.0).prop_map(Ev::Pressure),
        (-90.0f32..=90.0).prop_map(Ev::TiltX),
        (-90.0f32..=90.0).prop_map(Ev::TiltY),
        (0.0f32..=1.0).prop_map(Ev::Distance),
    ]
}

/// One hover session: frames of events, starting with proximity-in (alone, or merged into the
/// first frame) and ending with proximity-out (merged into the last frame, or alone).
fn session() -> impl Strategy<Value = (u32, Vec<Vec<Ev>>)> {
    let frames = prop::collection::vec(prop::collection::vec(middle_event(), 0..5), 1..6);
    (1u32..4, frames, any::<bool>(), any::<bool>()).prop_map(
        |(pen, mut frames, in_alone, out_alone)| {
            if in_alone {
                frames.insert(0, vec![Ev::In]);
            } else {
                frames[0].insert(0, Ev::In);
            }
            if out_alone {
                frames.push(vec![Ev::Out]);
            } else if let Some(last) = frames.last_mut() {
                last.push(Ev::Out);
            }
            (pen, frames)
        },
    )
}

/// Sessions with their frames interleaved in a random order, mouse moves sprinkled between,
/// and times growing by 1 to 3 ms per frame. Sessions of the same pen id never overlap: SDL
/// never reuses an id that is still in range.
fn stream() -> impl Strategy<Value = Vec<Item>> {
    let sessions = prop::collection::vec(session(), 1..5);
    let picks = prop::collection::vec((any::<u8>(), any::<u8>(), 1u64..=3), 64);
    let mice = prop::collection::vec((0.0f32..3000.0, 0.0f32..2000.0), 64);
    (sessions, picks, mice).prop_map(|(sessions, picks, mice)| interleave(sessions, &picks, &mice))
}

fn interleave(
    sessions: Vec<(u32, Vec<Vec<Ev>>)>,
    picks: &[(u8, u8, u64)],
    mice: &[(f32, f32)],
) -> Vec<Item> {
    // Distinct ids for sessions that may overlap: id + 4 * session index.
    let mut queues: Vec<(u32, std::collections::VecDeque<Vec<Ev>>)> = sessions
        .into_iter()
        .enumerate()
        .map(|(i, (pen, frames))| (pen + 4 * i as u32, frames.into()))
        .collect();
    let (mut items, mut t, mut step) = (Vec::new(), 1_000_000u64, 0usize);
    while queues.iter().any(|(_, q)| !q.is_empty()) {
        let (pick, mouse, dt) = picks[step % picks.len()];
        let live: Vec<usize> = (0..queues.len())
            .filter(|&i| !queues[i].1.is_empty())
            .collect();
        let (pen, queue) = &mut queues[live[pick as usize % live.len()]];
        t += dt * 1_000_000;
        let events = queue.pop_front().unwrap_or_default();
        items.push(Item::Frame(Frame {
            pen: *pen,
            t_ns: t,
            events,
        }));
        if mouse % 3 == 0 {
            let (x, y) = mice[step % mice.len()];
            items.push(Item::Mouse(x, y));
        }
        step += 1;
    }
    items
}

/// Each pen's position as SDL reports it on every event: the last motion, starting at a fixed
/// spot per pen.
fn sdl_events(items: &[Item]) -> Vec<Event> {
    let mut at: BTreeMap<u32, (f32, f32)> = BTreeMap::new();
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Mouse(x, y) => out.push(mouse_motion(*x, *y)),
            Item::Frame(frame) => {
                for ev in &frame.events {
                    let pos = at.entry(frame.pen).or_insert((100.0, 50.0));
                    if let Ev::Motion(x, y) = ev {
                        *pos = (*x, *y);
                    }
                    out.push(sdl_pen(frame.pen, frame.t_ns, *pos, ev));
                }
            }
        }
    }
    out
}

fn mouse_motion(x: f32, y: f32) -> Event {
    Event::MouseMotion {
        timestamp: 0,
        window_id: 1,
        which: 1,
        mousestate: sdl3::mouse::MouseState::from_sdl_state(0),
        x,
        y,
        xrel: 0.0,
        yrel: 0.0,
    }
}

fn sdl_pen(which: u32, timestamp: u64, (x, y): (f32, f32), ev: &Ev) -> Event {
    let window = 1;
    let axis = |axis, value| Event::PenAxis {
        timestamp,
        which,
        window,
        x,
        y,
        axis,
        value,
    };
    match *ev {
        Ev::In => Event::PenProximityIn {
            timestamp,
            which,
            window,
        },
        Ev::Out => Event::PenProximityOut {
            timestamp,
            which,
            window,
        },
        Ev::Motion(..) => Event::PenMotion {
            timestamp,
            which,
            window,
            x,
            y,
        },
        Ev::Touch { down: true, eraser } => Event::PenDown {
            timestamp,
            which,
            window,
            x,
            y,
            eraser,
        },
        Ev::Touch {
            down: false,
            eraser,
        } => Event::PenUp {
            timestamp,
            which,
            window,
            x,
            y,
            eraser,
        },
        Ev::Button { button, down: true } => Event::PenButtonDown {
            timestamp,
            which,
            window,
            x,
            y,
            button,
        },
        Ev::Button {
            button,
            down: false,
        } => Event::PenButtonUp {
            timestamp,
            which,
            window,
            x,
            y,
            button,
        },
        Ev::Pressure(v) => axis(PenAxis::Pressure, v),
        Ev::TiltX(v) => axis(PenAxis::XTilt, v),
        Ev::TiltY(v) => axis(PenAxis::YTilt, v),
        Ev::Distance(v) => axis(PenAxis::Distance, v),
    }
}

/// The slow reference: whole frames folded one at a time.
fn reference(items: &[Item], density: f64) -> Vec<Input> {
    let mut pens: BTreeMap<u32, (PenSample, bool)> = BTreeMap::new();
    let mut at: BTreeMap<u32, (f32, f32)> = BTreeMap::new();
    let mut out = Vec::new();
    for item in items {
        let frame = match item {
            Item::Mouse(x, y) => {
                if !pens.values().any(|(s, _)| s.contact) {
                    let (x, y) = (f64::from(*x) * density, f64::from(*y) * density);
                    out.push(Input::MouseMove { x, y });
                }
                continue;
            }
            Item::Frame(frame) => frame,
        };
        let tracked = frame.events.iter().any(|e| !matches!(e, Ev::Distance(_)));
        if !tracked {
            continue;
        }
        let (sample, placed) = pens.entry(frame.pen).or_insert_with(|| {
            let mut fresh = PenSample::hover(0, 0.0, 0.0);
            fresh.in_proximity = true;
            (fresh, false)
        });
        sample.t_ns = frame.t_ns;
        for ev in &frame.events {
            let pos = at.entry(frame.pen).or_insert((100.0, 50.0));
            if let Ev::Motion(x, y) = ev {
                *pos = (*x, *y);
            }
            if !matches!(ev, Ev::In | Ev::Out | Ev::Distance(_)) {
                sample.x = f64::from(pos.0) * density;
                sample.y = f64::from(pos.1) * density;
                *placed = true;
            }
            fold(sample, ev);
        }
        if *placed {
            out.push(Input::Pen(*sample));
        }
        if !sample.in_proximity {
            pens.remove(&frame.pen);
        }
    }
    out
}

fn fold(s: &mut PenSample, ev: &Ev) {
    match *ev {
        Ev::Out => {
            *s = PenSample {
                in_proximity: false,
                contact: false,
                pressure: 0.0,
                buttons: 0,
                ..*s
            };
        }
        Ev::Touch { down, eraser } => {
            s.contact = down;
            s.tool = if eraser {
                PenTool::Eraser
            } else {
                PenTool::Pen
            };
        }
        Ev::Button { button, down } => {
            let bit = 1 << (button - 1);
            s.buttons = if down {
                s.buttons | bit
            } else {
                s.buttons & !bit
            };
        }
        Ev::Pressure(v) => s.pressure = v,
        Ev::TiltX(v) => s.tilt_x = v,
        Ev::TiltY(v) => s.tilt_y = v,
        Ev::In | Ev::Motion(..) | Ev::Distance(_) => {}
    }
}

/// Feeds events the way `Platform::pump` does: everything, then `take` when the queue is empty.
fn mapped(events: &[Event], density: f32) -> Vec<Input> {
    let mut mapper = EventMapper::default();
    for event in events {
        mapper.feed(event, density);
    }
    let out = mapper.take();
    assert!(!mapper.pen_near(), "every session ended, so no pen is near");
    out
}

proptest! {
    #[test]
    fn mapper_merges_frames_drops_state_and_holds_the_mouse_while_a_tip_is_down(
        items in stream(),
        density in prop_oneof![Just(1.0f32), Just(1.25), Just(2.0)],
    ) {
        let got = mapped(&sdl_events(&items), density);
        let want = reference(&items, f64::from(density));
        prop_assert_eq!(got, want);
    }

    /// The same stream read in several pumps (the queue ran dry at random points) gives the
    /// same samples, as long as no pump boundary falls inside a frame.
    #[test]
    fn pump_boundaries_between_frames_change_nothing(items in stream(), cut in any::<u8>()) {
        let events = sdl_events(&items);
        let mut mapper = EventMapper::default();
        let mut got = Vec::new();
        let mut frame_starts = Vec::new();
        let mut i = 0;
        for item in &items {
            frame_starts.push(i);
            i += match item {
                Item::Frame(f) => f.events.len(),
                Item::Mouse(..) => 1,
            };
        }
        let cut_at = frame_starts[cut as usize % frame_starts.len()];
        for (i, event) in events.iter().enumerate() {
            if i == cut_at {
                got.extend(mapper.take());
            }
            mapper.feed(event, 1.0);
        }
        got.extend(mapper.take());
        prop_assert_eq!(got, mapped(&events, 1.0));
    }
}

/// The case the merge rule exists for: SDL sends the touch before the pressure axis of the same
/// frame, so an unmerged touch would carry the previous frame's pressure.
#[test]
fn a_touch_carries_the_pressure_of_its_own_frame() {
    let hover = Item::Frame(Frame {
        pen: 7,
        t_ns: 10,
        events: vec![Ev::In, Ev::Motion(10.0, 20.0), Ev::Pressure(0.0)],
    });
    let touch = Item::Frame(Frame {
        pen: 7,
        t_ns: 14,
        events: vec![
            Ev::Motion(11.0, 21.0),
            Ev::Touch {
                down: true,
                eraser: false,
            },
            Ev::Pressure(0.4),
            Ev::Button {
                button: 1,
                down: true,
            },
        ],
    });
    let out = Item::Frame(Frame {
        pen: 7,
        t_ns: 18,
        events: vec![Ev::Out],
    });
    let got = mapped(&sdl_events(&[hover, touch, out]), 1.25);
    let samples: Vec<PenSample> = got
        .into_iter()
        .filter_map(|i| match i {
            Input::Pen(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(samples.len(), 3, "one sample per frame: {samples:?}");
    let touched = samples[1];
    assert!(touched.contact, "touching");
    assert!(
        (touched.pressure - 0.4).abs() < f32::EPSILON,
        "own pressure"
    );
    assert_eq!(touched.buttons, 1, "barrel button 1 is bit 0");
    assert!(
        (touched.x - 13.75).abs() < 1e-9 && (touched.y - 26.25).abs() < 1e-9,
        "physical pixels at density 1.25: {touched:?}"
    );
    let last = samples[2];
    assert!(
        !last.in_proximity && !last.contact && last.buttons == 0 && last.pressure == 0.0,
        "leaving range clears the pen: {last:?}"
    );
}
