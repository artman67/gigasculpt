//! Key presses into commands, through the keymap: single keys, sequences (B, then S, then T),
//! slider keys (an argument written `"drag"`), keys whose command runs once when pressed and
//! once when let go (an argument written `"held"` is true, then false: the brush wheel opens
//! and then picks), and keys that turn something on and off (an argument written `"flip"`,
//! filled in from the control that sets the same argument: see [`Keys::flip`]). A marked
//! argument may sit inside a table, as a slider's may. A wheel notch the keymap binds runs its
//! command once a notch ([`Keys::notch`]).
//!
//! A slider key (`interface.on-canvas-adjust`), with sticky keys on (`ui.settings`, the
//! default), held past the activation time sets its argument as the pointer moves sideways
//! from there, and runs its command once when let go; a number typed while it is held sets
//! the argument exactly instead; a short tap shows that slider alone at the pointer;
//! Escape lets go without running anything. With sticky keys off, a held slider key never
//! follows the pointer: a number typed while it is held still sets the argument, and let go
//! with nothing typed it shows the slider.

use std::collections::BTreeMap;

use egui::{Event, Key};
use gs_engine::{BrushState, SessionState, WindowSettings};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::direction::{Control, Controls, put};
use crate::held::{SlotKeys, Sticky};
use crate::keymap::{Binding, Chord, Keymap, Press, Pressed};
use crate::record::with_args;
use crate::values::Value as Brush;

mod catch;

pub use catch::Catch;

/// What the keys read: the keymap, the session as the engine last reported it (a held key's
/// starting value, what a `"flip"` turns), the controls (which say what a `"flip"` turns),
/// pixels per point, the time in seconds by the window's clock, and the window's settings
/// (sticky keys).
pub struct Reads<'a> {
    pub keymap: &'a Keymap,
    pub state: &'a SessionState,
    pub controls: &'a Controls,
    pub ppp: f32,
    pub now: f64,
    pub settings: WindowSettings,
    /// The pointer is over the 3D view: a key whose argument is written `"off-view"` runs only
    /// when it is not, with that argument true (decision 0025, section 5: F6).
    pub over_view: bool,
}

/// The value an argument starts from when its key is held, how fast it moves per point the
/// pointer moves, where it stops, and what one of a typed number is worth. Other arguments
/// start at 0, move a hundredth a point and are typed as they are.
fn adjust(arg: &str, brush: &BrushState, ppp: f32) -> (f32, f32, [f32; 2], f32) {
    match Brush::from_arg(arg) {
        Some(v) => (v.of(brush), v.rate(ppp), v.range(), v.unit()),
        None => (0.0, 0.01, [f32::MIN, f32::MAX], 1.0),
    }
}

/// A slider key held to set an argument.
struct Hold {
    key: Key,
    command: String,
    args: Value,
    arg: String,
    /// When the key went down, by the window's clock.
    at: f64,
    /// Held past the activation time: the pointer sets the argument.
    active: bool,
    /// The number typed while it is held, as typed.
    typed: String,
    /// Where the pointer was when the hold became active, in points.
    from: Option<f32>,
    start: f32,
    rate: f32,
    range: [f32; 2],
    unit: f32,
    value: f32,
}

/// A short tap of the slider key `h` shows its slider alone at the pointer, in the quick menu's
/// window (decision 0025, amended 2026-10-12): the brush's three by name, any other by the
/// control that drags the same argument of the same command. One no control drags shows the
/// quick menu.
fn show_slider(h: &Hold, controls: &Controls) -> Step {
    let brush = (h.command == "brush.set")
        .then(|| Brush::from_arg(&h.arg))
        .flatten();
    let same = |c: &&Control| {
        c.command.as_deref() == Some(&*h.command) && c.dragged().as_deref() == Some(&*h.arg)
    };
    let mut all = controls.sets.iter().flat_map(|s| &s.controls);
    let label =
        (brush.map(|v| v.title().to_owned())).or_else(|| all.find(same).map(|c| c.label.clone()));
    match label {
        Some(slider) => with_args("ui.quick", json!({ "slider": slider })),
        None => with_args("ui.quick", json!({ "open": true })),
    }
}

/// How long, in seconds, a slider key is held before the pointer sets its value: never with
/// sticky keys off.
fn delay(settings: &WindowSettings) -> Option<f64> {
    settings
        .sticky_keys
        .then_some(f64::from(settings.sticky_delay))
}

/// A key whose command runs again with its `"held"` argument false when it is let go.
struct Latch {
    key: Key,
    command: String,
    args: Value,
    arg: String,
}

/// Keys pressed so far in a sequence, a key held to set an argument, keys held down for their
/// command, where the pointer is, and what keys turned off.
#[derive(Default)]
pub struct Keys {
    pending: Vec<Chord>,
    hold: Option<Hold>,
    latches: Vec<Latch>,
    /// The pointer's x, in points, as of the last event that moved it.
    x: Option<f32>,
    /// A slider's value a `"flip"` key set to 0, by the argument's dotted path: the key sets
    /// it back.
    before: BTreeMap<String, f64>,
    /// The brush slot the held modifier keys hold (`brush.slot`).
    pub slots: SlotKeys,
    /// The modifiers a tap or the pen's button turned on (sticky modifier keys).
    pub sticky: Sticky,
    /// The part of a bound wheel notch turned and not run yet (a touch ring turns in parts).
    turned: f32,
    /// A Ctrl+Alt press aimed at a control, waiting for the key to bind to it.
    pub catch: Catch,
}

impl Keys {
    /// This frame's events, in order, as commands to run.
    pub fn frame(&mut self, events: &[Event], reads: &Reads) -> Vec<Step> {
        if let Some(delay) = delay(&reads.settings)
            && let Some(h) = (self.hold.as_mut()).filter(|h| !h.active && reads.now - h.at >= delay)
        {
            h.active = true;
            h.from = self.x;
        }
        let mut out = Vec::new();
        for e in events {
            match e {
                Event::PointerMoved(p) => self.moved(p.x),
                Event::Key {
                    key,
                    pressed,
                    repeat: false,
                    modifiers,
                    ..
                } => out.extend(self.key(*key, *pressed, *modifiers, reads)),
                _ => {}
            }
        }
        out
    }

    /// The wheel turned `dy` notches (up is positive) with `modifiers`: `None` unless the keymap
    /// binds that notch alone, and else its command, with its arguments as written (a `"flip"`
    /// filled in by the session's state and the controls in `seen`, as a key's is), once for
    /// each whole notch turned (part of a notch waits for the rest).
    pub fn notch(
        &mut self,
        dy: f32,
        modifiers: egui::Modifiers,
        keymap: &Keymap,
        seen: (&SessionState, &Controls),
    ) -> Option<Vec<Step>> {
        let press = match dy {
            dy if dy > 0.0 => Press::WheelUp,
            dy if dy < 0.0 => Press::WheelDown,
            _ => return None,
        };
        let b = keymap.lookup(&Chord::pressed(press, modifiers))?;
        if self.turned * dy < 0.0 {
            self.turned = 0.0;
        }
        self.turned += dy;
        let args = serde_json::to_value(&b.args).unwrap_or_default();
        let mut steps = Vec::new();
        while self.turned.abs() >= 1.0 {
            self.turned -= self.turned.signum();
            steps.push(match marked(&b.args, "flip") {
                Some(arg) => self.flip(b, args.clone(), &arg, seen),
                None => with_args(&b.command, args.clone()),
            });
        }
        Some(steps)
    }

    /// How long, in seconds from `now`, until a slider key held now passes the activation time
    /// of `settings`, so the window draws then with no input: `None` while none is waiting.
    pub fn wake(&self, now: f64, settings: &WindowSettings) -> Option<f64> {
        let delay = delay(settings)?;
        let h = self.hold.as_ref().filter(|h| !h.active)?;
        Some((h.at + delay - now).max(0.0))
    }

    /// Whether a slider key is down, from its press until it is let go.
    pub fn holding(&self) -> bool {
        self.hold.is_some()
    }

    /// What a slider key held past the activation time, or with a number typed, is setting, for
    /// the cursor: `Size 80 px`, `Strength 75%`, or another argument's last name and value.
    pub fn adjusting(&self) -> Option<String> {
        let h = self
            .hold
            .as_ref()
            .filter(|h| h.active || !h.typed.is_empty())?;
        Some(match Brush::from_arg(&h.arg) {
            Some(v) => format!("{} {}", v.title(), v.text(h.value)),
            None => format!(
                "{} {:.2}",
                h.arg.rsplit('.').next().unwrap_or(&h.arg),
                h.value
            ),
        })
    }

    /// The pointer moved to `x`: the value of a key held past the activation time follows,
    /// unless a number was typed.
    fn moved(&mut self, x: f32) {
        self.x = Some(x);
        if let Some(h) = (self.hold.as_mut()).filter(|h| h.active && h.typed.is_empty()) {
            let from = *h.from.get_or_insert(x);
            h.value = (h.start + (x - from) * h.rate).clamp(h.range[0], h.range[1]);
        }
    }

    /// A key pressed while a slider key is held: a digit, point or minus adds to the number
    /// typed and Backspace takes one off, and the value follows; other keys wait.
    fn typed(&mut self, key: Key) {
        let Some(h) = &mut self.hold else {
            return;
        };
        let symbol = key.symbol_or_name();
        if key == Key::Backspace {
            h.typed.pop();
        } else if symbol.len() == 1 && "0123456789.-".contains(symbol) {
            h.typed.push_str(symbol);
        } else {
            return;
        }
        let number = h.typed.parse::<f32>().ok();
        h.value = number.map_or(h.start, |n| (n * h.unit).clamp(h.range[0], h.range[1]));
    }

    /// A key went down or up: the command it runs, if any.
    fn key(
        &mut self,
        key: Key,
        pressed: bool,
        modifiers: egui::Modifiers,
        reads: &Reads,
    ) -> Option<Step> {
        match (&self.hold, pressed) {
            (Some(h), false) if h.key == key => self.let_go(true, reads.controls),
            (Some(_), true) if key == Key::Escape => self.let_go(false, reads.controls),
            (_, false) => self.unlatch(key),
            (Some(_), true) => {
                self.typed(key);
                None
            }
            (None, true) => {
                let b = self.press(Chord::pressed(key, modifiers), reads.keymap)?;
                self.run_or_hold(key, &b, reads)
            }
        }
    }

    /// Adds a press to the sequence: the binding it completes, if any.
    fn press(&mut self, chord: Chord, keymap: &Keymap) -> Option<Binding> {
        self.pending.push(chord.clone());
        match keymap.sequence(&self.pending) {
            Pressed::Binding(b) => {
                self.pending.clear();
                Some(b.clone())
            }
            Pressed::Prefix => None,
            // A key that ends no sequence may start one of its own.
            Pressed::Nothing if self.pending.len() > 1 => {
                self.pending.clear();
                self.press(chord, keymap)
            }
            Pressed::Nothing => {
                self.pending.clear();
                None
            }
        }
    }

    /// A binding's command; when one of its arguments is `"flip"`, with it filled in; when one
    /// is `"drag"`, a hold of its key; when one is `"held"`, its command with that argument
    /// true, and a latch to run it false later.
    fn run_or_hold(&mut self, key: Key, b: &Binding, reads: &Reads) -> Option<Step> {
        let mut args = serde_json::to_value(&b.args).unwrap_or_default();
        let marked = |mark: &str| marked(&b.args, mark);
        // Off the 3D view only: over it the key runs nothing.
        if let Some(arg) = marked("off-view") {
            if reads.over_view {
                return None;
            }
            put(&mut args, &arg, json!(true));
        }
        if let Some(arg) = marked("flip") {
            return Some(self.flip(b, args, &arg, (reads.state, reads.controls)));
        }
        if let Some(arg) = marked("held") {
            put(&mut args, &arg, json!(true));
            let latch = Latch {
                key,
                command: b.command.clone(),
                args: args.clone(),
                arg,
            };
            self.latches.push(latch);
            return Some(with_args(&b.command, args));
        }
        let Some(arg) = marked("drag") else {
            return Some(with_args(&b.command, args));
        };
        let (start, rate, range, unit) = adjust(&arg, &reads.state.brush, reads.ppp);
        self.hold = Some(Hold {
            key,
            command: b.command.clone(),
            args,
            arg,
            at: reads.now,
            active: false,
            typed: String::new(),
            from: None,
            start,
            rate,
            range,
            unit,
            value: start,
        });
        None
    }

    /// `b`'s command with its argument at `path`, given as `"flip"`, filled in as the control
    /// that sets the same argument of the same command says. A switch (Library) fills it as it
    /// does when tapped (`Control::step_in`): false while it is lit, true while it is not. A
    /// slider's argument (Smoothing) is set to 0 while the slider's value is not 0, and else
    /// back to what it was when the key last set it to 0, or 1 the first time. With no such
    /// control the command runs as written, and says what is wrong.
    fn flip(
        &mut self,
        b: &Binding,
        mut args: Value,
        path: &str,
        (state, controls): (&SessionState, &Controls),
    ) -> Step {
        let state = state.values();
        let controls = controls.sets.iter().flat_map(|s| &s.controls);
        for c in controls.filter(|c| c.command.as_deref() == Some(b.command.as_str())) {
            if c.dragged().as_deref() == Some(path) {
                let shows = c.shows.as_deref().and_then(|v| state.get(v));
                let now = shows.and_then(Value::as_f64).unwrap_or_default();
                let value = if now == 0.0 {
                    self.before.get(path).copied().unwrap_or(1.0)
                } else {
                    self.before.insert(path.into(), now);
                    0.0
                };
                put(&mut args, path, json!(value));
                return with_args(&b.command, args);
            }
            if c.args == b.plain_args()
                && let Some(step) = c.step_in(&state)
            {
                return step;
            }
        }
        with_args(&b.command, args)
    }

    /// Ends a hold, when `run`: its command with the value reached or typed, or with nothing
    /// typed after a short tap (any hold with sticky keys off), the slider shown.
    fn let_go(&mut self, run: bool, controls: &Controls) -> Option<Step> {
        let mut h = self.hold.take().filter(|_| run)?;
        if !h.active && h.typed.is_empty() {
            return Some(show_slider(&h, controls));
        }
        put(&mut h.args, &h.arg, json!(h.value));
        Some(with_args(&h.command, h.args))
    }

    /// `key` was let go: the command of its latch, with the held argument false.
    fn unlatch(&mut self, key: Key) -> Option<Step> {
        let i = self.latches.iter().position(|l| l.key == key)?;
        let mut latch = self.latches.remove(i);
        put(&mut latch.args, &latch.arg, json!(false));
        Some(with_args(&latch.command, latch.args))
    }

    /// `brush` as the window shows it: with the value a held key is setting in place of its
    /// own.
    pub fn live(&self, brush: &BrushState) -> BrushState {
        let mut live = brush.clone();
        let held = self.hold.as_ref();
        if let Some((v, value)) = held.and_then(|h| Some((Brush::from_arg(&h.arg)?, h.value))) {
            v.put(&mut live, value);
        }
        live
    }
}

/// The dotted path to the argument given as `mark` in `args`, looking inside tables.
fn marked(args: &toml::Table, mark: &str) -> Option<String> {
    args.iter().find_map(|(k, v)| match v {
        toml::Value::String(s) if s == mark => Some(k.clone()),
        toml::Value::Table(t) => Some(format!("{k}.{}", marked(t, mark)?)),
        _ => None,
    })
}
