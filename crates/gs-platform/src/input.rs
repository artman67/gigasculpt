//! SDL events to platform-neutral input. Everything above this crate sees only [`Input`].

use gs_protocol::PenSample;
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Mod;
use sdl3::mouse::{MouseButton, MouseWheelDirection};
use sdl3::pen::PenAxis;

use crate::pen::{PenChange, PenEvent, PenTracker};

/// Keyboard modifiers held down.
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent keys, the same shape egui uses"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// The Super (Windows, Command) key.
    pub logo: bool,
}

impl From<Mod> for Modifiers {
    /// Either the left or the right key counts.
    fn from(m: Mod) -> Self {
        let either = |left, right| m.contains(left) || m.contains(right);
        Self {
            shift: either(Mod::LSHIFTMOD, Mod::RSHIFTMOD),
            ctrl: either(Mod::LCTRLMOD, Mod::RCTRLMOD),
            alt: either(Mod::LALTMOD, Mod::RALTMOD),
            logo: either(Mod::LGUIMOD, Mod::RGUIMOD),
        }
    }
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// One piece of input, in physical window pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// One merged pen sample.
    Pen(PenSample),
    /// The mouse moved.
    MouseMove { x: f64, y: f64 },
    /// A mouse button changed.
    MouseButton {
        button: Button,
        down: bool,
        x: f64,
        y: f64,
    },
    /// The wheel turned, in notches: positive `dy` scrolls up (away from the user).
    Wheel { dx: f32, dy: f32 },
    /// A key changed. `key` is SDL's key name, for example `Space`, `Z` or `Left Ctrl`.
    Key {
        key: String,
        down: bool,
        repeat: bool,
        modifiers: Modifiers,
    },
    /// Typed text, while text input is on.
    Text(String),
    /// The window's size or pixel density changed.
    Resized,
    /// The mouse left the window.
    PointerGone,
    /// The user asked to close the window.
    Close,
}

/// The pen fields of an SDL pen event, or `None` for every other event and for pen axes no
/// sample carries (distance, rotation, slider).
pub fn pen_event(event: &Event) -> Option<PenEvent> {
    let (pen, t_ns, at, change) = match *event {
        Event::PenProximityIn {
            which, timestamp, ..
        } => (which, timestamp, None, PenChange::ProximityIn),
        Event::PenProximityOut {
            which, timestamp, ..
        } => (which, timestamp, None, PenChange::ProximityOut),
        Event::PenMotion {
            which,
            timestamp,
            x,
            y,
            ..
        } => (which, timestamp, Some([x, y]), PenChange::Motion),
        Event::PenDown {
            which,
            timestamp,
            x,
            y,
            eraser,
            ..
        } => (which, timestamp, Some([x, y]), touch(true, eraser)),
        Event::PenUp {
            which,
            timestamp,
            x,
            y,
            eraser,
            ..
        } => (which, timestamp, Some([x, y]), touch(false, eraser)),
        Event::PenButtonDown {
            which,
            timestamp,
            x,
            y,
            button,
            ..
        } => (which, timestamp, Some([x, y]), press(button, true)),
        Event::PenButtonUp {
            which,
            timestamp,
            x,
            y,
            button,
            ..
        } => (which, timestamp, Some([x, y]), press(button, false)),
        Event::PenAxis {
            which,
            timestamp,
            x,
            y,
            axis,
            value,
            ..
        } => (which, timestamp, Some([x, y]), pen_axis(axis, value)?),
        _ => return None,
    };
    Some(PenEvent {
        pen,
        t_ns,
        at,
        change,
    })
}

fn touch(down: bool, eraser: bool) -> PenChange {
    PenChange::Touch { down, eraser }
}

fn press(button: u8, down: bool) -> PenChange {
    PenChange::Button { button, down }
}

fn pen_axis(axis: PenAxis, value: f32) -> Option<PenChange> {
    match axis {
        PenAxis::Pressure => Some(PenChange::Pressure(value)),
        PenAxis::XTilt => Some(PenChange::TiltX(value)),
        PenAxis::YTilt => Some(PenChange::TiltY(value)),
        _ => None,
    }
}

/// SDL's mouse id on the mouse events it makes from a pen (`SDL_PEN_MOUSEID`).
const PEN_MOUSE: u32 = sdl3::sys::pen::SDL_PEN_MOUSEID.0;

/// Turns a stream of SDL events into [`Input`]: merges pen frames, drops the mouse events SDL
/// made from the pen and the mouse's presses and motion while a pen's tip is down (decision
/// 0017 as amended), and scales points to physical pixels.
#[derive(Default)]
pub struct EventMapper {
    pens: PenTracker,
    out: Vec<Input>,
}

impl EventMapper {
    /// Takes one event. `density` is the window's pixel density (pixels per point).
    pub fn feed(&mut self, event: &Event, density: f32) {
        let density = f64::from(density);
        if event.is_pen() {
            // Axes no sample carries are part of the frame too: skip them without ending it.
            if let Some(pen) = pen_event(event) {
                self.pens.push(pen, density);
            }
            return;
        }
        // Samples merged so far come before this event.
        let samples = self.pens.drain();
        self.out.extend(samples.into_iter().map(Input::Pen));
        let touching = self.pens.touching();
        if let Some(input) = other_input(event, density, touching) {
            self.out.push(input);
        }
    }

    /// Everything since the last call, in order. Call when the event queue is empty, which
    /// ends the pen frame being merged.
    pub fn take(&mut self) -> Vec<Input> {
        let samples = self.pens.drain();
        self.out.extend(samples.into_iter().map(Input::Pen));
        std::mem::take(&mut self.out)
    }

    /// True while a pen is in range.
    pub fn pen_near(&self) -> bool {
        self.pens.near()
    }

    /// True while a pen's tip is down: the mouse's presses and motion are dropped then.
    pub fn pen_touching(&self) -> bool {
        self.pens.touching()
    }
}

/// Non-pen events. Mouse events SDL made from the pen are dropped (the pen's own events carry
/// it), and so are the mouse's presses and motion while a pen's tip is `touching`, so one
/// stroke runs at a time. A release still passes, so no button stays held; the wheel always
/// passes.
fn other_input(event: &Event, density: f64, touching: bool) -> Option<Input> {
    let px = |v: f32| f64::from(v) * density;
    let mouse = |which: u32| which != PEN_MOUSE;
    let input = match event {
        Event::MouseMotion { which, x, y, .. } if mouse(*which) && !touching => Input::MouseMove {
            x: px(*x),
            y: px(*y),
        },
        Event::MouseButtonDown {
            which,
            mouse_btn,
            x,
            y,
            ..
        } if mouse(*which) && !touching => mouse_button(*mouse_btn, true, px(*x), px(*y))?,
        Event::MouseButtonUp {
            which,
            mouse_btn,
            x,
            y,
            ..
        } if mouse(*which) => mouse_button(*mouse_btn, false, px(*x), px(*y))?,
        Event::MouseWheel {
            x, y, direction, ..
        } => {
            let (dx, dy) = if *direction == MouseWheelDirection::Flipped {
                (-x, -y)
            } else {
                (*x, *y)
            };
            Input::Wheel { dx, dy }
        }
        Event::KeyDown {
            keycode: Some(key),
            keymod,
            repeat,
            ..
        } => key_input(key.name(), true, *repeat, *keymod),
        Event::KeyUp {
            keycode: Some(key),
            keymod,
            ..
        } => key_input(key.name(), false, false, *keymod),
        Event::TextInput { text, .. } => Input::Text(text.clone()),
        Event::Window { win_event, .. } => window_input(win_event)?,
        Event::Quit { .. } => Input::Close,
        _ => return None,
    };
    Some(input)
}

fn mouse_button(button: MouseButton, down: bool, x: f64, y: f64) -> Option<Input> {
    let button = match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
        _ => return None,
    };
    Some(Input::MouseButton { button, down, x, y })
}

fn key_input(key: String, down: bool, repeat: bool, keymod: Mod) -> Input {
    Input::Key {
        key,
        down,
        repeat,
        modifiers: keymod.into(),
    }
}

fn window_input(event: &WindowEvent) -> Option<Input> {
    match event {
        WindowEvent::Resized(..)
        | WindowEvent::PixelSizeChanged(..)
        | WindowEvent::DisplayChanged(..) => Some(Input::Resized),
        WindowEvent::MouseLeave => Some(Input::PointerGone),
        WindowEvent::CloseRequested => Some(Input::Close),
        _ => None,
    }
}
