//! The held modifier keys, the one place that says what each does to a stroke: Shift strokes
//! with the Smooth slot's brush, Ctrl with the Mask slot's and Shift+Ctrl with the Select
//! slot's (`brush-settings.modifier-key-slots`, gs-engine `brush.slot`), and Alt works the
//! other way (subtract, unmask). The pen's stroke, the cursor and the status line all read it
//! here, so what the cursor shows is what the stroke does. Ctrl alone on empty canvas is a
//! gesture on the mask instead ([`background`]). With the gizmo tool on, a press works the
//! gizmo, never a stroke, so the keys say what they do to its drag. [`SlotKeys`] tells the
//! engine which slot the keys hold; [`Sticky`] adds the keys a tap or the pen's button turned
//! on to those held down.

use egui::{Color32, Modifiers};
use gs_core::brush::BrushPreset;
use gs_engine::{BarrelTwo, Slot, Slots};
use gs_protocol::Step;
use serde_json::json;

use crate::gizmo::InUse;
use crate::record::with_args;
use crate::theme::Colors;

/// What the held keys do to a stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    /// The keys, as the window writes them: `Shift`, `Ctrl+Alt`.
    pub keys: String,
    /// The slot the keys call.
    pub slot: Option<Slot>,
    /// The brush on that slot, which strokes instead of the session's.
    pub brush: Option<String>,
    /// Alt is held: the stroke works the other way from its brush's stored mode. The stroke
    /// sends this; the engine flips it again on a brush stored as Subtract.
    pub invert: bool,
    /// What the stroke ends up doing: Alt and the stored mode together, on a brush that
    /// sculpts. The cue shows this.
    pub subtracts: bool,
    /// Alt on a brush that paints color: it paints the Secondary color (decision 0039,
    /// section 7).
    pub secondary: bool,
    /// With the gizmo tool on, what the keys do to a drag instead, by the handle in use
    /// (`gizmo::InUse`): Alt moves the gizmo only, Ctrl copies, Shift snaps.
    pub drag: Option<String>,
}

impl Held {
    /// What `m` does, with the gizmo tool on (`gizmo`, its handle in use) or off, or `None`
    /// when no modifier is held. `preset` is the session's brush as a stroke uses it, `None`
    /// before the engine has said: stored as Subtract (`brush-settings.add-subtract`), Alt
    /// adds; painting color, Alt paints the Secondary color; painting only, Alt does not
    /// subtract. A slot's brush, from `slots`, keeps its own mode.
    pub fn of(
        m: Modifiers,
        gizmo: Option<InUse>,
        preset: Option<&BrushPreset>,
        slots: &Slots,
    ) -> Option<Held> {
        let names = [(m.ctrl, "Ctrl"), (m.shift, "Shift"), (m.alt, "Alt")];
        let keys: Vec<&str> = (names.iter()).filter(|n| n.0).map(|n| n.1).collect();
        if keys.is_empty() {
            return None;
        }
        if let Some(gizmo) = gizmo {
            return Some(Held {
                keys: keys.join("+"),
                slot: None,
                brush: None,
                invert: false,
                subtracts: false,
                secondary: false,
                drag: Some(gizmo.says(m)),
            });
        }
        let slot = slot(m);
        let own = slot.is_none();
        let stored = own && preset.is_some_and(BrushPreset::subtracts);
        let sculpts = !own || preset.is_none_or(BrushPreset::sculpts);
        Some(Held {
            keys: keys.join("+"),
            slot,
            brush: slot.map(|s| slots.brush(s).to_string()),
            invert: m.alt,
            subtracts: (m.alt ^ stored) && sculpts,
            secondary: m.alt && own && preset.is_some_and(BrushPreset::paints_color),
            drag: None,
        })
    }

    /// One line for the cursor and the status line: the slot's brush (`Shift: Smooth`,
    /// `Ctrl+Shift: Select Rect`), `Alt: Subtract` (`Alt: Add` on a brush stored as Subtract,
    /// `Alt: Secondary` on one that only paints color, `Alt: Subtract, Secondary` on one that
    /// sculpts and paints it), `Ctrl+Alt: Mask Pen, subtract`, with the gizmo tool on
    /// `Ctrl: Copy`.
    pub fn text(&self) -> String {
        if let Some(drag) = &self.drag {
            return format!("{}: {drag}", self.keys);
        }
        let keys = &self.keys;
        match (&self.brush, self.subtracts, self.secondary) {
            (Some(brush), true, _) => format!("{keys}: {brush}, subtract"),
            (Some(brush), false, _) => format!("{keys}: {brush}"),
            (None, true, true) => format!("{keys}: Subtract, Secondary"),
            (None, true, false) => format!("{keys}: Subtract"),
            (None, false, true) => format!("{keys}: Secondary"),
            (None, false, false) => format!("{keys}: Add"),
        }
    }

    /// The cursor's color in `colors` (the theme's), by slot whatever brush is on it: blue
    /// smooths, yellow masks, the text's color selects; red subtracts; a drag's keys, and Alt
    /// adding on a Subtract brush, keep the brush's own.
    pub fn color(&self, colors: &Colors) -> Color32 {
        if self.drag.is_some() {
            return colors.brush;
        }
        match (self.slot, self.subtracts) {
            (Some(Slot::Smooth), _) => colors.smooth,
            (Some(Slot::Mask), _) => colors.mask,
            (Some(Slot::Select), _) => colors.text,
            (None, true) => colors.subtract,
            (None, false) => colors.brush,
        }
    }
}

/// The slot `m` calls (`interface.gesture-map-model`): Shift+Ctrl the Select slot, Shift the
/// Smooth slot, Ctrl the Mask slot. Alt calls none.
pub fn slot(m: Modifiers) -> Option<Slot> {
    match (m.shift, m.ctrl) {
        (true, true) => Some(Slot::Select),
        (true, false) => Some(Slot::Smooth),
        (false, true) => Some(Slot::Mask),
        (false, false) => None,
    }
}

/// The keys whose slot in `slots` holds `brush`, as [`Held`] writes them (`Shift`, `Ctrl`,
/// `Ctrl+Shift`): held, they stroke with it. `None` while no slot holds it.
pub fn holding(brush: &str, slots: &Slots) -> Option<String> {
    let calls = [
        Modifiers::SHIFT,
        Modifiers::CTRL,
        Modifiers::SHIFT | Modifiers::CTRL,
    ];
    (calls.into_iter())
        .filter_map(|m| Held::of(m, None, None, slots))
        .find(|h| h.brush.as_deref() == Some(brush))
        .map(|h| h.keys)
}

/// The keys a slot is called with, Shift one bit and Ctrl the other: a slot's keys are within
/// another's when no key of the first is missing from the second.
fn bits(slot: Option<Slot>) -> u8 {
    match slot {
        None => 0,
        Some(Slot::Smooth) => 1,
        Some(Slot::Mask) => 2,
        Some(Slot::Select) => 3,
    }
}

fn within(a: Option<Slot>, b: Option<Slot>) -> bool {
    bits(a) & !bits(b) == 0
}

/// How long, in seconds, a slot's keys may be let go for pressing them again to count as a tap
/// (and Alt, for a curve stroke's double tap; decision 0055).
pub(crate) const TAP: f64 = 0.3;

/// The slot the window's keys hold, as last sent to the engine, and the slot last let go with
/// when, for a tap.
#[derive(Default)]
pub struct SlotKeys {
    sent: Option<Slot>,
    let_go: Option<(Slot, f64)>,
}

impl SlotKeys {
    /// The `brush.slot` call for the keys held now (`m`, at `now` in seconds by the window's
    /// clock), when the slot they call changed: hold the new one, or let go. The Mask or Select
    /// slot's keys let go and pressed again within [`TAP`], with no other key pressed between,
    /// step the slot to the next brush of its family (the catalog's proposal, unverified),
    /// unless sticky modifier keys are on (`sticky`): then a tap only turns keys on or off.
    pub fn frame(&mut self, m: Modifiers, now: f64, sticky: bool) -> Option<Step> {
        let next = slot(m);
        let fresh = self.let_go.filter(|(_, t)| !sticky && now - t <= TAP);
        if next == self.sent {
            return None;
        }
        let was = std::mem::replace(&mut self.sent, next);
        let remembered = |s: Option<Slot>| fresh.is_some_and(|(f, _)| within(s, Some(f)));
        if within(next, was) {
            if !remembered(was) {
                self.let_go = was.map(|s| (s, now));
            }
            let args = match next {
                Some(s) => json!({ "slot": s, "hold": true }),
                None => json!({ "release": true }),
            };
            return Some(with_args("brush.slot", args));
        }
        let step = fresh.is_some_and(|(f, _)| next == Some(f) && f != Slot::Smooth);
        if step || !remembered(next) {
            self.let_go = None;
        }
        let mut args = json!({ "slot": next, "hold": true });
        if step {
            args["step"] = json!(true);
        }
        Some(with_args("brush.slot", args))
    }

    /// `m` as a stroke under way reads it: the keys of the slot it pressed with (the slot last
    /// sent, which a stroke keeps until the pen lifts), and Alt as it is now. Shift pressed after
    /// the press holds a line instead (`stroke.freehand`), so it calls no slot here.
    pub fn stroking(&self, m: Modifiers) -> Modifiers {
        let b = bits(self.sent);
        let keys = Modifiers {
            shift: b & 1 != 0,
            ctrl: b & 2 != 0,
            ..m
        };
        commanding(keys, MAC)
    }
}

/// This build runs on macOS, where egui's command key is Cmd.
const MAC: bool = cfg!(target_os = "macos");

/// `m` with egui's command key as the platform has it, `mac` saying whether that is macOS: Cmd
/// (`mac_cmd`) there, Ctrl elsewhere, as gs-ui `glue.rs` `modifiers` reads the platform's keys.
pub fn commanding(m: Modifiers, mac: bool) -> Modifiers {
    Modifiers {
        command: if mac { m.mac_cmd } else { m.ctrl },
        ..m
    }
}

/// How long, in seconds, a modifier key may be held for letting go of it to be a tap, which
/// sticky modifier keys turn on or off: longer is a hold, as without them.
const HOLD: f64 = 0.3;

/// The modifiers in effect beyond the keys held down (`interface.modifier-keys`): those a tap
/// turned on with sticky modifier keys (`ui.settings` `sticky_modifiers`), the one barrel
/// button 2 stands for while it is down (`ui.navigation` `barrel_two`), and those the eraser
/// end stands for from its first touch until it leaves range (`ui.navigation` `eraser`; the
/// tablet names the end only when it touches). The window feeds every
/// modifier it reads through here, so a key on by a tap, by the pen's button or by its eraser
/// does all a key held down does: the stroke, the cursor and the status line.
#[derive(Default)]
pub struct Sticky {
    /// Sticky modifier keys are on, as last read.
    on: bool,
    /// The keys held down, as last read.
    keys: Modifiers,
    /// The keys a tap turned on: Shift, Ctrl, Alt.
    latched: [bool; 3],
    /// When each key held down went down, and whether anything else went down since: a
    /// shortcut, a click or a stroke made with it held, which is not a tap.
    down: [Option<(f64, bool)>; 3],
    /// The modifiers barrel button 2 stands for while it is down.
    pen: [bool; 3],
    /// The modifiers the pen's eraser end stands for while the pen reads as the eraser.
    eraser: [bool; 3],
}

impl Sticky {
    /// Reads the keys held down, `keys`, at `now` in seconds by the window's clock. With `on`
    /// (sticky modifier keys), a key let go within [`HOLD`] of going down, nothing else having
    /// gone down meanwhile, turns on, or off if a tap had turned it on; `other` says something
    /// else went down now. Off, nothing stays on. Returns the modifiers in effect.
    pub fn keys(&mut self, keys: Modifiers, now: f64, on: bool, other: bool) -> Modifiers {
        let held = [keys.shift, keys.ctrl, keys.alt];
        let each = (self.down.iter_mut()).zip(&mut self.latched).zip(held);
        for ((down, latched), held) in each {
            *down = match (*down, held) {
                (None, true) => Some((now, other)),
                (Some((at, spoiled)), true) => Some((at, spoiled || other)),
                (Some((at, spoiled)), false) => {
                    *latched ^= on && !spoiled && now - at <= HOLD;
                    None
                }
                (None, false) => None,
            };
            *latched &= on;
        }
        (self.on, self.keys) = (on, keys);
        self.held()
    }

    /// Barrel button 2 is down (`down`) standing for `key`, or not.
    pub fn pen(&mut self, key: BarrelTwo, down: bool) {
        self.pen = match key {
            BarrelTwo::Shift => [down, false, false],
            BarrelTwo::Ctrl => [false, down, false],
            BarrelTwo::Alt => [false, false, down],
            BarrelTwo::Pan => [false; 3],
        };
    }

    /// The eraser end stands for `keys` (Shift, Ctrl, Alt) while the pen reads as it, or for
    /// none.
    pub fn eraser(&mut self, keys: [bool; 3]) {
        self.eraser = keys;
    }

    /// The pen touched or a button went down: the keys held down now are not tapped.
    pub fn spoil(&mut self) {
        for d in self.down.iter_mut().flatten() {
            d.1 = true;
        }
    }

    /// The modifiers in effect: held down, turned on by a tap, the pen button's or the eraser
    /// end's.
    pub fn held(&self) -> Modifiers {
        let on = |i: usize| self.latched[i] || self.pen[i] || self.eraser[i];
        let k = self.keys;
        let keys = Modifiers {
            shift: k.shift || on(0),
            ctrl: k.ctrl || on(1),
            alt: k.alt || on(2),
            ..k
        };
        commanding(keys, MAC)
    }
}

/// Whether a press with `m` held is a background gesture when it falls on empty canvas (decision
/// 0017 as amended, item 10): Ctrl alone. Ctrl with Alt or Shift keeps its own row.
pub fn background(m: Modifiers) -> bool {
    m.ctrl && !m.alt && !m.shift
}
