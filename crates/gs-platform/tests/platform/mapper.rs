//! Every non-pen SDL event the window uses, mapped to `Input`, and the rule for the mouse
//! beside a pen: SDL's mouse events made from the pen are dropped, and the mouse's presses and
//! motion while the pen's tip is down.

use gs_platform::{Button, EventMapper, Input, Modifiers};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::{MouseButton, MouseWheelDirection};

fn map(events: &[Event], density: f32) -> Vec<Input> {
    let mut mapper = EventMapper::default();
    for event in events {
        mapper.feed(event, density);
    }
    mapper.take()
}

fn key(down: bool, keycode: Option<Keycode>, keymod: Mod, repeat: bool) -> Event {
    let (timestamp, window_id, scancode, which, raw) = (0, 1, None, 0, 0);
    if down {
        Event::KeyDown {
            timestamp,
            window_id,
            keycode,
            scancode,
            keymod,
            repeat,
            which,
            raw,
        }
    } else {
        Event::KeyUp {
            timestamp,
            window_id,
            keycode,
            scancode,
            keymod,
            repeat,
            which,
            raw,
        }
    }
}

fn button(down: bool, mouse_btn: MouseButton, x: f32, y: f32) -> Event {
    let (timestamp, window_id, which, clicks) = (0, 1, 1, 1);
    if down {
        Event::MouseButtonDown {
            timestamp,
            window_id,
            which,
            mouse_btn,
            clicks,
            x,
            y,
        }
    } else {
        Event::MouseButtonUp {
            timestamp,
            window_id,
            which,
            mouse_btn,
            clicks,
            x,
            y,
        }
    }
}

fn wheel(y: f32, direction: MouseWheelDirection) -> Event {
    Event::MouseWheel {
        timestamp: 0,
        window_id: 1,
        which: 1,
        x: 0.5,
        y,
        direction,
        mouse_x: 0.0,
        mouse_y: 0.0,
        integer_x: 0,
        integer_y: 0,
    }
}

fn window(win_event: WindowEvent) -> Event {
    Event::Window {
        timestamp: 0,
        window_id: 1,
        win_event,
    }
}

#[test]
fn keys_carry_their_name_repeat_and_modifiers() {
    let ctrl_shift = Mod::LCTRLMOD | Mod::RSHIFTMOD;
    let alt_logo = Mod::RALTMOD | Mod::LGUIMOD;
    let got = map(
        &[
            key(true, Some(Keycode::Space), ctrl_shift, true),
            key(false, Some(Keycode::Z), alt_logo, false),
            key(true, None, Mod::NOMOD, false),
            Event::TextInput {
                timestamp: 0,
                window_id: 1,
                text: "é".into(),
            },
        ],
        1.0,
    );
    let mods = |shift, ctrl, alt, logo| Modifiers {
        shift,
        ctrl,
        alt,
        logo,
    };
    assert_eq!(
        got,
        vec![
            Input::Key {
                key: "Space".into(),
                down: true,
                repeat: true,
                modifiers: mods(true, true, false, false),
            },
            Input::Key {
                key: "Z".into(),
                down: false,
                repeat: false,
                modifiers: mods(false, false, true, true),
            },
            Input::Text("é".into()),
        ],
        "a key without a keycode is dropped"
    );
    let left_alt = map(&[key(true, Some(Keycode::A), Mod::LALTMOD, false)], 1.0);
    let right_ctrl = map(&[key(true, Some(Keycode::A), Mod::RCTRLMOD, false)], 1.0);
    let right_logo = map(&[key(true, Some(Keycode::A), Mod::RGUIMOD, false)], 1.0);
    let left_shift = map(&[key(true, Some(Keycode::A), Mod::LSHIFTMOD, false)], 1.0);
    let modifiers = |inputs: &[Input]| match inputs {
        [Input::Key { modifiers, .. }] => *modifiers,
        other => panic!("one key expected: {other:?}"),
    };
    assert_eq!(modifiers(&left_alt), mods(false, false, true, false));
    assert_eq!(modifiers(&right_ctrl), mods(false, true, false, false));
    assert_eq!(modifiers(&right_logo), mods(false, false, false, true));
    assert_eq!(modifiers(&left_shift), mods(true, false, false, false));
}

#[test]
fn mouse_buttons_and_wheel_in_physical_pixels() {
    let got = map(
        &[
            button(true, MouseButton::Left, 10.0, 20.0),
            button(false, MouseButton::Middle, 10.0, 20.0),
            button(true, MouseButton::Right, 4.0, 8.0),
            button(true, MouseButton::X1, 4.0, 8.0),
            wheel(1.0, MouseWheelDirection::Normal),
            wheel(1.0, MouseWheelDirection::Flipped),
        ],
        2.0,
    );
    let press = |button, down, x, y| Input::MouseButton { button, down, x, y };
    assert_eq!(
        got,
        vec![
            press(Button::Left, true, 20.0, 40.0),
            press(Button::Middle, false, 20.0, 40.0),
            press(Button::Right, true, 8.0, 16.0),
            Input::Wheel { dx: 0.5, dy: 1.0 },
            Input::Wheel { dx: -0.5, dy: -1.0 },
        ],
        "side buttons are dropped; a flipped wheel is turned back"
    );
}

/// A click on the close button asks to close (the window may ask about unsaved changes first);
/// SDL's quit, from the system (a signal, the session ending), quits at once, but on macOS,
/// where Cmd+Q is how a person quits, it asks too. A file dropped on the window is its path.
#[test]
fn window_events_that_matter_and_no_others() {
    let dropped = Event::DropFile {
        timestamp: 0,
        window_id: 1,
        filename: "/a folder/head.gss".into(),
    };
    let got = map(
        &[
            window(WindowEvent::Resized(10, 10)),
            window(WindowEvent::PixelSizeChanged(10, 10)),
            window(WindowEvent::DisplayChanged(1)),
            window(WindowEvent::MouseLeave),
            window(WindowEvent::Exposed),
            window(WindowEvent::CloseRequested),
            Event::Quit { timestamp: 0 },
            dropped,
        ],
        1.0,
    );
    let quit = if cfg!(target_os = "macos") {
        Input::Close
    } else {
        Input::Quit
    };
    assert_eq!(
        got,
        vec![
            Input::Resized,
            Input::Resized,
            Input::Resized,
            Input::PointerGone,
            Input::Close,
            quit,
            Input::Dropped("/a folder/head.gss".into()),
        ]
    );
}

/// A mouse event from mouse `which` (SDL's pen mouse is `u32::MAX - 1`).
fn from(which: u32, event: Event) -> Event {
    match event {
        Event::MouseButtonDown {
            timestamp,
            window_id,
            mouse_btn,
            clicks,
            x,
            y,
            ..
        } => Event::MouseButtonDown {
            timestamp,
            window_id,
            which,
            mouse_btn,
            clicks,
            x,
            y,
        },
        other => other,
    }
}

fn motion(which: u32, x: f32, y: f32) -> Event {
    Event::MouseMotion {
        timestamp: 0,
        window_id: 1,
        which,
        mousestate: sdl3::mouse::MouseState::from_sdl_state(0),
        x,
        y,
        xrel: 0.0,
        yrel: 0.0,
    }
}

/// Pen 3 coming into range (`None`), or its tip going down or up.
fn pen(down: Option<bool>) -> Event {
    let (timestamp, which, window, x, y, eraser) = (6, 3, 1, 1.0, 1.0, false);
    match down {
        None => Event::PenProximityIn {
            timestamp,
            which,
            window,
        },
        Some(true) => Event::PenDown {
            timestamp,
            which,
            window,
            x,
            y,
            eraser,
        },
        Some(false) => Event::PenUp {
            timestamp,
            which,
            window,
            x,
            y,
            eraser,
        },
    }
}

/// A mouse button as `Input`.
fn pressed(button: Button, down: bool, at: f64) -> Input {
    Input::MouseButton {
        button,
        down,
        x: at,
        y: at,
    }
}

#[test]
fn the_mouse_works_beside_a_pen_except_while_its_tip_is_down() {
    // Rewritten from the old one-pointer rule, which dropped every mouse event while a pen was
    // in range, so the wheel stopped zooming whenever the pen lay on the tablet (decision 0017,
    // amended 2026-10-11).
    let pen_mouse = u32::MAX - 1;
    let mut mapper = EventMapper::default();
    let mut feed = |events: &[Event]| {
        for event in events {
            mapper.feed(event, 1.0);
        }
        let got = mapper.take();
        got.into_iter()
            .filter(|i| !matches!(i, Input::Pen(_)))
            .collect::<Vec<_>>()
    };
    let near = feed(&[
        pen(None),
        motion(1, 2.0, 2.0),
        button(true, MouseButton::Right, 2.0, 2.0),
        button(false, MouseButton::Right, 2.0, 2.0),
        wheel(1.0, MouseWheelDirection::Normal),
        motion(pen_mouse, 4.0, 4.0),
        from(pen_mouse, button(true, MouseButton::Left, 4.0, 4.0)),
    ]);
    assert_eq!(
        near,
        vec![
            Input::MouseMove { x: 2.0, y: 2.0 },
            pressed(Button::Right, true, 2.0),
            pressed(Button::Right, false, 2.0),
            Input::Wheel { dx: 0.5, dy: 1.0 },
        ],
        "with the pen in range the mouse works; what SDL made from the pen is dropped"
    );
    let touching = feed(&[
        pen(Some(true)),
        motion(1, 5.0, 5.0),
        button(true, MouseButton::Left, 5.0, 5.0),
        wheel(1.0, MouseWheelDirection::Normal),
        button(false, MouseButton::Middle, 5.0, 5.0),
        key(true, Some(Keycode::B), Mod::NOMOD, false),
    ]);
    assert_eq!(
        touching,
        vec![
            Input::Wheel { dx: 0.5, dy: 1.0 },
            pressed(Button::Middle, false, 5.0),
            Input::Key {
                key: "B".into(),
                down: true,
                repeat: false,
                modifiers: Modifiers::default(),
            },
        ],
        "while the tip is down: no presses or motion; the wheel, releases and keys pass"
    );
    let lifted = feed(&[pen(Some(false)), motion(1, 6.0, 6.0)]);
    assert_eq!(
        lifted,
        vec![Input::MouseMove { x: 6.0, y: 6.0 }],
        "the tip lifted"
    );
}

#[test]
fn a_pen_is_near_from_its_first_event_until_it_leaves_range() {
    let mut mapper = EventMapper::default();
    assert!(!mapper.pen_near(), "no pen yet");
    let event = |change: bool| {
        if change {
            Event::PenProximityIn {
                timestamp: 1,
                which: 4,
                window: 1,
            }
        } else {
            Event::PenProximityOut {
                timestamp: 2,
                which: 4,
                window: 1,
            }
        }
    };
    mapper.feed(&event(true), 1.0);
    assert!(mapper.pen_near(), "in range");
    mapper.feed(&event(false), 1.0);
    assert!(!mapper.pen_near(), "left range");
    assert_eq!(mapper.take(), vec![], "never placed, so no sample");
}
